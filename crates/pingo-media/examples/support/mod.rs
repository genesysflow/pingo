use anyhow::{Result, bail};
pub fn value(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    args.windows(2).find(|v| v[0] == name).map(|v| v[1].clone())
}
pub fn number(name: &str, default: f64) -> Result<f64> {
    let value = value(name)
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(default);
    if !value.is_finite() || value < 0.0 {
        bail!("{name} must be a nonnegative number");
    }
    Ok(value)
}
pub fn flag(name: &str) -> bool {
    std::env::args().any(|v| v == name)
}
pub fn random(seed: &mut u64) -> f64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed as u32) as f64 / u32::MAX as f64
}
