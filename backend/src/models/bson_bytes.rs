/// Serde helpers for `Vec<u8>` and `Option<Vec<u8>>` stored in MongoDB.
///
/// MongoDB stores byte arrays in two ways depending on how they were written:
/// - `insert_one(&struct)` with default serde -> BSON Array of integers
/// - `$set` with `bson::Binary` / `serde_bytes` -> BSON Binary
///
/// These helpers serialize as BSON Binary (compact, canonical) but deserialize
/// from either BSON Binary or BSON Array for backward compatibility.

use bson::Bson;
use serde::{Deserialize, Deserializer, Serializer};

fn bytes_from_bson<E: serde::de::Error>(bson: Bson) -> Result<Vec<u8>, E> {
    match bson {
        Bson::Binary(bin) => Ok(bin.bytes),
        Bson::Array(arr) => arr
            .into_iter()
            .map(|v| match v {
                Bson::Int32(n) => u8::try_from(n).map_err(|_| {
                    E::custom(format!("array element {n} out of u8 range"))
                }),
                Bson::Int64(n) => u8::try_from(n).map_err(|_| {
                    E::custom(format!("array element {n} out of u8 range"))
                }),
                other => Err(E::custom(format!(
                    "expected integer in byte array, got {other:?}"
                ))),
            })
            .collect(),
        other => Err(E::custom(format!(
            "expected Binary or Array for bytes, got {other:?}"
        ))),
    }
}

/// For required `Vec<u8>` fields.
///
/// Usage: `#[serde(with = "bson_bytes::required")]`
pub mod required {
    use super::*;

    pub fn serialize<S: Serializer>(val: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        serde_bytes::serialize(val, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let bson = Bson::deserialize(deserializer)?;
        bytes_from_bson(bson)
    }
}

/// For optional `Option<Vec<u8>>` fields.
///
/// Usage: `#[serde(with = "bson_bytes::optional")]`
pub mod optional {
    use super::*;

    pub fn serialize<S: Serializer>(
        val: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match val {
            Some(bytes) => serde_bytes::serialize(bytes, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        let bson = Option::<Bson>::deserialize(deserializer)?;
        match bson {
            Some(Bson::Null) | None => Ok(None),
            Some(b) => bytes_from_bson(b).map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestRequired {
        #[serde(with = "super::required")]
        pub data: Vec<u8>,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestOptional {
        #[serde(default, with = "super::optional")]
        pub data: Option<Vec<u8>>,
    }

    #[test]
    fn required_roundtrip_via_bson() {
        let original = TestRequired {
            data: vec![1, 2, 3, 255],
        };
        let doc = bson::to_document(&original).expect("serialize");
        let restored: TestRequired = bson::from_document(doc).expect("deserialize");
        assert_eq!(original.data, restored.data);
    }

    #[test]
    fn required_deserializes_bson_array() {
        // Simulate data stored via insert_one with default serde (BSON Array of ints)
        let doc = bson::doc! { "data": [1_i32, 2_i32, 3_i32, 255_i32] };
        let restored: TestRequired = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.data, vec![1, 2, 3, 255]);
    }

    #[test]
    fn required_deserializes_bson_binary() {
        // Simulate data stored as BSON Binary (via $set or serde_bytes)
        let doc = bson::doc! { "data": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![10, 20, 30] } };
        let restored: TestRequired = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.data, vec![10, 20, 30]);
    }

    #[test]
    fn optional_some_roundtrip_via_bson() {
        let original = TestOptional {
            data: Some(vec![4, 5, 6]),
        };
        let doc = bson::to_document(&original).expect("serialize");
        let restored: TestOptional = bson::from_document(doc).expect("deserialize");
        assert_eq!(original.data, restored.data);
    }

    #[test]
    fn optional_none_roundtrip_via_bson() {
        let original = TestOptional { data: None };
        let doc = bson::to_document(&original).expect("serialize");
        let restored: TestOptional = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.data, None);
    }

    #[test]
    fn optional_deserializes_bson_array() {
        let doc = bson::doc! { "data": [7_i32, 8_i32] };
        let restored: TestOptional = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.data, Some(vec![7, 8]));
    }

    #[test]
    fn optional_deserializes_bson_binary() {
        let doc = bson::doc! { "data": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![9] } };
        let restored: TestOptional = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.data, Some(vec![9]));
    }

    #[test]
    fn optional_missing_field_defaults_to_none() {
        let doc = bson::doc! {};
        let restored: TestOptional = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.data, None);
    }

    #[test]
    fn optional_null_deserializes_to_none() {
        let doc = bson::doc! { "data": bson::Bson::Null };
        let restored: TestOptional = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.data, None);
    }
}
