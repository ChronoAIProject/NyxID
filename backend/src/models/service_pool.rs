use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "service_pools";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PoolStrategy {
    RoundRobin,
    Weighted,
}

impl PoolStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::Weighted => "weighted",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "round_robin" => Some(Self::RoundRobin),
            "weighted" => Some(Self::Weighted),
            _ => None,
        }
    }
}

fn default_member_weight() -> u32 {
    1
}

fn default_member_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServicePoolMember {
    pub user_service_id: String,
    #[serde(default = "default_member_weight")]
    pub weight: u32,
    #[serde(default = "default_member_enabled")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServicePool {
    #[serde(rename = "_id")]
    pub id: String,
    /// Polymorphic owner: a person user or an org user (`user_type=Org`).
    /// Use `org_service::resolve_owner_access` for ACL checks.
    pub user_id: String,
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub strategy: PoolStrategy,
    #[serde(default)]
    pub members: Vec<ServicePoolMember>,
    #[serde(default)]
    pub rr_counter: i64,
    pub is_active: bool,
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
        assert_eq!(COLLECTION_NAME, "service_pools");
    }

    #[test]
    fn strategy_parse_and_as_str() {
        assert_eq!(PoolStrategy::RoundRobin.as_str(), "round_robin");
        assert_eq!(PoolStrategy::Weighted.as_str(), "weighted");
        assert_eq!(
            PoolStrategy::parse("round_robin"),
            Some(PoolStrategy::RoundRobin)
        );
        assert_eq!(
            PoolStrategy::parse("weighted"),
            Some(PoolStrategy::Weighted)
        );
        assert_eq!(PoolStrategy::parse("least_in_flight"), None);
    }

    #[test]
    fn bson_roundtrip() {
        let pool = ServicePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            slug: "llm-pool".to_string(),
            name: "LLM Pool".to_string(),
            description: Some("Two interchangeable services".to_string()),
            strategy: PoolStrategy::Weighted,
            members: vec![
                ServicePoolMember {
                    user_service_id: uuid::Uuid::new_v4().to_string(),
                    weight: 2,
                    enabled: true,
                },
                ServicePoolMember {
                    user_service_id: uuid::Uuid::new_v4().to_string(),
                    weight: 1,
                    enabled: false,
                },
            ],
            rr_counter: 7,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let doc = bson::to_document(&pool).expect("serialize");
        assert_eq!(doc.get_str("_id").expect("_id"), pool.id);
        assert!(doc.contains_key("created_at"));
        assert!(doc.contains_key("updated_at"));

        let restored: ServicePool = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.id, pool.id);
        assert_eq!(restored.user_id, pool.user_id);
        assert_eq!(restored.slug, pool.slug);
        assert_eq!(restored.strategy, PoolStrategy::Weighted);
        assert_eq!(restored.members, pool.members);
        assert_eq!(restored.rr_counter, 7);
        assert!(restored.is_active);
    }

    #[test]
    fn member_defaults_deserialize() {
        let doc = bson::doc! {
            "_id": "pool-1",
            "user_id": "owner-1",
            "slug": "svc-pool",
            "name": "Service Pool",
            "strategy": "round_robin",
            "members": [{ "user_service_id": "svc-1" }],
            "is_active": true,
            "created_at": bson::DateTime::from_chrono(Utc::now()),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };

        let restored: ServicePool = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.rr_counter, 0);
        assert_eq!(restored.members[0].weight, 1);
        assert!(restored.members[0].enabled);
    }
}
