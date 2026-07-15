/// Deterministic LCG shared by the benchmarks' scenario builders and
/// server/vordar-server/tests/soak.rs's Wander.
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493))
    }

    /// Advance the state and return it.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }

    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 33) as u32) as f32 / (u32::MAX as f32 + 1.0)
    }
}
