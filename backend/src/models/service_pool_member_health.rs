use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_pool_member_health";
pub const OBSERVATION_COLLECTION_NAME: &str = "service_pool_health_observations";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServicePoolHealthObservation {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub user_service_id: String,
    pub pool_config_revision: i64,
    pub pool_reset_generation: i64,
    pub member_reset_generation: i64,
    pub observation_sequence: i64,
    pub health_instance_id: String,
    #[serde(default)]
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<i32>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

/// Passive health state for one effective member route. Credential and
/// destination fingerprints are part of the scope so a rotated key or changed
/// endpoint cannot inherit a previous tenant's cooldown.
#[derive(Clone, Serialize, Deserialize)]
pub struct ServicePoolMemberHealth {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub user_service_id: String,
    pub owner_id: String,
    /// Configuration revision captured by this immutable health scope.
    /// Legacy rows default to zero and are never matched by a newer pool
    /// configuration because the planner always supplies the live revision.
    #[serde(default)]
    pub pool_config_revision: i64,
    pub credential_identity: String,
    #[serde(default)]
    pub credential_epoch: i64,
    pub destination_fingerprint: String,
    pub config_fingerprint: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub cooldown_until: Option<DateTime<Utc>>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_failure_class: Option<String>,
    #[serde(default)]
    pub last_status: Option<i32>,
    #[serde(default)]
    pub generation: i64,
    #[serde(default)]
    pub observation_id: Option<String>,
    #[serde(default)]
    pub observation_sequence: i64,
    #[serde(default)]
    pub observation_outcome: Option<String>,
    #[serde(default)]
    pub pool_reset_generation: i64,
    #[serde(default)]
    pub member_reset_generation: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_failure_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for ServicePoolMemberHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServicePoolMemberHealth")
            .field("id", &self.id)
            .field("pool_id", &self.pool_id)
            .field("user_service_id", &self.user_service_id)
            .field("owner_id", &self.owner_id)
            .field("pool_config_revision", &self.pool_config_revision)
            .field("credential_identity", &"[REDACTED]")
            .field("credential_epoch", &self.credential_epoch)
            .field("destination_fingerprint", &"[REDACTED]")
            .field("config_fingerprint", &"[REDACTED]")
            .field("model", &self.model)
            .field("consecutive_failures", &self.consecutive_failures)
            .field("cooldown_until", &self.cooldown_until)
            .field("expires_at", &self.expires_at)
            .field("last_failure_class", &self.last_failure_class)
            .field("last_status", &self.last_status)
            .field("generation", &self.generation)
            .field("pool_reset_generation", &self.pool_reset_generation)
            .field("member_reset_generation", &self.member_reset_generation)
            .field("last_failure_at", &self.last_failure_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_roundtrip_preserves_configuration_revision() {
        let now = DateTime::<Utc>::from_timestamp_millis(Utc::now().timestamp_millis())
            .expect("current time is representable");
        let health = ServicePoolMemberHealth {
            id: uuid::Uuid::new_v4().to_string(),
            pool_id: "pool".into(),
            user_service_id: "service".into(),
            owner_id: "owner".into(),
            pool_config_revision: 17,
            credential_identity: "credential-fingerprint".into(),
            credential_epoch: 2,
            destination_fingerprint: "destination-fingerprint".into(),
            config_fingerprint: "config-fingerprint".into(),
            model: Some("model".into()),
            consecutive_failures: 1,
            cooldown_until: Some(now),
            expires_at: Some(now),
            last_failure_class: Some("ambiguous".into()),
            last_status: Some(503),
            generation: 4,
            observation_id: Some("observation".into()),
            observation_sequence: 9,
            observation_outcome: Some("failure".into()),
            pool_reset_generation: 1,
            member_reset_generation: 2,
            last_failure_at: now,
            updated_at: now,
        };
        let restored: ServicePoolMemberHealth =
            bson::from_document(bson::to_document(&health).expect("serialize")).expect("decode");
        assert_eq!(restored.pool_config_revision, 17);
        assert_eq!(restored.id, health.id);
        assert_eq!(restored.last_failure_at, health.last_failure_at);
    }

    #[test]
    fn missing_configuration_revision_defaults_for_legacy_rows() {
        let mut document = bson::to_document(&ServicePoolMemberHealth {
            id: "id".into(),
            pool_id: "pool".into(),
            user_service_id: "service".into(),
            owner_id: "owner".into(),
            pool_config_revision: 4,
            credential_identity: "credential".into(),
            credential_epoch: 1,
            destination_fingerprint: "destination".into(),
            config_fingerprint: "config".into(),
            model: None,
            consecutive_failures: 0,
            cooldown_until: None,
            expires_at: None,
            last_failure_class: None,
            last_status: None,
            generation: 0,
            observation_id: None,
            observation_sequence: 0,
            observation_outcome: None,
            pool_reset_generation: 0,
            member_reset_generation: 0,
            last_failure_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .expect("serialize");
        document.remove("pool_config_revision");
        let restored: ServicePoolMemberHealth = bson::from_document(document).expect("decode");
        assert_eq!(restored.pool_config_revision, 0);
    }
}
