//! Remote CLI pairing — Mode B of the wizard flow.
//!
//! The CLI (authenticated by the user's session/access token) creates a
//! pairing record and prints a short human-typeable code + a pair URL.
//! The user opens the pair URL on a device that CAN run a browser, logs
//! in as themselves, enters the code, and completes the DisplayOnce
//! wizard on the frontend. The CLI polls until the frontend posts a
//! typed ack; the ack carries only non-secret identifiers (UUIDs, the
//! same shape as the local-server DisplayOnce flows).
//!
//! Security posture mirrors RFC 8628:
//!   - the code is bound to `user_id` at creation; claim rejects on mismatch
//!   - the code is stored only as an HMAC-SHA256 hash (server-side secret)
//!   - TTL is enforced by a MongoDB TTL index on `expires_at`
//!   - failed claim attempts are counted and capped
//!   - ack payloads that flow back to the CLI are drawn from the same
//!     narrow typed shapes used by the DisplayOnce local-server path —
//!     secrets are NEVER stored server-side in this record

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "cli_pairings";

/// Lifecycle states for a pairing record. The happy path is
/// `Pending -> Claimed -> Completed`; the CLI polls until `Completed`
/// (or a terminal failure state) and then exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliPairingStatus {
    /// CLI created the pairing; waiting for the user to enter the code.
    Pending,
    /// User entered the code on the frontend; wizard is in progress.
    Claimed,
    /// Frontend posted the typed ack; CLI's next poll returns it.
    Completed,
    /// User explicitly cancelled the wizard on the frontend.
    Cancelled,
}

/// MongoDB document. `_id` is a UUID v4 string (the CLI uses this id,
/// not the code, for polling — the code is only for the user's eyes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliPairing {
    #[serde(rename = "_id")]
    pub id: String,

    /// Bound at creation time; claim and complete enforce this matches
    /// the session user. This is the primary anti-phishing guard: a
    /// shoulder-surfer who grabs the code still can't claim it unless
    /// they are logged into the frontend as the same user.
    pub user_id: String,

    /// HMAC-SHA256(code, server_secret) — the plaintext code is
    /// returned once at creation and never stored. Unique index so the
    /// (astronomically unlikely) hash collision is rejected server-side.
    pub code_hash: String,

    /// Opaque flow identifier that maps 1:1 to `FlowKind` in the CLI
    /// wizard (`api-key-create`, `node-register-token`, `ai-key`, etc.).
    /// Persisted as a string so adding new flows doesn't require a
    /// migration; the frontend uses it to pick the right wizard panel.
    pub kind: String,

    /// JSON-encoded flow-specific prefill (e.g. `{"name": "my-agent",
    /// "platform": "claude-code"}` for api-key-create). Whatever shape
    /// the CLI put in is what the frontend gets back on claim.
    ///
    /// CRITICAL: the CLI MUST NOT put secrets in here. This field is
    /// returned to the browser in cleartext on claim — treat it like
    /// a query-string.
    pub prefill: serde_json::Value,

    pub status: CliPairingStatus,

    /// Populated when `status` transitions to `Claimed`.
    #[serde(default, with = "bson_datetime::optional")]
    pub claimed_at: Option<DateTime<Utc>>,

    /// Remote IP of the browser that claimed the code. Logged for audit
    /// only — we do NOT pin the IP (mobile/VPN false-positive rate is
    /// too high; see design notes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_from_ip: Option<String>,

    /// Populated when `status` transitions to `Completed`.
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,

    /// Set by `POST /cli-pairings/{id}/reserve-action` right before
    /// the frontend executes the destructive API call for a
    /// DisplayOnce flow. Read on re-claim (when `resumed: true`) to
    /// distinguish the recoverable "user refreshed BEFORE clicking
    /// Create" case from the dangerous "user refreshed AFTER the
    /// mint already happened" case — the former is safe to resume,
    /// the latter must be blocked because a replay would invalidate
    /// the secret the user already saved.
    #[serde(default, with = "bson_datetime::optional")]
    pub action_started_at: Option<DateTime<Utc>>,

    /// Typed ack payload written by the frontend on completion. Shape
    /// is flow-specific and MUST match one of the `*AckPayload` structs
    /// in `cli/src/wizard/mod.rs`. Re-serialized and returned verbatim
    /// on the CLI's poll response.
    ///
    /// Size-limited and field-allowlisted on the way in (see
    /// `cli_pairing_service::complete`) so a buggy frontend can't
    /// smuggle secret material into this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack_payload: Option<serde_json::Value>,

    /// Count of failed claim attempts (wrong user, wrong status, etc.).
    /// Used to lock the pairing after a threshold to prevent brute
    /// forcing the code even within the TTL window.
    #[serde(default)]
    pub failed_claim_attempts: u32,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,

    /// TTL index auto-deletes the record when this time passes. The CLI
    /// sees `404` on poll after expiry (or `status: expired` if we
    /// caught it before the sweeper ran).
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "cli_pairings");
    }

    fn make_pairing() -> CliPairing {
        CliPairing {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            code_hash: "deadbeef".repeat(8),
            kind: "api-key-create".to_string(),
            prefill: serde_json::json!({"name": "test"}),
            status: CliPairingStatus::Pending,
            claimed_at: None,
            claimed_from_ip: None,
            completed_at: None,
            action_started_at: None,
            ack_payload: None,
            failed_claim_attempts: 0,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::seconds(900),
        }
    }

    #[test]
    fn bson_roundtrip_pending() {
        let p = make_pairing();
        let doc = bson::to_document(&p).expect("serialize");
        let restored: CliPairing = bson::from_document(doc).expect("deserialize");
        assert_eq!(p.id, restored.id);
        assert_eq!(p.user_id, restored.user_id);
        assert_eq!(p.kind, restored.kind);
        assert_eq!(p.status, CliPairingStatus::Pending);
    }

    #[test]
    fn bson_roundtrip_completed() {
        let mut p = make_pairing();
        p.status = CliPairingStatus::Completed;
        p.claimed_at = Some(Utc::now());
        p.claimed_from_ip = Some("127.0.0.1".to_string());
        p.completed_at = Some(Utc::now());
        p.ack_payload = Some(serde_json::json!({
            "acknowledged": true,
            "api_key_id": "abc-123"
        }));
        let doc = bson::to_document(&p).expect("serialize");
        let restored: CliPairing = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.status, CliPairingStatus::Completed);
        assert!(restored.claimed_at.is_some());
        assert_eq!(restored.claimed_from_ip.as_deref(), Some("127.0.0.1"));
        assert!(restored.completed_at.is_some());
        assert_eq!(
            restored
                .ack_payload
                .as_ref()
                .and_then(|v| v.get("api_key_id"))
                .and_then(|v| v.as_str()),
            Some("abc-123")
        );
    }

    #[test]
    fn status_serializes_snake_case() {
        let s = serde_json::to_string(&CliPairingStatus::Pending).unwrap();
        assert_eq!(s, "\"pending\"");
        let s = serde_json::to_string(&CliPairingStatus::Claimed).unwrap();
        assert_eq!(s, "\"claimed\"");
        let s = serde_json::to_string(&CliPairingStatus::Completed).unwrap();
        assert_eq!(s, "\"completed\"");
        let s = serde_json::to_string(&CliPairingStatus::Cancelled).unwrap();
        assert_eq!(s, "\"cancelled\"");
    }
}
