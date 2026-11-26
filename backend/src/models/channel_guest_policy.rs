use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_guest_policies";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestDataPolicy {
    #[serde(rename = "_id")]
    pub id: String,
    pub bot_id: String,
    pub revision: i64,
    pub rules: Vec<GuestDataRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestDataRule {
    pub source_id: String,
    pub collection_id: String,
    pub audience: GuestAudience,
    pub scope: GuestDataScope,
    pub delivery: GuestDataDelivery,
    pub operations: BTreeMap<GuestDataOperation, GuestFields>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuestAudience {
    AllGuests,
    Guest {
        guest_id: String,
    },
    VerifiedAudience {
        audience_id: String,
        guest_ids: BTreeSet<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room_id: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuestDataScope {
    OwnRecords,
    Resources { record_ids: BTreeSet<String> },
    Collection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestDataDelivery {
    Private,
    Shared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestDataOperation {
    Create,
    Read,
    Update,
    Delete,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestFields {
    #[serde(default)]
    pub read: BTreeSet<String>,
    #[serde(default)]
    pub write: BTreeSet<String>,
}
