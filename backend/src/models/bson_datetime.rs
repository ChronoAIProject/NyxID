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
