use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_event_logs";

/// Metadata-only record of a device event forwarded through the Event Gateway.
///
/// Per ADR-013 (NyxID Pure Passthrough), this log stores only envelope metadata
/// and never the event payload. Payload content lives exclusively in the
/// downstream agent's state (Aevatar grain state).
///
/// The collection is append-only: multiple rows may exist for the same
/// `(conversation_id, event_id)` pair when a client retries after a
/// transient callback failure or when a later duplicate is recorded as a
/// dedup hit. The `(conversation_id, event_id)` index is non-unique by
/// design — see `db::ensure_indexes` for the rationale.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelEventLog {
    #[serde(rename = "_id")]
    pub id: String,
    pub conversation_id: String,
    /// API key (agent identity) that forwarded the event.
    pub api_key_id: String,
    /// Client-supplied event id from the envelope. Used for idempotency.
    pub event_id: String,
    /// Logical source of the event (e.g. "camera-analyzer").
    pub source: String,
    /// Event type (e.g. "person_detected").
    pub event_type: String,
    /// Event timestamp as reported by the client (envelope `timestamp`).
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub event_timestamp: DateTime<Utc>,
    /// When the gateway actually forwarded the event.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub forwarded_at: DateTime<Utc>,
    /// Upstream HTTP status code from the agent callback (0 on transport error).
    pub upstream_status_code: i32,
    /// Round-trip duration measured around `forward_to_agent()` in milliseconds.
    /// Required by ADR-013 §4 metadata fields.
    pub latency_ms: i64,
    /// Outcome classification for dashboards:
    /// `"delivered"` | `"deduped"` | `"rate_limited"` | `"callback_failed"`.
    pub outcome: String,
}

pub const OUTCOME_DELIVERED: &str = "delivered";
pub const OUTCOME_DEDUPED: &str = "deduped";
pub const OUTCOME_RATE_LIMITED: &str = "rate_limited";
pub const OUTCOME_CALLBACK_FAILED: &str = "callback_failed";

#[cfg(test)]
mod tests {
    use super::*;

    fn make_log() -> ChannelEventLog {
        ChannelEventLog {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            api_key_id: uuid::Uuid::new_v4().to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            source: "camera-analyzer".to_string(),
            event_type: "person_detected".to_string(),
            event_timestamp: Utc::now(),
            forwarded_at: Utc::now(),
            upstream_status_code: 202,
            latency_ms: 42,
            outcome: OUTCOME_DELIVERED.to_string(),
        }
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "channel_event_logs");
    }

    #[test]
    fn bson_roundtrip() {
        let log = make_log();
        let doc = bson::to_document(&log).expect("serialize");
        let restored: ChannelEventLog = bson::from_document(doc).expect("deserialize");
        assert_eq!(log.id, restored.id);
        assert_eq!(log.conversation_id, restored.conversation_id);
        assert_eq!(log.event_id, restored.event_id);
        assert_eq!(log.upstream_status_code, restored.upstream_status_code);
        assert_eq!(log.latency_ms, restored.latency_ms);
        assert_eq!(log.outcome, restored.outcome);
    }

    #[test]
    fn bson_no_payload_field() {
        // ADR-013 compliance: verify the serialized document never contains
        // a field that could accidentally hold payload content.
        let log = make_log();
        let doc = bson::to_document(&log).expect("serialize");
        assert!(!doc.contains_key("payload"));
        assert!(!doc.contains_key("content"));
        assert!(!doc.contains_key("body"));
        assert!(!doc.contains_key("metadata"));
    }

    #[test]
    fn bson_required_fields_present() {
        let log = make_log();
        let doc = bson::to_document(&log).expect("serialize");
        assert!(doc.contains_key("_id"));
        assert!(doc.contains_key("conversation_id"));
        assert!(doc.contains_key("api_key_id"));
        assert!(doc.contains_key("event_id"));
        assert!(doc.contains_key("source"));
        assert!(doc.contains_key("event_type"));
        assert!(doc.contains_key("event_timestamp"));
        assert!(doc.contains_key("forwarded_at"));
        assert!(doc.contains_key("upstream_status_code"));
        assert!(doc.contains_key("latency_ms"));
        assert!(doc.contains_key("outcome"));
    }
}
