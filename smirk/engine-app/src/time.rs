// Time — frame-level timing resource.
//
// Per-phase accumulators and fixed_dt live inside the Scheduler.
// This resource exposes the current frame's wall-clock delta for systems
// that need it (typically render systems computing interpolation or effects).

#[derive(Debug)]
pub struct Time {
    pub frame_dt: f32,
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

impl Time {
    pub fn new() -> Self {
        Self { frame_dt: 0.0 }
    }
}