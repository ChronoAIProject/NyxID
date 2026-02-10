/// Serde helper for `Option<DateTime<Utc>>` as BSON DateTime.
///
/// The `bson` crate provides `chrono_datetime_as_bson_datetime` for required
/// fields but not for optional ones. This module fills that gap.
pub mod optional {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        val: &Option<DateTime<Utc>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match val {
            Some(dt) => {
                bson::serde_helpers::chrono_datetime_as_bson_datetime::serialize(dt, serializer)
            }
            None => Option::<bson::DateTime>::None.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<DateTime<Utc>>, D::Error> {
        let opt = Option::<bson::DateTime>::deserialize(deserializer)?;
        Ok(opt.map(|dt| dt.to_chrono()))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};

    /// Test struct using the optional bson_datetime helper.
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestOptional {
        #[serde(default, with = "super::optional")]
        pub ts: Option<DateTime<Utc>>,
    }

    /// Test struct using the required bson_datetime helper.
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestRequired {
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        pub ts: DateTime<Utc>,
    }

    #[test]
    fn optional_some_roundtrip_via_bson() {
        let now = Utc::now();
        let original = TestOptional { ts: Some(now) };
        let doc = bson::to_document(&original).expect("serialize to bson doc");
        let restored: TestOptional = bson::from_document(doc).expect("deserialize from bson doc");
        // BSON DateTime has millisecond precision; truncate for comparison
        let expected_ms = now.timestamp_millis();
        let actual_ms = restored.ts.expect("should be Some").timestamp_millis();
        assert_eq!(expected_ms, actual_ms);
    }

    #[test]
    fn optional_none_roundtrip_via_bson() {
        let original = TestOptional { ts: None };
        let doc = bson::to_document(&original).expect("serialize to bson doc");
        let restored: TestOptional = bson::from_document(doc).expect("deserialize from bson doc");
        assert_eq!(restored.ts, None);
    }

    #[test]
    fn required_datetime_roundtrip_via_bson() {
        let now = Utc::now();
        let original = TestRequired { ts: now };
        let doc = bson::to_document(&original).expect("serialize to bson doc");
        let restored: TestRequired = bson::from_document(doc).expect("deserialize from bson doc");
        assert_eq!(now.timestamp_millis(), restored.ts.timestamp_millis());
    }
}
