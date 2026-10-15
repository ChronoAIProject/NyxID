use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_public_turns";

/// Disposable public transcript, encrypted independently of owner history.
/// Authority and cleanup remain in non-TTL session/ingress records.
#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformPublicTurn {
    #[serde(rename = "_id")]
    pub id: String,
    pub public_session_id: String,
    pub conversation_id: String,
    pub receipt_id: String,
    pub claim_id: String,
    pub sponsor_owner_user_id: String,
    pub policy_digest: String,
    pub credential_api_key_id: String,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub input_ciphertext: Vec<u8>,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub reply_ciphertext: Vec<u8>,
    pub reply_id: Option<String>,
    pub outcome: Option<String>,
    pub max_output_bytes: u32,
    /// Server-chosen outer NoAuth transport, frozen during preparation.
    #[serde(default)]
    pub nyxagent_service_id: String,
    #[serde(default)]
    pub nyxagent_catalog_digest: String,
    /// Spent before outbound; a lost response never authorizes a replay.
    #[serde(default)]
    pub outer_dispatch_admitted: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for PlatformPublicTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlatformPublicTurn { [REDACTED] }")
    }
}
