//! Readiness for security rollout, separate from flag precedence and node OS
//! support. The roster is an operator assertion about the serving deployment;
//! observations cannot discover an old, unregistered replica behind a balancer.
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Serialize;
use std::{collections::BTreeSet, sync::OnceLock};

#[derive(Clone, Debug)]
pub struct Runtime {
    pub identity: ReplicaIdentity,
    pub config: RolloutConfig,
    pub platform_policy_required: bool,
}
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn initialize(runtime: Runtime) {
    RUNTIME
        .set(runtime)
        .expect("security rollout runtime initialized twice");
}
fn runtime() -> AppResult<&'static Runtime> {
    RUNTIME.get().ok_or(AppError::SecurityReadinessUnavailable)
}
const ROLLOUTS: &str = "security_rollouts";
const PLATFORM_CUTOVER: &str = "platform-policy-v1";

use super::node_owner_service::ReplicaIdentity;
use crate::{
    errors::{AppError, AppResult},
    models::security_replica::{COLLECTION_NAME, SecurityReplica},
};

pub const PLATFORM_POLICY: &str = "platform:operation-policy";
pub const CAPABILITIES: &[&str] = &[
    "assistant:org-agents",
    "assistant:operation-scopes",
    "assistant:machine-capabilities",
    "assistant:machine-contexts",
    "assistant:live-turn-gate",
    PLATFORM_POLICY,
    "approval:required-web-v1",
];
const MAX_AGE_SECS: i64 = 45;

pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"observed_at": 1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("security_replica_observations".to_owned())
                        .expire_after(std::time::Duration::from_secs(7 * 24 * 3600))
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct RolloutConfig {
    pub expected_replicas: Vec<String>,
    pub drained: bool,
}

impl RolloutConfig {
    pub fn parse(roster: &str, drained: bool) -> Result<Self, String> {
        let mut names = BTreeSet::new();
        if !roster.trim().is_empty() {
            for name in roster.split(',').map(str::trim) {
                if name.is_empty()
                    || name.len() > 128
                    || !name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                    || !names.insert(name.to_owned())
                {
                    return Err(
                        "SECURITY_ROLLOUT_REPLICAS must contain unique, nonempty instance names"
                            .into(),
                    );
                }
            }
        }
        if names.len() > 128 {
            return Err("Security rollout roster exceeds 128 replicas".into());
        }
        Ok(Self {
            expected_replicas: names.into_iter().collect(),
            drained,
        })
    }
}

pub async fn observe(
    db: &Database,
    identity: &ReplicaIdentity,
    config: &RolloutConfig,
    platform_policy_required: bool,
) -> AppResult<()> {
    if platform_policy_required {
        db.collection::<bson::Document>(ROLLOUTS)
            .update_one(
                doc! {"_id": PLATFORM_CUTOVER},
                doc! {"$setOnInsert": {"required": true, "created_at": bson::DateTime::now()}},
            )
            .upsert(true)
            .await?;
    }
    let row = SecurityReplica {
        id: identity.generation_id.clone(),
        instance_name: identity.instance_name.clone(),
        generation_id: identity.generation_id.clone(),
        protocol: 1,
        capabilities: CAPABILITIES.iter().map(|s| (*s).to_owned()).collect(),
        expected_replicas: config.expected_replicas.clone(),
        drained: config.drained,
        platform_policy_required,
        observed_at: Utc::now(),
    };
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .replace_one(doc! {"_id": &row.id}, &row)
        .upsert(true)
        .await?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Readiness {
    pub ready: bool,
    pub blockers: Vec<String>,
    pub expected_replicas: Vec<String>,
    pub live_generations: usize,
    pub platform_policy_required: bool,
    pub catalog_missing_policy: u64,
    pub catalog_explicit_empty_policy: u64,
}

fn assess(
    rows: &[SecurityReplica],
    local: &Runtime,
    capability: &str,
    cutover_required: bool,
    now: DateTime<Utc>,
) -> Readiness {
    let live: Vec<_> = rows
        .iter()
        .filter(|r| r.observed_at >= now - Duration::seconds(MAX_AGE_SECS))
        .collect();
    let expected: BTreeSet<_> = local.config.expected_replicas.iter().cloned().collect();
    let mut blockers = BTreeSet::new();
    if !live.iter().any(|row| {
        row.instance_name == local.identity.instance_name
            && row.generation_id == local.identity.generation_id
    }) {
        blockers.insert("executing_process_not_observed".to_owned());
    }
    if !local.config.drained {
        blockers.insert("old_replicas_not_declared_drained".to_owned());
    }
    if expected.is_empty() {
        blockers.insert("serving_roster_not_declared".to_owned());
    }
    for name in &expected {
        if live.iter().filter(|r| &r.instance_name == name).count() != 1 {
            blockers.insert(format!("missing_or_duplicate_replica:{name}"));
        }
    }
    for row in &live {
        if row.observed_at > now + Duration::seconds(5) {
            blockers.insert(format!("future_observation:{}", row.instance_name));
        }
        if !expected.contains(&row.instance_name) {
            blockers.insert(format!("undeclared_replica:{}", row.instance_name));
        }
        if row
            .expected_replicas
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != expected
        {
            blockers.insert("inconsistent_serving_roster".to_owned());
        }
        if !row.drained {
            blockers.insert("old_replicas_not_declared_drained".to_owned());
        }
        if row.protocol != 1 || !row.capabilities.iter().any(|c| c == capability) {
            blockers.insert(format!("unsupported_replica:{}", row.instance_name));
        }
    }
    let platform_policy_required = cutover_required
        || local.platform_policy_required
        || live.iter().any(|r| r.platform_policy_required);
    if capability == PLATFORM_POLICY {
        if live
            .iter()
            .any(|r| r.platform_policy_required != platform_policy_required)
        {
            blockers.insert("platform_policy_configuration_mismatch".to_owned());
        }
        if !local.platform_policy_required {
            blockers.insert("executing_process_platform_policy_disabled".to_owned());
        }
        if !platform_policy_required {
            blockers.insert("platform_policy_not_enabled".to_owned());
        }
    }
    Readiness {
        ready: blockers.is_empty(),
        blockers: blockers.into_iter().collect(),
        expected_replicas: expected.into_iter().collect(),
        live_generations: live.len(),
        platform_policy_required,
        catalog_missing_policy: 0,
        catalog_explicit_empty_policy: 0,
    }
}

async fn rows(db: &Database) -> AppResult<Vec<SecurityReplica>> {
    // Old observations remain diagnostic only; bounded reads fail closed if a
    // malicious/misconfigured fleet exceeds the supported maximum.
    let rows: Vec<_> = db.collection::<SecurityReplica>(COLLECTION_NAME)
        .find(doc! {"observed_at": {"$gte": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(MAX_AGE_SECS))}})
        .limit(257).await?.try_collect().await?;
    if rows.len() > 256 {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    Ok(rows)
}

pub async fn report(db: &Database, capability: &str) -> AppResult<Readiness> {
    report_for(db, runtime()?, capability).await
}

async fn platform_cutover_required(db: &Database) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(ROLLOUTS)
        .find_one(doc! {"_id": PLATFORM_CUTOVER})
        .await?
        .is_some())
}

async fn report_for(db: &Database, local: &Runtime, capability: &str) -> AppResult<Readiness> {
    let mut report = assess(
        &rows(db).await?,
        local,
        capability,
        platform_cutover_required(db).await?,
        Utc::now(),
    );
    if capability == PLATFORM_POLICY {
        // Include every active catalog row which can supply a master credential.
        // A conservative superset is intentional; never fetch credential bytes.
        let base = doc! {"is_active": true, "auth_method": {"$ne": "none"}, "$or": [{"platform_key": {"$ne": null}}, {"requires_user_credential": false}]};
        let collection =
            db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME);
        let mut missing = base.clone();
        missing.insert("proxy_operation_policy", bson::Bson::Null);
        report.catalog_missing_policy = collection.count_documents(missing).await?;
        let mut empty = base;
        empty.insert("proxy_operation_policy.rules", doc! {"$size": 0});
        report.catalog_explicit_empty_policy = collection.count_documents(empty).await?;
        if report.catalog_missing_policy != 0 {
            report.blockers.push("catalog_master_policy_missing".into());
            report.ready = false;
        }
    }
    Ok(report)
}

pub async fn require(db: &Database, capability: &str) -> AppResult<()> {
    if CAPABILITIES.contains(&capability) && !report(db, capability).await?.ready {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    Ok(())
}

/// A replica with the legacy env value must not bypass a sibling's enabled
/// policy during a mixed deployment. A drained coordinated cutover is required.
pub async fn require_platform(db: &Database, local_required: bool) -> AppResult<()> {
    let rows = rows(db).await?;
    if local_required
        || platform_cutover_required(db).await?
        || rows.iter().any(|r| r.platform_policy_required)
    {
        require(db, PLATFORM_POLICY).await?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn ready_test_replica(db: &Database) {
    let local = Runtime {
        identity: ReplicaIdentity {
            instance_name: "test-backend".into(),
            generation_id: "test-process".into(),
            internal_base_url: "http://127.0.0.1:3002".into(),
        },
        config: RolloutConfig {
            expected_replicas: vec!["test-backend".into()],
            drained: true,
        },
        platform_policy_required: false,
    };
    RUNTIME.get_or_init(|| local);
    let db = db.clone();
    let heartbeat = || async {
        observe(
            &db,
            &ReplicaIdentity {
                instance_name: "test-backend".into(),
                generation_id: "test-process".into(),
                internal_base_url: "http://127.0.0.1:3002".into(),
            },
            &RolloutConfig {
                expected_replicas: vec!["test-backend".into()],
                drained: true,
            },
            false,
        )
        .await
        .expect("test replica observation");
    };
    heartbeat().await;
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            observe(
                &db,
                &ReplicaIdentity {
                    instance_name: "test-backend".into(),
                    generation_id: "test-process".into(),
                    internal_base_url: "http://127.0.0.1:3002".into(),
                },
                &RolloutConfig {
                    expected_replicas: vec!["test-backend".into()],
                    drained: true,
                },
                false,
            )
            .await
            .expect("test replica heartbeat");
        }
    });
}

#[cfg(test)]
#[path = "security_readiness_tests.rs"]
mod tests;
