use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

pub const MAX_QUANTITY: u128 = 10_u128.pow(38) - 1;
pub const MAX_TIME: u64 = i64::MAX as u64;

fn decimal(value: &str, maximum: u128, positive: bool) -> Result<u128, &'static str> {
    if value.is_empty()
        || value.len() > 38
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err("noncanonical_decimal");
    }
    let value = value.parse::<u128>().map_err(|_| "decimal_overflow")?;
    if value > maximum || (positive && value == 0) {
        return Err("decimal_out_of_range");
    }
    Ok(value)
}

macro_rules! integer_string {
    ($name:ident, $inner:ty, $max:expr, $positive:expr) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
        pub struct $name(pub(super) $inner);
        impl $name {
            pub fn get(self) -> $inner {
                self.0
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0.to_string())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                let value = decimal(&value, $max, $positive).map_err(D::Error::custom)?;
                Ok(Self(value as $inner))
            }
        }
    };
}
integer_string!(Quantity, u128, MAX_QUANTITY, false);
integer_string!(Counter, u64, MAX_TIME as u128, true);
integer_string!(Millis, u64, MAX_TIME as u128, false);
integer_string!(SourcePart, u64, MAX_TIME as u128, false);

macro_rules! checked_string {
    ($name:ident, $valid:expr) => {
        #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub(super) String);
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                if !($valid)(&value) {
                    return Err(D::Error::custom(stringify!($name)));
                }
                Ok(Self(value))
            }
        }
    };
}
checked_string!(Text, |s: &str| !s.is_empty()
    && s.len() <= 256
    && s.trim() == s
    && !s.chars().any(char::is_control));
checked_string!(Digest, |s: &str| s.len() == 64
    && s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
checked_string!(Id, |s: &str| uuid::Uuid::parse_str(s).is_ok_and(|id| id
    .get_version()
    == Some(uuid::Version::Random)
    && id.get_variant() == uuid::Variant::RFC4122
    && id.to_string() == s));

/// Nullable fields remain required on the wire; omission must not become null.
pub(super) fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
