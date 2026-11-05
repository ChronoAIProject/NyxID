//! Additive organization-specialist channel identities. No credential material.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::credits::Credits;

pub const LINKS: &str = "org_channel_sender_links";
pub const CHALLENGES: &str = "org_channel_link_challenges";
pub const LANES: &str = "org_channel_lanes";
pub const MESSAGES: &str = "org_channel_messages";
pub const BUDGETS: &str = "org_channel_guest_budgets";
pub const HOLDS: &str = "org_channel_guest_holds";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub version: u8,
    pub org_id: String,
    pub revision: String,
    #[serde(default)]
    pub guests: GuestPolicy,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuestPolicy {
    pub revision: String,
    pub service_authority: std::collections::BTreeMap<String, String>,
    pub enabled: bool,
    pub service_ids: Vec<String>,
    /// Absent until an admin explicitly sets a positive UTC daily cap.
    #[serde(with = "super::credits::whole::optional")]
    pub daily_credits: Option<Credits>,
}

/// Server-authored source/actor binding, revalidated at admission and delivery.
/// `guest` identities execute as the org, never as the linking administrator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    pub version: u8,
    pub org_id: String,
    pub revision: String,
    pub agent_id: String,
    pub actor_id: String,
    pub guest: bool,
    #[serde(default)]
    pub guest_revision: Option<String>,
    pub sender_hash: String,
    pub link_id: Option<String>,
    pub link_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SenderLink {
    #[serde(rename = "_id")]
    pub id: String,
    pub channel_id: String,
    pub org_id: String,
    pub revision: String,
    pub binding_revision: String,
    pub sender_hash: String,
    pub user_id: String,
    pub active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Metadata-only billing attribution; never an authority token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GuestUsage {
    pub channel_id: String,
    pub sender_hash: String,
}
