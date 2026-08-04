use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "invite_codes";

/// Per-usage record of who redeemed an invite code and when.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteCodeUsage {
    pub user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub used_at: DateTime<Utc>,
    /// Normalized (trimmed + lowercased) email of the registrant. Absent on
    /// legacy usages recorded before per-identity enforcement was added.
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub code: String,
    pub max_uses: i32,
    pub used_count: i32,
    pub created_by: String,
    pub note: Option<String>,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    /// List of users who redeemed this invite code, with timestamps.
    /// Defaults to empty for back-compat with codes created before the
    /// usage-tracking change landed.
    #[serde(default)]
    pub usages: Vec<InviteCodeUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_roundtrip() {
        let code = InviteCode {
            id: uuid::Uuid::new_v4().to_string(),
            code: "NYXID-ABC123".to_string(),
            max_uses: 10,
            used_count: 2,
            created_by: uuid::Uuid::new_v4().to_string(),
            note: Some("Beta testers".to_string()),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            usages: vec![InviteCodeUsage {
                user_id: uuid::Uuid::new_v4().to_string(),
                used_at: Utc::now(),
                email: Some("test@example.com".to_string()),
            }],
        };
        let doc = bson::to_document(&code).expect("serialize");
        let restored: InviteCode = bson::from_document(doc).expect("deserialize");
        assert_eq!(code.id, restored.id);
        assert_eq!(code.code, restored.code);
        assert_eq!(code.max_uses, restored.max_uses);
        assert_eq!(code.used_count, restored.used_count);
        assert_eq!(restored.usages.len(), 1);
    }

    #[test]
    fn bson_backward_compat_missing_usages() {
        let code = InviteCode {
            id: "id".to_string(),
            code: "CODE".to_string(),
            max_uses: 1,
            used_count: 0,
            created_by: "admin".to_string(),
            note: None,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            usages: vec![],
        };
        let mut doc = bson::to_document(&code).expect("serialize");
        doc.remove("usages");
        let restored: InviteCode = bson::from_document(doc).expect("deserialize");
        assert!(restored.usages.is_empty());
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "invite_codes");
    }
}
