use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "oracle_sessions";

pub fn default_session_origin() -> String {
    "nyxid".into()
}

/// A multi-turn oracle conversation. Turn bodies live on the tasks
/// themselves (query `oracle_tasks` by `conversation_id`); the session
/// carries only routing state — most importantly the browser-side
/// conversation URL workers navigate back to for follow-ups.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OracleSession {
    /// Conversation id (`conv_<hex16>`), minted at session open.
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    /// The submitter who opened the session; only they may continue it.
    pub owner_user_id: String,
    /// "nyxid" for sessions opened by NyxID prompts; "imported" for
    /// sessions attached from an existing ChatGPT conversation.
    #[serde(default = "default_session_origin")]
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Browser-side conversation URL pinned by the worker after turn 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_url: Option<String>,
    /// Worker label of the account that created this conversation, stamped
    /// on the first result. Follow-ups copy it onto their task as
    /// `required_worker_label` so multi-turn lands back on the owning
    /// account in a multi-account pool. `None` for legacy/unstamped
    /// sessions (unpinned, today's behavior).
    ///
    /// Affinity keys on the worker *label*, so correctness rests on the
    /// operational invariant that one stable label maps to one ChatGPT
    /// account. Two tabs of the same account under different labels are
    /// treated as different accounts (over-pinning — harmless beyond lost
    /// load-balancing); two different accounts sharing a label would
    /// reintroduce the misroute this pinning prevents. Worker label
    /// assignment lives in the CDP/userscript clients, which already mint a
    /// stable per-tab label (`?nyx=N` → `tab_N`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_worker_label: Option<String>,
    pub turn_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "oracle_sessions");
    }

    #[test]
    fn bson_roundtrip() {
        let session = OracleSession {
            id: "conv_0123456789abcdef".to_string(),
            pool_id: uuid::Uuid::new_v4().to_string(),
            owner_user_id: uuid::Uuid::new_v4().to_string(),
            origin: "nyxid".to_string(),
            api_key_id: Some(uuid::Uuid::new_v4().to_string()),
            tag: Some("bedc-deep".to_string()),
            chatgpt_url: Some("https://chatgpt.com/c/abc".to_string()),
            owner_worker_label: Some("tab_1".to_string()),
            turn_count: 3,
            last_task_id: Some("task-3".to_string()),
            closed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&session).expect("serialize");
        let restored: OracleSession = bson::from_document(doc).expect("deserialize");
        assert_eq!(session.id, restored.id);
        assert_eq!(restored.turn_count, 3);
        assert_eq!(restored.origin, "nyxid");
        assert!(restored.closed_at.is_none());
        assert_eq!(
            restored.chatgpt_url.as_deref(),
            Some("https://chatgpt.com/c/abc")
        );
    }

    #[test]
    fn bson_roundtrip_closed() {
        let session = OracleSession {
            id: "conv_ffffffffffffffff".to_string(),
            pool_id: "p1".to_string(),
            owner_user_id: "u1".to_string(),
            origin: "nyxid".to_string(),
            api_key_id: None,
            tag: None,
            chatgpt_url: None,
            owner_worker_label: None,
            turn_count: 0,
            last_task_id: None,
            closed_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&session).expect("serialize");
        let restored: OracleSession = bson::from_document(doc).expect("deserialize");
        assert!(restored.closed_at.is_some());
    }

    #[test]
    fn missing_origin_defaults_to_nyxid() {
        let session = OracleSession {
            id: "conv_0123456789abcdef".to_string(),
            pool_id: "p1".to_string(),
            owner_user_id: "u1".to_string(),
            origin: "nyxid".to_string(),
            api_key_id: None,
            tag: None,
            chatgpt_url: None,
            owner_worker_label: None,
            turn_count: 0,
            last_task_id: None,
            closed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut doc = bson::to_document(&session).expect("serialize");
        doc.remove("origin");
        let restored: OracleSession = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.origin, "nyxid");
    }
}
