// Time — frame-level timing resource.
//
// Per-phase accumulators and fixed_dt live inside the Scheduler.
// This resource exposes the current frame's wall-clock delta for systems
// that need it (typically render systems computing interpolation or effects).

#[derive(Debug)]
pub struct Time {
    pub frame_dt: f32,
    /// Local→server clock offset in microseconds, maintained by the network
    /// client from NTP-style sync samples (DESIGN.md §3). Zero on the server
    /// and in offline builds, where local time IS server time.
    pub server_offset_micros: i64,
}

impl Time {
    pub fn new() -> Self {
        Self { frame_dt: 0.0, server_offset_micros: 0 }
    }
}