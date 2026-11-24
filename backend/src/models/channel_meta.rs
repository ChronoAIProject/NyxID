use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SESSIONS: &str = "channel_meta_oauth_sessions";

/// Minimal context for independently signed deletion after account retirement.
/// No account, asset, subject, credential or participant identifier is copied.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct RetiredGrant {
    pub grant_id: String,
    pub identity: String,
    pub platform: String,
    pub account_erasure_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub bot_created_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub authorized_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthSession {
    #[serde(rename = "_id")]
    pub id: String,
    pub state_hash: String,
    pub actor_id: String,
    pub owner_id: String,
    pub platform: String,
    pub app_id: String,
    pub redirect_uri: String,
    pub label: String,
    pub status: String,
    pub bot_id: Option<String>,
    pub reconnect_bot_id: Option<String>,
    pub expected_generation: Option<i64>,
    #[serde(default)]
    pub connect_link_id: Option<String>,
    #[serde(default)]
    pub grant_subject_id: Option<String>,
    pub selected_asset_id: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub assets_encrypted: Option<Vec<u8>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for OAuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MetaOAuthSession([REDACTED])")
    }
}

#[derive(Serialize, Deserialize)]
pub struct AssetCredential {
    pub id: String,
    pub name: String,
    pub token: zeroize::Zeroizing<String>,
    pub grant_subject_id: String,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}

pub const GRANT_SCOPES: &str = "channel_meta_grant_scopes";

/// Provenance on an original record; no provider address or credential is copied.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantRef {
    pub bot_id: String,
    pub grant_id: String,
    pub generation: i64,
}
impl std::fmt::Debug for GrantRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaGrantRef").finish_non_exhaustive()
    }
}

/// One installation grant's immutable identity and generation interval.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct GrantScope {
    #[serde(rename = "_id")]
    pub id: String,
    pub bot_id: String,
    pub grant_id: String,
    pub identity: Option<String>,
    pub platform: String,
    pub app_id: String,
    pub from_generation: i64,
    pub to_generation: Option<i64>,
    pub legacy_history: bool,
    pub binding_fingerprint: String,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub authorized_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub bot_created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub sealed_at: Option<DateTime<Utc>>,
    pub status: String,
    pub outstanding: bool,
    #[serde(default)]
    pub erasure_requested: bool,
}
