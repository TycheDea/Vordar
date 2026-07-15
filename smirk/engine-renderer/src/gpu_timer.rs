// GPU frame timing for the dev overlay: two timestamps bracket the frame
// (shadow-pass begin → tonemap-pass end). `None` when the adapter lacks
// TIMESTAMP_QUERY. Sampled sparsely and read with a blocking map —
// dev-overlay-only cost.

use wgpu::{Device, Queue};

pub(crate) struct GpuTimer {
    pub(crate) query_set: wgpu::QuerySet,
    resolve:   wgpu::Buffer,
    staging:   wgpu::Buffer,
    period_ns: f32,
}

impl GpuTimer {
    pub(crate) fn new(device: &Device, queue: &Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame Timestamps"),
            ty:    wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Timestamp Resolve"),
            size:               16,
            usage:              wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Timestamp Staging"),
            size:               16,
            usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Some(Self { query_set, resolve, staging, period_ns: queue.get_timestamp_period() })
    }

    pub(crate) fn begin_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set:                    &self.query_set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index:       None,
        }
    }

    pub(crate) fn end_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set:                    &self.query_set,
            beginning_of_pass_write_index: None,
            end_of_pass_write_index:       Some(1),
        }
    }

    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.query_set, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, 16);
    }

    /// Blocking read of the last resolved pair → frame GPU milliseconds.
    pub(crate) fn read_blocking(&self, device: &Device) -> Option<f32> {
        let slice = self.staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .ok()?;
        let data = slice.get_mapped_range();
        let stamps: &[u64] = bytemuck::cast_slice(&data);
        let ms = (stamps[1].saturating_sub(stamps[0])) as f32 * self.period_ns * 1e-6;
        drop(data);
        self.staging.unmap();
        Some(ms)
    }
}
