//! Exact credits at 10^-12 resolution. JSON uses strings; model fields use
//! the explicit BSON adapters below (BSON's document serializer is human-readable).
use std::{fmt, str::FromStr};

use bson::{Bson, Decimal128};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

pub const SCALE: i128 = 1_000_000_000_000;
// Decimal128 has 34 significant digits. Restrict *all* values to this range
// so every permitted pico amount, including sums, has an exact stored form.
pub const MAX_PICO: i128 = 9_999_999_999_999_999_999_999_999_999_999_999;
// BSON Decimal128 uses IEEE 754 BID, little-endian. Every valid coefficient
// fits the normal 113-bit significand. The biased exponent for picocredits is
// 6176 - 12; accepted Credits never need the alternate combination-field form.
const DECIMAL_PICO_EXPONENT: u128 = 6176 - 12;
const DECIMAL_COEFFICIENT_MASK: u128 = (1_u128 << 113) - 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, ToSchema)]
#[schema(value_type = String, example = "0.0000008")]
pub struct Credits(i128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid or unrepresentable credit amount")]
pub struct CreditsError;

impl Credits {
    pub const ZERO: Self = Self(0);

    pub fn from_pico(value: i128) -> Result<Self, CreditsError> {
        if !(-MAX_PICO..=MAX_PICO).contains(&value) {
            return Err(CreditsError);
        }
        Ok(Self(value))
    }

    // An i64 in either legacy unit always fits the supported Decimal128 range.
    pub const fn from_whole(value: i64) -> Self {
        Self(value as i128 * SCALE)
    }
    pub const fn from_micros(value: i64) -> Self {
        Self(value as i128 * 1_000_000)
    }
    pub const fn pico(self) -> i128 {
        self.0
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, CreditsError> {
        Self::from_pico(self.0.checked_add(rhs.0).ok_or(CreditsError)?)
    }
    pub fn checked_sub(self, rhs: Self) -> Result<Self, CreditsError> {
        Self::from_pico(self.0.checked_sub(rhs.0).ok_or(CreditsError)?)
    }
    pub fn checked_mul(self, quantity: i64) -> Result<Self, CreditsError> {
        Self::from_pico(
            self.0
                .checked_mul(i128::from(quantity))
                .ok_or(CreditsError)?,
        )
    }
    pub fn checked_sum(values: impl IntoIterator<Item = Self>) -> Result<Self, CreditsError> {
        values.into_iter().try_fold(Self::ZERO, Self::checked_add)
    }
    pub fn checked_neg(self) -> Result<Self, CreditsError> {
        Self::ZERO.checked_sub(self)
    }

    /// Display compatibility only. Truncate once, after exact aggregation.
    pub fn legacy_whole(self) -> Result<i64, CreditsError> {
        (self.0 / SCALE).try_into().map_err(|_| CreditsError)
    }
    /// Display compatibility only. Truncate once, after exact aggregation.
    pub fn legacy_micros(self) -> Result<i64, CreditsError> {
        (self.0 / 1_000_000).try_into().map_err(|_| CreditsError)
    }
    /// Bounded legacy display field; never use this projection for accounting.
    pub fn display_whole(self) -> i64 {
        (self.0 / SCALE).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }
    /// Bounded legacy display field; never use this projection for accounting.
    pub fn display_micros(self) -> i64 {
        (self.0 / 1_000_000).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }
    pub fn decimal(self) -> Decimal128 {
        // Preserve the same normalized bytes as parsing our canonical string,
        // without formatting and reparsing every amount in every rollup batch.
        let mut coefficient = self.0.unsigned_abs();
        let mut exponent = DECIMAL_PICO_EXPONENT;
        if coefficient == 0 {
            exponent = 6176;
        } else {
            // Canonical credit strings retain whole-credit zeroes (1000 is
            // exponent zero, not 1E+3).
            while exponent < 6176 && coefficient.is_multiple_of(10) {
                coefficient /= 10;
                exponent += 1;
            }
        }
        let sign = u128::from(self.0 < 0) << 127;
        Decimal128::from_bytes((sign | (exponent << 113) | coefficient).to_le_bytes())
    }
    pub fn from_bson(value: Bson, legacy_scale: i128) -> Result<Self, CreditsError> {
        match value {
            Bson::Decimal128(value) => {
                let bits = u128::from_le_bytes(value.bytes());
                let exponent = (bits >> 113) & 0x3fff;
                // Fast path for canonical stored credits and ordinary Mongo
                // sums. Other exponents retain the full library parser, which
                // also handles trailing-zero precision and rejects specials.
                if (DECIMAL_PICO_EXPONENT..=6176).contains(&exponent) {
                    let coefficient = (bits & DECIMAL_COEFFICIENT_MASK) as i128;
                    let pico = coefficient
                        .checked_mul(10_i128.pow((exponent - DECIMAL_PICO_EXPONENT) as u32))
                        .ok_or(CreditsError)?;
                    Self::from_pico(if bits >> 127 == 0 { pico } else { -pico })
                } else {
                    value.to_string().parse()
                }
            }
            Bson::Int32(value) => Self::from_pico(i128::from(value) * legacy_scale),
            Bson::Int64(value) => Self::from_pico(i128::from(value) * legacy_scale),
            _ => Err(CreditsError),
        }
    }
}

impl fmt::Display for Credits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let magnitude = self.0.unsigned_abs();
        if self.0 < 0 {
            f.write_str("-")?;
        }
        write!(f, "{}", magnitude / SCALE as u128)?;
        let fraction = magnitude % SCALE as u128;
        if fraction != 0 {
            write!(f, ".{}", format!("{fraction:012}").trim_end_matches('0'))?;
        }
        Ok(())
    }
}

impl FromStr for Credits {
    type Err = CreditsError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (mantissa, exponent) = match input.find(['e', 'E']) {
            Some(index) => (
                &input[..index],
                input[index + 1..]
                    .parse::<i32>()
                    .map_err(|_| CreditsError)?,
            ),
            None => (input, 0),
        };
        let (negative, mantissa) = if let Some(value) = mantissa.strip_prefix('-') {
            (true, value)
        } else {
            (false, mantissa.strip_prefix('+').unwrap_or(mantissa))
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if (whole.is_empty() && fraction.is_empty())
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return Err(CreditsError);
        }
        let combined = format!("{whole}{fraction}");
        let digits = combined.trim_start_matches('0');
        if digits.is_empty() {
            return Ok(Self::ZERO);
        }
        let shift = 12_i64 + i64::from(exponent)
            - i64::try_from(fraction.len()).map_err(|_| CreditsError)?;
        let pico = if shift < 0 {
            let cut = usize::try_from(-shift).map_err(|_| CreditsError)?;
            if cut >= digits.len() || !digits[digits.len() - cut..].bytes().all(|b| b == b'0') {
                return Err(CreditsError);
            }
            digits[..digits.len() - cut]
                .parse::<i128>()
                .map_err(|_| CreditsError)?
        } else {
            if shift > 34 || digits.len() + shift as usize > 34 {
                return Err(CreditsError);
            }
            digits
                .parse::<i128>()
                .map_err(|_| CreditsError)?
                .checked_mul(10_i128.pow(shift as u32))
                .ok_or(CreditsError)?
        };
        Self::from_pico(if negative { -pico } else { pico })
    }
}

impl Serialize for Credits {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for Credits {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}
impl From<Credits> for Bson {
    fn from(value: Credits) -> Self {
        Self::Decimal128(value.decimal())
    }
}

macro_rules! storage_adapter {
    ($name:ident, $scale:expr) => {
        pub mod $name {
            use super::*;
            pub fn serialize<S: Serializer>(
                value: &Credits,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                value.decimal().serialize(serializer)
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Credits, D::Error> {
                Credits::from_bson(Bson::deserialize(deserializer)?, $scale)
                    .map_err(serde::de::Error::custom)
            }
            pub mod optional {
                use super::*;
                pub fn serialize<S: Serializer>(
                    value: &Option<Credits>,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    value.map(Credits::decimal).serialize(serializer)
                }
                pub fn deserialize<'de, D: Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Option<Credits>, D::Error> {
                    Option::<Bson>::deserialize(deserializer)?
                        .map(|value| Credits::from_bson(value, $scale))
                        .transpose()
                        .map_err(serde::de::Error::custom)
                }
            }
        }
    };
}
storage_adapter!(whole, SCALE);
#[cfg(test)]
storage_adapter!(micros, 1_000_000);

impl std::ops::Neg for Credits {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

/// BSON/JSON model compatibility: prefer the unit-free key; normalize a legacy
/// integer only when that key is absent. A removed key can never override v2.
pub fn normalize_legacy_fields(
    document: &mut bson::Document,
    aliases: &[(&str, &str)],
) -> Result<(), CreditsError> {
    for &(new, old) in aliases {
        if let Some(legacy) = document.remove(old)
            && !document.contains_key(new)
        {
            if legacy == bson::Bson::Null {
                document.insert(new, legacy);
            } else {
                document.insert(new, Credits::from_bson(legacy, 1_000_000)?);
            }
        }
    }
    Ok(())
}

/// Generate a model's compatibility deserializer without duplicating its field
/// declarations. The private wire struct retains all serde field conventions.
#[macro_export]
macro_rules! exact_credit_model {
    ([$(($new:literal, $old:literal)),* $(,)?]
     $(#[$meta:meta])* pub struct $name:ident {
        $($(#[$field_meta:meta])* pub $field:ident: $ty:ty),* $(,)?
    }) => {
        $(#[$meta])* pub struct $name {
            $($(#[$field_meta])* pub $field: $ty),*
        }
        #[allow(deprecated)]
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                struct Wire { $($(#[$field_meta])* $field: $ty),* }
                let mut document = <bson::Document as serde::Deserialize>::deserialize(deserializer)?;
                $crate::models::credits::normalize_legacy_fields(&mut document, &[$(($new, $old)),*])
                    .map_err(serde::de::Error::custom)?;
                // Consume the normalized document directly. A BSON bytes round
                // trip here decodes every field a third time on hot models.
                let wire: Wire = bson::from_document_with_options(
                    document,
                    bson::de::DeserializerOptions::builder().human_readable(false).build(),
                ).map_err(serde::de::Error::custom)?;
                Ok(Self { $($field: wire.$field),* })
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::exact_credit_model! {
        [("amount", "amount_micros")]
        #[derive(Debug, Serialize, PartialEq)]
        pub struct LegacyProbe {
            #[serde(with = "crate::models::credits::whole")]
            pub amount: Credits,
        }
    }
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Stored {
        #[serde(with = "whole")]
        whole: Credits,
        #[serde(with = "micros")]
        micros: Credits,
        #[serde(default, with = "micros::optional")]
        optional: Option<Credits>,
    }
    #[test]
    fn checked_arithmetic_and_range() {
        let amount: Credits = "0.0000008".parse().unwrap();
        assert_eq!(amount.checked_mul(44_608).unwrap().to_string(), "0.0356864");
        assert!(
            Credits::from_pico(MAX_PICO)
                .unwrap()
                .checked_add(amount)
                .is_err()
        );
        assert!(Credits::from_pico(i128::MAX).is_err());
        assert_eq!(
            Credits::from_whole(i64::MAX).to_string(),
            i64::MAX.to_string()
        );
    }
    #[test]
    fn scientific_notation_and_precision() {
        for (raw, expected) in [
            ("8E-7", "0.0000008"),
            ("0.0000008000000", "0.0000008"),
            ("1E+3", "1000"),
            ("1.23E+3", "1230"),
            ("-0E-6176", "0"),
            ("-1E-12", "-0.000000000001"),
            ("1000E-15", "0.000000000001"),
        ] {
            assert_eq!(
                raw.parse::<Credits>().unwrap().to_string(),
                expected,
                "{raw}"
            );
        }
        for raw in [
            "NaN",
            "sNaN",
            "Infinity",
            "-Infinity",
            "1E-13",
            "0.0000000000001",
            "1E+23",
            "1E2147483647",
            "1E-2147483648",
            "",
            ".",
            "1.2.3",
        ] {
            assert!(raw.parse::<Credits>().is_err(), "{raw}");
        }
    }
    #[test]
    fn json_and_bson_are_exact_and_distinct() {
        let amount: Credits = "9999.9999992".parse().unwrap();
        assert_eq!(serde_json::to_string(&amount).unwrap(), "\"9999.9999992\"");
        assert_eq!(
            serde_json::from_str::<Credits>("\"9999.9999992\"").unwrap(),
            amount
        );
        assert!(serde_json::from_str::<Credits>("0.6").is_err());
        let stored = Stored {
            whole: amount,
            micros: amount,
            optional: Some(amount),
        };
        let document = bson::to_document(&stored).unwrap();
        assert!(matches!(document.get("whole"), Some(Bson::Decimal128(_))));
        assert_eq!(bson::from_document::<Stored>(document).unwrap(), stored);
        assert_eq!(
            bson::from_slice::<Stored>(&bson::to_vec(&stored).unwrap()).unwrap(),
            stored
        );
        let old =
            bson::from_document::<Stored>(bson::doc! { "whole": 2_i64, "micros": 800_000_i32 })
                .unwrap();
        assert_eq!(old.whole, Credits::from_whole(2));
        assert_eq!(old.micros.to_string(), "0.8");
        assert_eq!(old.optional, None);
        assert!(Credits::from_bson(Bson::Double(0.6), SCALE).is_err());
    }

    #[test]
    fn macro_legacy_amount_is_normalized_at_micro_scale() {
        let value = bson::from_document::<LegacyProbe>(bson::doc! {
            "amount_micros": 472_i64,
        })
        .unwrap();
        assert_eq!(value.amount, Credits::from_micros(472));
    }
    #[test]
    fn decimal_encoding_matches_bson_reference_at_boundaries_and_random_values() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(1672);
        let boundaries = [
            -MAX_PICO,
            -SCALE,
            -1,
            0,
            1,
            10,
            SCALE,
            1_000 * SCALE,
            MAX_PICO,
        ];
        for pico in boundaries
            .into_iter()
            .chain((0..10_000).map(|_| rng.gen_range(-MAX_PICO..=MAX_PICO)))
        {
            let amount = Credits::from_pico(pico).unwrap();
            let reference: Decimal128 = amount.to_string().parse().unwrap();
            assert_eq!(amount.decimal().bytes(), reference.bytes());
            assert_eq!(Credits::from_bson(reference.into(), SCALE).unwrap(), amount);
        }
        for source in ["8E-7", "1000E-15", "1E+3", "-0E-6176"] {
            let value: Decimal128 = source.parse().unwrap();
            assert_eq!(
                Credits::from_bson(value.into(), SCALE).unwrap(),
                source.parse::<Credits>().unwrap()
            );
        }
        for source in ["NaN", "Infinity", "1E-13", "1E+23"] {
            let value: Decimal128 = source.parse().unwrap();
            assert!(Credits::from_bson(value.into(), SCALE).is_err());
        }
    }

    #[test]
    fn randomized_decimal_round_trips_and_conservation() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(1672);
        for _ in 0..10_000 {
            let pico = rng.gen_range(-MAX_PICO..=MAX_PICO);
            let value = Credits::from_pico(pico).unwrap();
            assert_eq!(value.to_string().parse::<Credits>().unwrap(), value);
            assert_eq!(Credits::from_bson(value.into(), SCALE).unwrap(), value);
            let rate = Credits::from_pico(rng.gen_range(0..=1_000_000_000_000_000_000)).unwrap();
            let quantity = rng.gen_range(0..=1_000_000);
            let total = rate.checked_mul(quantity).unwrap();
            let allowance = rate.checked_mul(rng.gen_range(0..=quantity)).unwrap();
            let remainder = total.checked_sub(allowance).unwrap();
            let grant = Credits::from_pico(rng.gen_range(0..=remainder.pico())).unwrap();
            let wallet = remainder.checked_sub(grant).unwrap();
            assert_eq!(
                Credits::checked_sum([allowance, grant, wallet]).unwrap(),
                total
            );
        }
    }
}
