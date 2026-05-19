use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "reply_token_uses";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplyTokenUse {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub exp_at: DateTime<Utc>,
    pub api_key_id: String,
    pub conversation_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub consumed_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "reply_token_uses");
    }

    #[test]
    fn bson_roundtrip() {
        let now = Utc::now();
        let usage = ReplyTokenUse {
            id: uuid::Uuid::new_v4().to_string(),
            exp_at: now,
            api_key_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            consumed_at: now,
        };

        let doc = bson::to_document(&usage).expect("serialize");
        let restored: ReplyTokenUse = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.id, usage.id);
        assert_eq!(restored.api_key_id, usage.api_key_id);
        assert_eq!(restored.conversation_id, usage.conversation_id);
    }
}
