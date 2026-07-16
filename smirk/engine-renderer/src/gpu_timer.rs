// GPU frame timing for the dev overlay: 6 pairs of timestamps bracket the
// recorded passes (shadow, main, particles, bloom, tonemap, egui) — bloom
// and tonemap sum into one overlay line. `None` when the adapter lacks
// TIMESTAMP_QUERY. Sampled sparsely and read with a blocking map —
// dev-overlay-only cost.

use wgpu::{Device, Queue};

/// Index into the 6-pair (12-timestamp) query set — one bracket per
/// recorded pass.
#[derive(Clone, Copy)]
pub(crate) enum GpuPass {
    Shadow    = 0,
    Main      = 1,
    Particles = 2,
    Bloom     = 3,
    Tonemap   = 4,
    Egui      = 5,
}

const PASS_COUNT:  u32 = 6;
const QUERY_COUNT: u32 = PASS_COUNT * 2;
const BUFFER_SIZE: u64 = QUERY_COUNT as u64 * 8; // u64 timestamps

/// Per-pass GPU milliseconds for one sampled frame; bloom and tonemap are
/// timed as separate brackets but published as one overlay line.
#[derive(Clone, Copy)]
pub(crate) struct GpuPassTimings {
    pub(crate) shadow:        f32,
    pub(crate) main:          f32,
    pub(crate) particles:     f32,
    pub(crate) bloom_tonemap: f32,
    pub(crate) egui:          f32,
}

fn pass_ms(stamps: &[u64], pass: GpuPass, period_ns: f32) -> f32 {
    let i = pass as usize * 2;
    stamps[i + 1].saturating_sub(stamps[i]) as f32 * period_ns * 1e-6
}

pub(crate) fn compute_pass_times(stamps: &[u64], period_ns: f32) -> GpuPassTimings {
    GpuPassTimings {
        shadow:        pass_ms(stamps, GpuPass::Shadow, period_ns),
        main:          pass_ms(stamps, GpuPass::Main, period_ns),
        particles:     pass_ms(stamps, GpuPass::Particles, period_ns),
        bloom_tonemap: pass_ms(stamps, GpuPass::Bloom, period_ns)
                     + pass_ms(stamps, GpuPass::Tonemap, period_ns),
        egui:          pass_ms(stamps, GpuPass::Egui, period_ns),
    }
}

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
            count: QUERY_COUNT,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Timestamp Resolve"),
            size:               BUFFER_SIZE,
            usage:              wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Timestamp Staging"),
            size:               BUFFER_SIZE,
            usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Some(Self { query_set, resolve, staging, period_ns: queue.get_timestamp_period() })
    }

    /// Begin+end writes for a pass recorded as a single render pass.
    pub(crate) fn pass_writes(&self, pass: GpuPass) -> wgpu::RenderPassTimestampWrites<'_> {
        let i = pass as u32 * 2;
        wgpu::RenderPassTimestampWrites {
            query_set:                    &self.query_set,
            beginning_of_pass_write_index: Some(i),
            end_of_pass_write_index:       Some(i + 1),
        }
    }

    /// Begin-only write for a pass split across multiple render passes (the
    /// bloom chain's variable stage count) — pair with `end_writes` on the
    /// pass's final stage.
    pub(crate) fn begin_writes(&self, pass: GpuPass) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set:                    &self.query_set,
            beginning_of_pass_write_index: Some(pass as u32 * 2),
            end_of_pass_write_index:       None,
        }
    }

    pub(crate) fn end_writes(&self, pass: GpuPass) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set:                    &self.query_set,
            beginning_of_pass_write_index: None,
            end_of_pass_write_index:       Some(pass as u32 * 2 + 1),
        }
    }

    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.query_set, 0..QUERY_COUNT, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, BUFFER_SIZE);
    }

    /// Blocking read of the last resolved set → per-pass GPU milliseconds.
    pub(crate) fn read_blocking(&self, device: &Device) -> Option<GpuPassTimings> {
        let slice = self.staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .ok()?;
        let data = slice.get_mapped_range();
        let stamps: &[u64] = bytemuck::cast_slice(&data);
        let timings = compute_pass_times(stamps, self.period_ns);
        drop(data);
        self.staging.unmap();
        Some(timings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_pass_times_isolates_each_bracket_and_sums_bloom_tonemap() {
        // 6 pairs of (begin, end) ticks; period_ns=1_000_000.0 makes 1 tick
        // == 1 ms so the deltas below read directly as milliseconds.
        let stamps: [u64; 12] = [
            0,   100, // shadow: 100
            100, 250, // main: 150
            250, 300, // particles: 50
            300, 350, // bloom: 50
            350, 420, // tonemap: 70
            420, 450, // egui: 30
        ];
        let t = compute_pass_times(&stamps, 1_000_000.0);

        assert!((t.shadow - 100.0).abs() < 1e-3, "shadow: {}", t.shadow);
        assert!((t.main - 150.0).abs() < 1e-3, "main: {}", t.main);
        assert!((t.particles - 50.0).abs() < 1e-3, "particles: {}", t.particles);
        assert!((t.bloom_tonemap - 120.0).abs() < 1e-3, "bloom_tonemap: {}", t.bloom_tonemap);
        assert!((t.egui - 30.0).abs() < 1e-3, "egui: {}", t.egui);
    }

    #[test]
    fn compute_pass_times_clamps_out_of_order_timestamps_to_zero() {
        // saturating_sub guards a begin > end pair (e.g. a stale/unwritten
        // bracket) instead of wrapping to a huge duration.
        let mut stamps = [0u64; 12];
        stamps[2] = 500; // main begin
        stamps[3] = 100; // main end < begin
        let t = compute_pass_times(&stamps, 1.0);
        assert_eq!(t.main, 0.0);
    }
}
