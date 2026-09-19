//! Exact fixed-point unit rates. Stored monetary amounts remain microcredits.
pub const PRICE_FRACTIONAL_DIGITS: usize = 12;
pub const PICO_PER_CREDIT: i128 = 1_000_000_000_000;
pub const PICO_PER_MICRO: i128 = 1_000_000;
pub const MAX_PRICE_PICO: i64 = 1_000_000_000_000_000_000;

pub fn rate_pico(precise: Option<i64>, legacy_micros: i64) -> i128 {
    precise
        .map(|value| i128::from(value.max(0)))
        .unwrap_or_else(|| i128::from(legacy_micros.max(0)) * PICO_PER_MICRO)
}

pub fn cost_pico(rate: i128, quantity: i64) -> i128 {
    rate.max(0).saturating_mul(i128::from(quantity.max(0)))
}

/// Gross/funding display amounts truncate only after multiplying the full rate.
pub fn cost_micros(rate: i128, quantity: i64) -> i64 {
    (cost_pico(rate, quantity) / PICO_PER_MICRO).min(i128::from(i64::MAX)) as i64
}

pub fn whole_credits(pico: i128) -> i64 {
    ((pico.max(0).saturating_add(PICO_PER_CREDIT - 1)) / PICO_PER_CREDIT).min(i128::from(i64::MAX))
        as i64
}

pub fn decimal_to_pico(raw: &str) -> Option<i64> {
    let value = raw.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > PRICE_FRACTIONAL_DIGITS
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let fraction: i64 = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i64>()
            .ok()?
            .checked_mul(10_i64.pow((PRICE_FRACTIONAL_DIGITS - fraction.len()) as u32))?
    };
    whole
        .checked_mul(PICO_PER_CREDIT as i64)?
        .checked_add(fraction)
}

pub fn format_pico(pico: i64) -> String {
    let whole = i128::from(pico) / PICO_PER_CREDIT;
    let fraction = i128::from(pico) % PICO_PER_CREDIT;
    if fraction == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fraction:012}")
        .trim_end_matches('0')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_precise_rates_and_legacy_rounding() {
        let pico = decimal_to_pico("0.000000250001").unwrap();
        assert_eq!(format_pico(pico), "0.000000250001");
        assert_eq!(cost_micros(i128::from(pico), 4_000_000), 1_000_004);
        assert_eq!(whole_credits(cost_pico(i128::from(pico), 1)), 1);
        assert_eq!(cost_micros(rate_pico(None, 125_000), 8), 1_000_000);
        assert_eq!(decimal_to_pico("1000000"), Some(MAX_PRICE_PICO));
        assert!(decimal_to_pico("0.0000000000001").is_none());
        assert_eq!(cost_micros(rate_pico(None, i64::MAX), i64::MAX), i64::MAX);
    }
}
