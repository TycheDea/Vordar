/// Percentile `p` (0.0-1.0) of `values`, sorted ascending in place.
pub fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    let idx = ((values.len() as f64 * p) as usize).min(values.len() - 1);
    values[idx]
}
