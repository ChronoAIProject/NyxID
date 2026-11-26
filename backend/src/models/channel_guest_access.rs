use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestAssurance {
    ProviderVerified,
    Unverified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerAudience {
    Private,
    Shared,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestActor {
    pub id: String,
    pub assurance: GuestAssurance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestTurn {
    pub bot_id: String,
    pub binding_id: String,
    pub platform: String,
    pub conversation_id: String,
    pub source_event_id: String,
    pub actor: GuestActor,
    pub audience: AnswerAudience,
    pub policy_revision: i64,
    pub activation_epoch: i64,
    #[serde(default)]
    pub bot_revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_grant_id: Option<String>,
}
