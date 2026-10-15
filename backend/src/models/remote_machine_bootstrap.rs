use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "managed_machine_bootstrap_deliveries";

/// A replayable delivery contains only recipient-sealed ciphertext. Node
/// credentials are never recoverable from this row by the server.
#[derive(Clone, Deserialize, Serialize)]
pub struct ManagedBootstrapDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub binding: nyxid_machine::managed::Binding,
    pub recipient_key_digest: String,
    pub sealed_digest: String,
    pub sealed: nyxid_machine::bootstrap::Envelope,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for ManagedBootstrapDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedBootstrapDelivery")
            .field("delivery", &"[REDACTED]")
            .field("sealed", &"[REDACTED]")
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}
