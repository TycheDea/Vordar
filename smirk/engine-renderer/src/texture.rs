// GPU texture creation and the shared `ColorTexture` bind-group unit: the
// compressed-BC7 loader (`load_dds`), plain/mipped RGBA8 uploads, and the
// procedural checker/white textures used when no asset is set. Every texture
// shares one sampler configuration (`make_sampler`).

use wgpu::{
    AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindingResource, Device, Extent3d, FilterMode, MipmapFilterMode, Queue, Sampler,
    SamplerDescriptor, Texture, TextureAspect, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};

// ── Color textures ─────────────────────────────────────────────────────────────

pub struct ColorTexture {
    pub texture: Texture,
    pub view:    TextureView,
    pub sampler: Sampler,
    /// Resident GPU bytes across the whole mip chain (`gpu_texture_bytes`) —
    /// feeds the dev overlay's texture-memory meter.
    pub bytes:   u64,
}

/// Resident GPU bytes for a texture's full mip chain: for each level, the
/// mip's texel dimensions rounded up to whole compressed/uncompressed blocks,
/// times the format's per-block byte size.
pub fn gpu_texture_bytes(format: TextureFormat, width: u32, height: u32, mip_count: u32) -> u64 {
    let (block_w, block_h) = format.block_dimensions();
    let block_bytes = format.block_copy_size(None).unwrap() as u64;
    (0..mip_count)
        .map(|level| {
            let w = (width >> level).max(1);
            let h = (height >> level).max(1);
            let blocks_x = w.div_ceil(block_w) as u64;
            let blocks_y = h.div_ceil(block_h) as u64;
            blocks_x * blocks_y * block_bytes
        })
        .sum()
}

fn make_sampler(device: &Device) -> Sampler {
    device.create_sampler(&SamplerDescriptor {
        label:           Some("Color Sampler"),
        address_mode_u:  AddressMode::Repeat,
        address_mode_v:  AddressMode::Repeat,
        address_mode_w:  AddressMode::Repeat,
        mag_filter:      FilterMode::Linear,
        min_filter:      FilterMode::Linear,
        mipmap_filter:   MipmapFilterMode::Linear,
        // Anisotropic filtering on every surface sampler. Requires all
        // three filters Linear (they are).
        anisotropy_clamp: 8,
        ..Default::default()
    })
}

/// Load a BC7-encoded DDS file directly as a GPU texture, uploading every
/// baked mip level the file carries. `srgb` picks Bc7RgbaUnormSrgb (for
/// sRGB-encoded images, e.g. albedo) vs Bc7RgbaUnorm (for linear data).
/// Returns Err if the file cannot be read or parsed.
pub fn load_dds(device: &Device, queue: &Queue, path: &str, srgb: bool) -> Result<ColorTexture, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
    let mut cursor = std::io::Cursor::new(bytes);
    let dds = ddsfile::Dds::read(&mut cursor).map_err(|e| format!("DDS parse error: {e}"))?;

    let width  = dds.header.width;
    let height = dds.header.height;
    let mips   = dds.get_num_mipmap_levels().max(1);

    let texture = device.create_texture(&TextureDescriptor {
        label:           Some("Color Texture BC7"),
        size:            Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count:    1,
        dimension:       TextureDimension::D2,
        format:          dds_format(srgb),
        usage:           TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats:    &[],
    });

    // BC7: 4×4 blocks, 16 bytes each; levels are stored contiguously.
    let mut offset = 0usize;
    for level in 0..mips {
        let w = (width >> level).max(1);
        let h = (height >> level).max(1);
        let blocks_x      = w.div_ceil(4);
        let blocks_y      = h.div_ceil(4);
        let bytes_per_row = blocks_x * 16;
        let mip_size      = (blocks_x * blocks_y * 16) as usize;
        if offset + mip_size > dds.data.len() {
            log::warn!("{path}: DDS data truncated at mip {level} — uploaded {level} of {mips} levels");
            break;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture:   &texture,
                mip_level: level,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    TextureAspect::All,
            },
            &dds.data[offset..offset + mip_size],
            wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(bytes_per_row),
                rows_per_image: Some(blocks_y),
            },
            Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        offset += mip_size;
    }

    let view    = texture.create_view(&TextureViewDescriptor::default());
    let sampler = make_sampler(device);
    let tex_bytes = gpu_texture_bytes(dds_format(srgb), width, height, mips);
    Ok(ColorTexture { texture, view, sampler, bytes: tex_bytes })
}

/// Upload RGBA8 pixels and build a full mip chain via the blit generator.
/// `srgb` as in `create_rgba_texture`. 1×1 textures skip mipgen.
pub(crate) fn create_rgba_texture_mipped(
    device: &Device,
    queue:  &Queue,
    mipgen: &crate::mipgen::MipGenerator,
    width:  u32,
    height: u32,
    pixels: &[u8],
    srgb:   bool,
) -> ColorTexture {
    let format = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    let mips = crate::mipgen::mip_level_count(width, height);
    let texture = device.create_texture(&TextureDescriptor {
        label:           Some("RGBA8 Mipped Texture"),
        size:            Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count:    1,
        dimension:       TextureDimension::D2,
        format,
        usage:           TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_DST
            | TextureUsages::COPY_SRC
            | TextureUsages::RENDER_ATTACHMENT,
        view_formats:    &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture:   &texture,
            mip_level: 0,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset:         0,
            bytes_per_row:  Some(width * 4),
            rows_per_image: Some(height),
        },
        Extent3d { width, height, depth_or_array_layers: 1 },
    );
    mipgen.generate(device, queue, &texture);

    let view    = texture.create_view(&TextureViewDescriptor::default());
    let sampler = make_sampler(device);
    let bytes   = gpu_texture_bytes(format, width, height, mips);
    ColorTexture { texture, view, sampler, bytes }
}

/// Upload tightly-packed RGBA8 pixels as a GPU texture. `srgb` picks
/// Rgba8UnormSrgb (for sRGB-encoded images, e.g. glTF base color) vs plain
/// Rgba8Unorm (for linear data).
pub(crate) fn create_rgba_texture(
    device: &Device,
    queue:  &Queue,
    width:  u32,
    height: u32,
    pixels: &[u8],
    srgb:   bool,
) -> ColorTexture {
    let format = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    let texture = device.create_texture(&TextureDescriptor {
        label:           Some("RGBA8 Texture"),
        size:            Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       TextureDimension::D2,
        format,
        usage:           TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats:    &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture:   &texture,
            mip_level: 0,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset:         0,
            bytes_per_row:  Some(width * 4),
            rows_per_image: Some(height),
        },
        Extent3d { width, height, depth_or_array_layers: 1 },
    );

    let view    = texture.create_view(&TextureViewDescriptor::default());
    let sampler = make_sampler(device);
    let bytes   = gpu_texture_bytes(format, width, height, 1);
    ColorTexture { texture, view, sampler, bytes }
}

/// Procedural RGBA8 checkerboard texture (size×size, tile_size pixels per square).
/// Useful for testing the texture pipeline without any asset files.
pub fn create_checker_texture(
    device:    &Device,
    queue:     &Queue,
    size:      u32,
    tile_size: u32,
    color_a:   [u8; 4],
    color_b:   [u8; 4],
) -> ColorTexture {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let tile_x = x / tile_size;
            let tile_y = y / tile_size;
            let color  = if (tile_x + tile_y) % 2 == 0 { color_a } else { color_b };
            let i      = ((y * size + x) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&color);
        }
    }

    let texture = device.create_texture(&TextureDescriptor {
        label:           Some("Checker Texture"),
        size:            Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       TextureDimension::D2,
        format:          TextureFormat::Rgba8Unorm,
        usage:           TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats:    &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture:   &texture,
            mip_level: 0,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset:         0,
            bytes_per_row:  Some(size * 4),
            rows_per_image: Some(size),
        },
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
    );

    let view    = texture.create_view(&TextureViewDescriptor::default());
    let sampler = make_sampler(device);
    let bytes   = gpu_texture_bytes(TextureFormat::Rgba8Unorm, size, size, 1);
    ColorTexture { texture, view, sampler, bytes }
}

/// 1×1 white RGBA8 texture. Used as default when no texture is set —
/// instance color multiplies by [1,1,1] so existing shapes are unaffected.
pub fn create_default_white(device: &Device, queue: &Queue) -> ColorTexture {
    let texture = device.create_texture(&TextureDescriptor {
        label:           Some("Default White Texture"),
        size:            Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       TextureDimension::D2,
        format:          TextureFormat::Rgba8Unorm,
        usage:           TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats:    &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture:   &texture,
            mip_level: 0,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    TextureAspect::All,
        },
        &[255u8, 255, 255, 255],
        wgpu::TexelCopyBufferLayout {
            offset:         0,
            bytes_per_row:  Some(4),
            rows_per_image: Some(1),
        },
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );

    let view    = texture.create_view(&TextureViewDescriptor::default());
    let sampler = make_sampler(device);
    let bytes   = gpu_texture_bytes(TextureFormat::Rgba8Unorm, 1, 1, 1);
    ColorTexture { texture, view, sampler, bytes }
}

/// Create a bind group for a `ColorTexture` against the texture bind group layout.
pub(crate) fn create_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    tex:    &ColorTexture,
) -> BindGroup {
    device.create_bind_group(&BindGroupDescriptor {
        label:   Some("Texture Bind Group"),
        layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&tex.view) },
            BindGroupEntry { binding: 1, resource: BindingResource::Sampler(&tex.sampler)  },
        ],
    })
}

/// Pick BC7 format based on color-space intent. sRGB-encoded data (color/albedo)
/// uses Bc7RgbaUnormSrgb; linear data uses Bc7RgbaUnorm.
pub fn dds_format(srgb: bool) -> TextureFormat {
    if srgb { TextureFormat::Bc7RgbaUnormSrgb } else { TextureFormat::Bc7RgbaUnorm }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dds_format_srgb_true_returns_srgb() {
        assert_eq!(dds_format(true), TextureFormat::Bc7RgbaUnormSrgb);
    }

    #[test]
    fn dds_format_srgb_false_returns_unorm() {
        assert_eq!(dds_format(false), TextureFormat::Bc7RgbaUnorm);
    }

    #[test]
    fn gpu_texture_bytes_rgba8_uncompressed_sums_mip_chain() {
        // 8x8 + 4x4 + 2x2 + 1x1 = 85 texels x 4 bytes/texel (1x1 block, no compression).
        let bytes = gpu_texture_bytes(TextureFormat::Rgba8Unorm, 8, 8, 4);
        assert_eq!(bytes, (64 + 16 + 4 + 1) * 4);
    }

    #[test]
    fn gpu_texture_bytes_bc7_sums_mip_chain_in_4x4_blocks() {
        // 8x8 -> 2x2 blocks, 4x4 -> 1x1, 2x2 -> 1x1, 1x1 -> 1x1: 4+1+1+1 = 7 blocks x 16 B/block.
        let bytes = gpu_texture_bytes(TextureFormat::Bc7RgbaUnormSrgb, 8, 8, 4);
        assert_eq!(bytes, (4 + 1 + 1 + 1) * 16);
    }

    #[test]
    fn gpu_texture_bytes_bc5_same_block_math_as_bc7() {
        // Same 4x4 block grid as BC7, and BC5 is also a 128-bit (16 B) block
        // format, so the mip chain totals match BC7's exactly.
        let bytes = gpu_texture_bytes(TextureFormat::Bc5RgUnorm, 8, 8, 4);
        assert_eq!(bytes, (4 + 1 + 1 + 1) * 16);
    }
}
