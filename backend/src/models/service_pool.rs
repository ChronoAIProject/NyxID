use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "service_pools";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PoolStrategy {
    #[default]
    RoundRobin,
    Weighted,
    /// Priority tiers with balancing within each tier and optional failover.
    /// Older pools omit this value and retain their single-attempt behaviour.
    Priority,
}

impl PoolStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::Weighted => "weighted",
            Self::Priority => "priority",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "round_robin" => Some(Self::RoundRobin),
            "weighted" => Some(Self::Weighted),
            "priority" => Some(Self::Priority),
            _ => None,
        }
    }
}

/// Balancing within a priority tier; priority itself never implies weights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TierBalance {
    #[default]
    RoundRobin,
    Weighted,
}

fn default_member_weight() -> u32 {
    1
}

fn default_member_enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePoolMember {
    pub user_service_id: String,
    #[serde(default = "default_member_weight")]
    pub weight: u32,
    #[serde(default = "default_member_enabled")]
    pub enabled: bool,
    /// Lower values are preferred. Members in the same tier are balanced by
    /// the pool tier_balance setting.
    #[serde(default)]
    pub priority: u32,
    /// Optional provider model rewrite used by the AI chat contract.
    #[serde(default)]
    pub model: Option<String>,
    /// Owner declaration of wire/API compatibility for a custom or different catalog API.
    #[serde(default)]
    pub same_api_compatible: bool,
    /// Durable member-scoped health reset fence. It is independent of the
    /// routing counter and survives a health row's TTL expiry.
    #[serde(default)]
    pub health_reset_generation: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PoolMemberContract {
    /// Members accept the same wire contract as the incoming request.
    #[default]
    SameApi,
    /// Members are adapted through the stable OpenAI chat contract.
    AiChat,
}

impl PoolMemberContract {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameApi => "same_api",
            Self::AiChat => "ai_chat",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetryTrigger {
    ConnectError,
    NodeOffline,
    TransportError,
    Timeout,
    #[serde(rename = "http_408")]
    Http408,
    #[serde(rename = "http_429")]
    Http429,
    #[serde(rename = "http_500")]
    Http500,
    #[serde(rename = "http_502")]
    Http502,
    #[serde(rename = "http_503")]
    Http503,
    #[serde(rename = "http_504")]
    Http504,
    #[serde(rename = "http_529")]
    Http529,
    #[serde(rename = "http_401")]
    Http401,
    #[serde(rename = "http_403")]
    Http403,
}

impl RetryTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConnectError => "connect_error",
            Self::NodeOffline => "node_offline",
            Self::TransportError => "transport_error",
            Self::Timeout => "timeout",
            Self::Http408 => "http_408",
            Self::Http429 => "http_429",
            Self::Http500 => "http_500",
            Self::Http502 => "http_502",
            Self::Http503 => "http_503",
            Self::Http504 => "http_504",
            Self::Http529 => "http_529",
            Self::Http401 => "http_401",
            Self::Http403 => "http_403",
        }
    }
}

fn default_max_attempts() -> u8 {
    3
}

fn default_per_attempt_timeout_ms() -> u32 {
    60_000
}

fn default_overall_deadline_ms() -> u32 {
    120_000
}

fn default_retry_on() -> Vec<RetryTrigger> {
    vec![
        RetryTrigger::ConnectError,
        RetryTrigger::NodeOffline,
        RetryTrigger::Timeout,
        RetryTrigger::Http429,
        RetryTrigger::Http502,
        RetryTrigger::Http503,
        RetryTrigger::Http504,
        RetryTrigger::Http529,
    ]
}

fn default_cooldown_base_ms() -> u32 {
    5_000
}

fn default_cooldown_max_ms() -> u32 {
    300_000
}

fn default_failures_to_open() -> u32 {
    1
}

fn default_honor_retry_after() -> bool {
    true
}

fn default_max_replay_body_bytes() -> u32 {
    8 * 1024 * 1024
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CooldownPolicy {
    #[serde(default = "default_cooldown_base_ms")]
    pub base_ms: u32,
    #[serde(default = "default_cooldown_max_ms")]
    pub max_ms: u32,
    #[serde(default = "default_honor_retry_after")]
    pub honor_retry_after: bool,
    #[serde(default = "default_failures_to_open")]
    pub failures_to_open: u32,
}

impl Default for CooldownPolicy {
    fn default() -> Self {
        Self {
            base_ms: default_cooldown_base_ms(),
            max_ms: default_cooldown_max_ms(),
            honor_retry_after: default_honor_retry_after(),
            failures_to_open: default_failures_to_open(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FailoverPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u8,
    #[serde(default = "default_per_attempt_timeout_ms")]
    pub per_attempt_timeout_ms: u32,
    #[serde(default = "default_overall_deadline_ms")]
    pub overall_deadline_ms: u32,
    #[serde(default = "default_retry_on")]
    pub retry_on: Vec<RetryTrigger>,
    #[serde(default)]
    pub retry_ambiguous_dispatch: bool,
    #[serde(default)]
    pub cooldown: CooldownPolicy,
    #[serde(default = "default_max_replay_body_bytes")]
    pub max_replay_body_bytes: u32,
}

impl Default for FailoverPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            per_attempt_timeout_ms: default_per_attempt_timeout_ms(),
            overall_deadline_ms: default_overall_deadline_ms(),
            retry_on: default_retry_on(),
            retry_ambiguous_dispatch: false,
            cooldown: CooldownPolicy::default(),
            max_replay_body_bytes: default_max_replay_body_bytes(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServicePool {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub strategy: PoolStrategy,
    #[serde(default)]
    pub tier_balance: TierBalance,
    #[serde(default)]
    pub member_contract: PoolMemberContract,
    #[serde(default)]
    pub failover: Option<FailoverPolicy>,
    #[serde(default)]
    pub members: Vec<ServicePoolMember>,
    #[serde(default)]
    pub rr_counter: i64,
    /// One counter per configured priority tier; reset on configuration edits.
    #[serde(default)]
    pub tier_counters: std::collections::BTreeMap<String, i64>,
    /// Monotonic configuration revision. Routing counters do not change it;
    /// every membership or policy mutation does.
    #[serde(default)]
    pub config_revision: i64,
    /// Durable pool-wide health reset fence.
    #[serde(default)]
    pub health_reset_generation: i64,
    /// Monotonic ticket sequence used to order concurrent observations.
    #[serde(default)]
    pub health_observation_sequence: i64,
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
    fn strategy_strings_roundtrip() {
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
        assert_eq!(PoolStrategy::parse("least_inflight"), None);
    }

    #[test]
    fn bson_roundtrip() {
        let pool = ServicePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            slug: "llm-pool".to_string(),
            name: "LLM Pool".to_string(),
            description: Some("Two interchangeable endpoints".to_string()),
            strategy: PoolStrategy::Weighted,
            tier_balance: TierBalance::default(),
            member_contract: PoolMemberContract::SameApi,
            failover: None,
            members: vec![
                ServicePoolMember {
                    user_service_id: uuid::Uuid::new_v4().to_string(),
                    weight: 2,
                    enabled: true,
                    priority: 0,
                    model: None,
                    same_api_compatible: false,
                    health_reset_generation: 0,
                },
                ServicePoolMember {
                    user_service_id: uuid::Uuid::new_v4().to_string(),
                    weight: 1,
                    enabled: false,
                    priority: 1,
                    model: Some("backup-model".to_string()),
                    same_api_compatible: false,
                    health_reset_generation: 0,
                },
            ],
            rr_counter: 42,
            tier_counters: Default::default(),
            config_revision: 0,
            health_reset_generation: 0,
            health_observation_sequence: 0,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let doc = bson::to_document(&pool).expect("serialize");
        let restored: ServicePool = bson::from_document(doc).expect("deserialize");
        assert_eq!(pool.id, restored.id);
        assert_eq!(pool.user_id, restored.user_id);
        assert_eq!(pool.slug, restored.slug);
        assert_eq!(pool.strategy, restored.strategy);
        assert_eq!(pool.members, restored.members);
        assert_eq!(pool.rr_counter, restored.rr_counter);
        assert!(restored.is_active);
    }

    #[test]
    fn bson_serializes_none_description() {
        let pool = ServicePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            slug: "null-description-pool".to_string(),
            name: "Null Description Pool".to_string(),
            description: None,
            strategy: PoolStrategy::RoundRobin,
            tier_balance: TierBalance::default(),
            member_contract: PoolMemberContract::SameApi,
            failover: None,
            members: vec![],
            rr_counter: 0,
            tier_counters: Default::default(),
            config_revision: 0,
            health_reset_generation: 0,
            health_observation_sequence: 0,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let doc = bson::to_document(&pool).expect("serialize");
        assert_eq!(doc.get("description"), Some(&bson::Bson::Null));
    }

    #[test]
    fn bson_defaults() {
        let mut doc = bson::doc! {
            "_id": "pool-id",
            "user_id": "user-id",
            "slug": "default-pool",
            "name": "Default Pool",
            "members": [
                { "user_service_id": "svc-1" }
            ],
            "is_active": true,
            "created_at": bson::DateTime::from_chrono(Utc::now()),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };
        doc.remove("strategy");
        doc.remove("rr_counter");

        let restored: ServicePool = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.strategy, PoolStrategy::RoundRobin);
        assert_eq!(restored.rr_counter, 0);
        assert_eq!(restored.members[0].weight, 1);
        assert!(restored.members[0].enabled);
    }

    #[test]
    fn retry_trigger_wire_names_round_trip_with_status_underscores() {
        let triggers = [
            (RetryTrigger::ConnectError, "connect_error"),
            (RetryTrigger::NodeOffline, "node_offline"),
            (RetryTrigger::TransportError, "transport_error"),
            (RetryTrigger::Timeout, "timeout"),
            (RetryTrigger::Http408, "http_408"),
            (RetryTrigger::Http429, "http_429"),
            (RetryTrigger::Http500, "http_500"),
            (RetryTrigger::Http502, "http_502"),
            (RetryTrigger::Http503, "http_503"),
            (RetryTrigger::Http504, "http_504"),
            (RetryTrigger::Http529, "http_529"),
            (RetryTrigger::Http401, "http_401"),
            (RetryTrigger::Http403, "http_403"),
        ];
        for (trigger, wire_name) in triggers {
            assert_eq!(trigger.as_str(), wire_name);
            let json = serde_json::to_string(&trigger).expect("serialize trigger");
            assert_eq!(json, format!("\"{wire_name}\""));
            assert_eq!(
                serde_json::from_str::<RetryTrigger>(&json).expect("deserialize trigger"),
                trigger
            );
            let bson = bson::to_bson(&trigger).expect("serialize BSON trigger");
            assert_eq!(bson, bson::Bson::String(wire_name.to_string()));
            assert_eq!(
                bson::from_bson::<RetryTrigger>(bson).expect("deserialize BSON trigger"),
                trigger
            );
        }
    }
}
