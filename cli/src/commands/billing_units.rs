//! Billing units and exact unit-price validation shared by CLI surfaces.
pub const METRICS: [&str; 8] = [
    "tokens",
    "requests",
    "bytes",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "images",
];
pub const PRICE_FRACTIONAL_DIGITS: usize = 12;

pub fn label(metric: &str, singular: bool) -> &str {
    match (metric, singular) {
        ("tokens", true) => "token",
        ("requests", true) => "request",
        ("bytes", true) => "byte",
        ("input_tokens", false) => "input tokens",
        ("input_tokens", true) => "input token",
        ("output_tokens", false) => "output tokens",
        ("output_tokens", true) => "output token",
        ("cache_read_tokens", false) => "cache-read tokens",
        ("cache_read_tokens", true) => "cache-read token",
        ("cache_write_tokens", false) => "cache-write tokens",
        ("cache_write_tokens", true) => "cache-write token",
        ("images", true) => "image",
        _ => metric,
    }
}

pub fn price(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || (raw.contains('.') && fraction.is_empty())
        || fraction.len() > PRICE_FRACTIONAL_DIGITS
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("Use a non-negative price with at most 12 fractional digits".into());
    }
    let amount = whole
        .parse::<u128>()
        .ok()
        .and_then(|whole| whole.checked_mul(1_000_000_000_000))
        .and_then(|whole| {
            fraction
                .parse::<u128>()
                .unwrap_or(0)
                .checked_mul(10_u128.pow((12 - fraction.len()) as u32))
                .and_then(|fraction| whole.checked_add(fraction))
        });
    if amount.is_none_or(|amount| amount > 1_000_000_000_000_000_000) {
        return Err("Price must not exceed 1,000,000 credits per unit".into());
    }
    Ok(raw.to_string())
}

pub fn component(raw: &str) -> Result<String, String> {
    let (metric, value) = raw.split_once('=').ok_or("Use <metric>=<price>")?;
    if !METRICS.contains(&metric) {
        return Err(format!("Unknown billing metric: {metric}"));
    }
    Ok(format!("{metric}={}", price(value)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prices_are_exact_and_components_are_validated() {
        assert_eq!(
            component("cache_read_tokens=0.000000250001").unwrap(),
            "cache_read_tokens=0.000000250001"
        );
        for bad in [
            "-1",
            "0.0000000000001",
            "1e-12",
            "1000000.000000000001",
            "1.",
        ] {
            assert!(price(bad).is_err(), "{bad}");
        }
        assert_eq!(label("future_unit", false), "future_unit");
    }
}
