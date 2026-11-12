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
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

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
use crate::models::security_capability_activation::{
    COLLECTION_NAME as ROLLOUTS, RECORD_KIND, SecurityCapabilityActivation,
};
const PLATFORM_CUTOVER: &str = "platform-policy-v1";

use super::node_owner_service::ReplicaIdentity;
use crate::{
    errors::{AppError, AppResult},
    models::security_replica::{COLLECTION_NAME, SecurityReplica},
};

pub const PLATFORM_POLICY: &str = "platform:operation-policy";
pub const CAPABILITIES: &[&str] = &[
    super::assistant_platform_model::READINESS,
    super::feature_flag_service::ASSISTANT_CHANNEL_GUEST_MEDIA_FLAG_KEY,
    super::platform_model_service::REGISTRY_READINESS,
    super::platform_model_service::EXECUTION_READINESS,
    super::platform_model_execution::images::READINESS,
    super::platform_model_execution::certification::READINESS,
    super::billing::model_tariffs::READINESS,
    "assistant:generated-artifacts",
    "assistant:org-agents",
    super::org_channel_service::FLAG,
    super::org_channel_service::GUEST_GRANTS_READINESS,
    "assistant:shared-agents",
    super::collaboration_group_service::READINESS,
    super::shared_agent_transport::READINESS,
    "assistant:operation-scopes",
    "assistant:guest-operation-scopes",
    "assistant:machine-capabilities",
    "assistant:machine-contexts",
    "assistant:live-turn-gate",
    PLATFORM_POLICY,
    super::feature_flag_service::ORG_INSTRUCTION_FLAG_KEY,
    super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
    "permissions:execution-dispatch-v1",
    "permissions:execution-authoring-v1",
    super::execution_permission_enrollment::READINESS,
    "permissions:execution-authoring-v2",
    super::execution_permission_dispatch::machine::READINESS,
    super::execution_permission_dispatch::proposals::v3::READINESS,
    super::execution_permission_channel::READINESS,
    super::feature_flag_service::HOSTED_CONNECT_INTENT_FLAG_KEY,
    "approval:required-web-v1",
    // Older transport writers do not wait for relay sockets before remote setup.
    "channel:discord-gateway",
    super::nyxbot_gateway_source::READINESS,
    super::nyxbot_response_recovery::RELAY_READINESS,
    super::channel_media_authority::READINESS,
];
pub const AUTOMATION_COMPLETION: &str = "automation:coordinator-completion-v2";
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
    db.collection::<SecurityCapabilityActivation>(ROLLOUTS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"record_kind": 1, "capability": 1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("security_capability_activation_unique".to_owned())
                        .unique(true)
                        .partial_filter_expression(doc! {"record_kind": RECORD_KIND})
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

#[cfg(test)]
pub async fn observe(
    db: &Database,
    identity: &ReplicaIdentity,
    config: &RolloutConfig,
    platform_policy_required: bool,
) -> AppResult<()> {
    observe_with_automation(db, identity, config, platform_policy_required, false).await
}

/// Optional writer capability: disabled replicas retain the main heartbeat payload.
pub async fn observe_with_automation(
    db: &Database,
    identity: &ReplicaIdentity,
    config: &RolloutConfig,
    platform_policy_required: bool,
    automation_enabled: bool,
) -> AppResult<()> {
    let mut capabilities = CAPABILITIES.to_vec();
    if automation_enabled {
        capabilities.push(AUTOMATION_COMPLETION);
    }
    observe_with_capabilities(
        db,
        identity,
        config,
        platform_policy_required,
        &capabilities,
    )
    .await
}

async fn observe_with_capabilities(
    db: &Database,
    identity: &ReplicaIdentity,
    config: &RolloutConfig,
    platform_policy_required: bool,
    capabilities: &[&str],
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
        capabilities: capabilities.iter().map(|s| (*s).to_owned()).collect(),
        expected_replicas: config.expected_replicas.clone(),
        drained: config.drained,
        platform_policy_required,
        observed_at: Utc::now(),
    };
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .replace_one(doc! {"_id": &row.id}, &row)
        .upsert(true)
        .await?;
    // Activation evidence is written only by startup/heartbeat work, never by
    // a request. Assess the live fleet after publishing this generation.
    if let Err(error) = record_clean_activations(
        db,
        &Runtime {
            identity: identity.clone(),
            config: config.clone(),
            platform_policy_required,
        },
        capabilities,
    )
    .await
    {
        // Evidence is retried on the next heartbeat. Publishing this process's
        // observation must not depend on optional activation reads or writes.
        tracing::warn!(%error, "Security capability activation recording incomplete; next heartbeat will retry");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Readiness {
    /// Strict activation readiness; its existing API meaning is unchanged.
    pub ready: bool,
    pub serving_ready: bool,
    pub serving_with_roster_warnings: bool,
    pub roster_warnings: Vec<String>,
    pub serving_blockers: Vec<String>,
    pub activated: bool,
    pub active_since: Option<DateTime<Utc>>,
    pub blockers: Vec<String>,
    pub expected_replicas: Vec<String>,
    pub live_generations: usize,
    pub platform_policy_required: bool,
    pub catalog_missing_policy: u64,
    pub catalog_explicit_empty_policy: u64,
}

/// Classify at construction, never by parsing the public diagnostic strings.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Blocker {
    MissingOrDuplicateReplica(String),
    UndeclaredReplica(String),
    InconsistentServingRoster,
    ServingRosterNotDeclared,
    ExecutingProcessNotObserved,
    OldReplicasNotDeclaredDrained,
    FutureObservation(String),
    UnsupportedReplica(String),
    PlatformPolicyConfigurationMismatch,
    ExecutingProcessPlatformPolicyDisabled,
    PlatformPolicyNotEnabled,
    CatalogMasterPolicyMissing,
    CapabilityNotActivated,
}
impl Blocker {
    fn roster_warning(&self) -> bool {
        match self {
            Self::MissingOrDuplicateReplica(_)
            | Self::UndeclaredReplica(_)
            | Self::InconsistentServingRoster
            | Self::ServingRosterNotDeclared => true,
            Self::ExecutingProcessNotObserved
            | Self::OldReplicasNotDeclaredDrained
            | Self::FutureObservation(_)
            | Self::UnsupportedReplica(_)
            | Self::PlatformPolicyConfigurationMismatch
            | Self::ExecutingProcessPlatformPolicyDisabled
            | Self::PlatformPolicyNotEnabled
            | Self::CatalogMasterPolicyMissing
            | Self::CapabilityNotActivated => false,
        }
    }
    fn code(&self) -> String {
        match self {
            Self::MissingOrDuplicateReplica(name) => format!("missing_or_duplicate_replica:{name}"),
            Self::UndeclaredReplica(name) => format!("undeclared_replica:{name}"),
            Self::InconsistentServingRoster => "inconsistent_serving_roster".into(),
            Self::ServingRosterNotDeclared => "serving_roster_not_declared".into(),
            Self::ExecutingProcessNotObserved => "executing_process_not_observed".into(),
            Self::OldReplicasNotDeclaredDrained => "old_replicas_not_declared_drained".into(),
            Self::FutureObservation(name) => format!("future_observation:{name}"),
            Self::UnsupportedReplica(name) => format!("unsupported_replica:{name}"),
            Self::PlatformPolicyConfigurationMismatch => {
                "platform_policy_configuration_mismatch".into()
            }
            Self::ExecutingProcessPlatformPolicyDisabled => {
                "executing_process_platform_policy_disabled".into()
            }
            Self::PlatformPolicyNotEnabled => "platform_policy_not_enabled".into(),
            Self::CatalogMasterPolicyMissing => "catalog_master_policy_missing".into(),
            Self::CapabilityNotActivated => "capability_not_activated".into(),
        }
    }
}

impl Readiness {
    fn block(&mut self, blocker: Blocker) {
        let code = blocker.code();
        if blocker.roster_warning() {
            self.roster_warnings.push(code.clone());
        } else {
            self.serving_blockers.push(code.clone());
        }
        self.blockers.push(code);
        self.ready = false;
        self.serving_ready = false;
    }

    fn with_activation(mut self, active_since: Option<DateTime<Utc>>) -> Self {
        self.activated = active_since.is_some();
        self.active_since = active_since;
        self.serving_ready = self.ready || (self.activated && self.serving_blockers.is_empty());
        if !self.ready && !self.activated {
            self.serving_blockers
                .push(Blocker::CapabilityNotActivated.code());
        }
        self.serving_with_roster_warnings = self.serving_ready && !self.roster_warnings.is_empty();
        self.serving_blockers.sort();
        self.roster_warnings.sort();
        self
    }
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
        blockers.insert(Blocker::ExecutingProcessNotObserved);
    }
    if !local.config.drained {
        blockers.insert(Blocker::OldReplicasNotDeclaredDrained);
    }
    if expected.is_empty() {
        blockers.insert(Blocker::ServingRosterNotDeclared);
    }
    for name in &expected {
        if live.iter().filter(|r| &r.instance_name == name).count() != 1 {
            blockers.insert(Blocker::MissingOrDuplicateReplica(name.clone()));
        }
    }
    for row in &live {
        if row.observed_at > now + Duration::seconds(5) {
            blockers.insert(Blocker::FutureObservation(row.instance_name.clone()));
        }
        if !expected.contains(&row.instance_name) {
            blockers.insert(Blocker::UndeclaredReplica(row.instance_name.clone()));
        }
        if row
            .expected_replicas
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != expected
        {
            blockers.insert(Blocker::InconsistentServingRoster);
        }
        if !row.drained {
            blockers.insert(Blocker::OldReplicasNotDeclaredDrained);
        }
        if row.protocol != 1 || !row.capabilities.iter().any(|c| c == capability) {
            blockers.insert(Blocker::UnsupportedReplica(row.instance_name.clone()));
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
            blockers.insert(Blocker::PlatformPolicyConfigurationMismatch);
        }
        if !local.platform_policy_required {
            blockers.insert(Blocker::ExecutingProcessPlatformPolicyDisabled);
        }
        if !platform_policy_required {
            blockers.insert(Blocker::PlatformPolicyNotEnabled);
        }
    }
    let mut report = Readiness {
        ready: true,
        serving_ready: true,
        serving_with_roster_warnings: false,
        roster_warnings: vec![],
        serving_blockers: vec![],
        activated: false,
        active_since: None,
        blockers: vec![],
        expected_replicas: expected.into_iter().collect(),
        live_generations: live.len(),
        platform_policy_required,
        catalog_missing_policy: 0,
        catalog_explicit_empty_policy: 0,
    };
    for blocker in blockers {
        report.block(blocker);
    }
    // Preserve the existing lexicographic public blocker order.
    report.blockers.sort();
    report.roster_warnings.sort();
    report.serving_blockers.sort();
    report
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

/// The existing cutover read also fetches activation evidence. No extra
/// command or activation cache on the healthy request path.
async fn rollout_state(
    db: &Database,
    capabilities: &[&str],
) -> AppResult<(bool, BTreeMap<String, DateTime<Utc>>)> {
    let records: Vec<bson::Document> = db
        .collection(ROLLOUTS)
        .find(doc! {"$or": [
            {"_id": PLATFORM_CUTOVER},
            {"record_kind": RECORD_KIND, "capability": {"$in": capabilities}},
        ]})
        // At most one row per capability, one cutover marker, and one sentinel
        // to detect duplicate activations even without the unique index.
        .limit((capabilities.len() + 2) as i64)
        .await?
        .try_collect()
        .await?;
    let mut cutover = false;
    let mut activations = BTreeMap::new();
    for record in records {
        if record.get_str("_id").ok() == Some(PLATFORM_CUTOVER) {
            cutover = true;
        } else {
            let activation: SecurityCapabilityActivation =
                bson::from_document(record).map_err(|_| AppError::SecurityReadinessUnavailable)?;
            if activations
                .insert(activation.capability, activation.active_since)
                .is_some()
            {
                return Err(AppError::SecurityReadinessUnavailable);
            }
        }
    }
    Ok((cutover, activations))
}

async fn report_for(db: &Database, local: &Runtime, capability: &str) -> AppResult<Readiness> {
    let rows = rows(db).await?;
    let (cutover, activations) = rollout_state(db, &[capability]).await?;
    let mut report = assess(&rows, local, capability, cutover, Utc::now());
    inventory(db, capability, &mut report).await?;
    Ok(report.with_activation(activations.get(capability).copied()))
}

async fn inventory(db: &Database, capability: &str, report: &mut Readiness) -> AppResult<()> {
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
            report.block(Blocker::CatalogMasterPolicyMissing);
        }
    }
    Ok(())
}

async fn record_clean_activations(
    db: &Database,
    local: &Runtime,
    capabilities: &[&str],
) -> AppResult<()> {
    let rows = rows(db).await?;
    let state = match rollout_state(db, capabilities).await {
        Ok(state) => Some(state),
        // Isolate malformed/duplicate evidence to its own capability. A shared
        // database failure still aborts this best-effort pass immediately.
        Err(AppError::SecurityReadinessUnavailable) => None,
        Err(error) => return Err(error),
    };
    let mut failure = None;
    for &capability in capabilities {
        let isolated;
        let (cutover, active) = if let Some(state) = &state {
            state
        } else {
            match rollout_state(db, &[capability]).await {
                Ok(state) => {
                    isolated = state;
                    &isolated
                }
                Err(error) => {
                    tracing::warn!(capability, %error, "Security capability activation evidence unavailable");
                    failure.get_or_insert(error);
                    continue;
                }
            }
        };
        if active.contains_key(capability) {
            continue;
        }
        let mut assessment = assess(&rows, local, capability, *cutover, Utc::now());
        if !assessment.ready {
            continue;
        }
        let result = async {
            inventory(db, capability, &mut assessment).await?;
            if assessment.ready {
                insert_activation(db, capability).await?;
            }
            Ok::<_, AppError>(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(capability, %error, "Security capability activation recording failed");
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

async fn insert_activation(db: &Database, capability: &str) -> AppResult<()> {
    let record = SecurityCapabilityActivation {
        id: uuid::Uuid::new_v4().to_string(),
        record_kind: RECORD_KIND.into(),
        capability: capability.into(),
        active_since: Utc::now(),
    };
    // Equal unique-index predicates plus $setOnInsert make concurrent
    // heartbeats converge on one immutable first activation timestamp.
    db.collection::<SecurityCapabilityActivation>(ROLLOUTS)
        .update_one(
            doc! {"record_kind": RECORD_KIND, "capability": capability},
            doc! {"$setOnInsert": bson::to_document(&record)
            .map_err(|_| AppError::SecurityReadinessUnavailable)?},
        )
        .upsert(true)
        .await?;
    Ok(())
}

/// Serving decisions still read every live generation, including undeclared
/// and duplicate names. Only a recorded activation can tolerate roster churn.
pub async fn serving_report(db: &Database, capability: &str) -> AppResult<Readiness> {
    let report = report(db, capability).await?;
    if report.serving_with_roster_warnings {
        diagnostics::warn(db, capability, &report);
    }
    Ok(report)
}

/// Execution requires a known capability. Unlike activation of arbitrary
/// feature flags, an unknown execution contract must never bypass readiness.
pub async fn require_serving(db: &Database, capability: &str) -> AppResult<()> {
    if !CAPABILITIES.contains(&capability) && capability != AUTOMATION_COMPLETION {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    if !serving_report(db, capability).await?.serving_ready {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    Ok(())
}

/// Platform rollout controls are strict for known capabilities. Unknown keys
/// remain allowed because feature-flag writes also pass ordinary, ungated flags.
pub async fn require_activation(db: &Database, capability: &str) -> AppResult<()> {
    if capability == super::platform_model_service::FLAG {
        // Keep this optional foundation out of every existing flag caller's
        // inline future layout.
        return Box::pin(super::platform_model_service::require_activation_ready(db)).await;
    }
    if (CAPABILITIES.contains(&capability) || capability == AUTOMATION_COMPLETION)
        && !report(db, capability).await?.ready
    {
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
        require_serving(db, PLATFORM_POLICY).await?;
    }
    Ok(())
}

mod diagnostics;

#[cfg(test)]
pub(crate) async fn ready_test_replica(db: &Database) {
    drop(ready_test_replica_inner(db, false, None).await);
}

#[cfg(test)]
pub(crate) async fn ready_automation_test_replica(db: &Database) {
    drop(ready_test_replica_inner(db, true, None).await);
}

#[cfg(test)]
pub(crate) async fn controlled_ready_test_replica(db: &Database) -> tokio::task::JoinHandle<()> {
    ready_test_replica_inner(db, false, None).await
}

#[cfg(test)]
pub(crate) async fn overlapping_test_replica(db: &Database) -> SecurityReplica {
    let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
    let mut row = replicas
        .find_one(doc! {"_id":"test-process"})
        .await
        .unwrap()
        .unwrap();
    row.id = uuid::Uuid::new_v4().to_string();
    row.generation_id = row.id.clone();
    row.observed_at = Utc::now();
    replicas.insert_one(&row).await.unwrap();
    row
}

#[cfg(test)]
pub(crate) async fn remove_test_replica_capability(
    db: &Database,
    replica: &SecurityReplica,
    capability: &str,
) {
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&replica.id},
            doc! {"$pull":{"capabilities":capability}},
        )
        .await
        .unwrap();
}

#[cfg(test)]
struct TestHeartbeat {
    stop: tokio_util::sync::CancellationToken,
    finished: tokio::sync::watch::Receiver<()>,
}

#[cfg(test)]
static TEST_HEARTBEATS: OnceLock<std::sync::Mutex<BTreeMap<String, TestHeartbeat>>> =
    OnceLock::new();

/// Finish any in-flight publication before a fixture expires its observation.
#[cfg(test)]
pub(crate) async fn stop_test_replica(db: &Database) {
    let heartbeat = TEST_HEARTBEATS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(db.name());
    if let Some(mut heartbeat) = heartbeat {
        heartbeat.stop.cancel();
        let _ = heartbeat.finished.changed().await;
    }
}

#[cfg(test)]
async fn ready_test_replica_inner(
    db: &Database,
    automation_enabled: bool,
    excluded: Option<&'static str>,
) -> tokio::task::JoinHandle<()> {
    stop_test_replica(db).await;
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
    RUNTIME.get_or_init(|| local.clone());
    let db = db.clone();
    let mut capabilities: Vec<_> = CAPABILITIES
        .iter()
        .copied()
        .filter(|c| Some(*c) != excluded)
        .collect();
    if automation_enabled {
        capabilities.push(AUTOMATION_COMPLETION);
    }
    ensure_indexes(&db).await.expect("test readiness indexes");
    let mut row = SecurityReplica {
        id: local.identity.generation_id.clone(),
        instance_name: local.identity.instance_name.clone(),
        generation_id: local.identity.generation_id.clone(),
        protocol: 1,
        capabilities: capabilities
            .iter()
            .map(|capability| (*capability).to_owned())
            .collect(),
        expected_replicas: local.config.expected_replicas.clone(),
        drained: true,
        platform_policy_required: false,
        observed_at: Utc::now(),
    };
    let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
    replicas
        .replace_one(doc! {"_id": &row.id}, &row)
        .upsert(true)
        .await
        .expect("test replica observation");
    // Fixtures seed the same clean evidence in one command. Production heartbeat
    // recording and its failure/recovery tests still use record_clean_activations.
    let live = rows(&db).await.expect("test live replicas");
    let (cutover, active) = rollout_state(&db, &capabilities)
        .await
        .expect("test rollout state");
    let mut updates = Vec::new();
    for capability in capabilities {
        let mut assessment = assess(&live, &local, capability, cutover, Utc::now());
        if active.contains_key(capability) || !assessment.ready {
            continue;
        }
        inventory(&db, capability, &mut assessment)
            .await
            .expect("test readiness inventory");
        if !assessment.ready {
            continue;
        }
        let record = SecurityCapabilityActivation {
            id: uuid::Uuid::new_v4().to_string(),
            record_kind: RECORD_KIND.into(),
            capability: capability.into(),
            active_since: Utc::now(),
        };
        updates.push(
            mongodb::options::UpdateOneModel::builder()
                .namespace(mongodb::Namespace::new(db.name(), ROLLOUTS))
                .filter(doc! {"record_kind": RECORD_KIND, "capability": capability})
                .update(doc! {"$setOnInsert": bson::to_document(&record).unwrap()})
                .upsert(true)
                .build(),
        );
    }
    if !updates.is_empty() {
        db.client()
            .bulk_write(updates)
            .await
            .expect("test activation evidence");
    }
    let stop = tokio_util::sync::CancellationToken::new();
    let (finished, receiver) = tokio::sync::watch::channel(());
    TEST_HEARTBEATS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(
            db.name().to_owned(),
            TestHeartbeat {
                stop: stop.clone(),
                finished: receiver,
            },
        );
    tokio::spawn(async move {
        let _finished = finished;
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {},
            }
            row.observed_at = Utc::now();
            replicas
                .replace_one(doc! {"_id": &row.id}, &row)
                .upsert(true)
                .await
                .expect("test replica heartbeat");
        }
    })
}

#[cfg(test)]
#[path = "security_readiness_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) async fn legacy_org_guest_test_replica(db: &Database) -> tokio::task::JoinHandle<()> {
    ready_test_replica_inner(
        db,
        false,
        Some(super::org_channel_service::GUEST_GRANTS_READINESS),
    )
    .await
}
