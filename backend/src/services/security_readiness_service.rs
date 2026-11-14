//! Fleet capability support, separate from flag precedence and node OS support.
//! Optional roster/drain declarations are diagnostic. Observations cannot discover
//! binaries predating this observation protocol behind a load balancer.
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

#[derive(Clone, Debug)]
pub struct Runtime {
    pub identity: ReplicaIdentity,
    pub config: RolloutConfig,
    pub platform_policy_required: bool,
    activation_progress: Arc<Mutex<BTreeMap<String, ActivationProgress>>>,
}

#[derive(Debug)]
struct ActivationProgress {
    since: Instant,
    last_observed: Instant,
}

impl Runtime {
    pub fn new(
        identity: ReplicaIdentity,
        config: RolloutConfig,
        platform_policy_required: bool,
    ) -> Self {
        Self {
            identity,
            config,
            platform_policy_required,
            activation_progress: Default::default(),
        }
    }

    fn reset_activation(&self, capability: Option<&str>) {
        let mut pending = self
            .activation_progress
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(capability) = capability {
            pending.remove(capability);
        } else {
            pending.clear();
        }
    }

    fn settled(&self, capability: &str, now: Instant) -> bool {
        let mut pending = self
            .activation_progress
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let progress = pending
            .entry(capability.to_owned())
            .or_insert(ActivationProgress {
                since: now,
                last_observed: now,
            });
        // A longer gap could hide an unsupported generation's entire live observation.
        if now.saturating_duration_since(progress.last_observed)
            > std::time::Duration::from_secs(MAX_AGE_SECS as u64)
        {
            progress.since = now;
        }
        progress.last_observed = now;
        now.saturating_duration_since(progress.since) >= ACTIVATION_SETTLE_WINDOW
    }

    fn pending_seconds(&self, capability: &str, now: Instant) -> Option<u64> {
        self.activation_progress
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(capability)
            .map(|progress| {
                let remaining = ACTIVATION_SETTLE_WINDOW
                    .saturating_sub(now.saturating_duration_since(progress.since));
                remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0)
            })
    }
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
const ACTIVATION_SETTLE_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(2 * MAX_AGE_SECS as u64);

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
    observe_with_automation(
        db,
        &Runtime::new(identity.clone(), config.clone(), platform_policy_required),
        false,
    )
    .await
}

/// Optional writer capability: disabled replicas retain the main heartbeat payload.
pub async fn observe_with_automation(
    db: &Database,
    local: &Runtime,
    automation_enabled: bool,
) -> AppResult<()> {
    let mut capabilities = CAPABILITIES.to_vec();
    if automation_enabled {
        capabilities.push(AUTOMATION_COMPLETION);
    }
    observe_for(db, local, &capabilities).await
}

#[cfg(test)]
async fn observe_with_capabilities(
    db: &Database,
    identity: &ReplicaIdentity,
    config: &RolloutConfig,
    platform_policy_required: bool,
    capabilities: &[&str],
) -> AppResult<()> {
    observe_for(
        db,
        &Runtime::new(identity.clone(), config.clone(), platform_policy_required),
        capabilities,
    )
    .await
}

async fn observe_for(db: &Database, local: &Runtime, capabilities: &[&str]) -> AppResult<()> {
    let published = publish_observation(db, local, capabilities).await;
    if published.is_err() {
        local.reset_activation(None);
        return published;
    }
    if let Err(error) = record_activations_with_clock(db, local, capabilities, Instant::now).await {
        tracing::warn!(%error, "Security capability activation recording incomplete; next heartbeat will retry");
    }
    Ok(())
}

async fn publish_observation(
    db: &Database,
    local: &Runtime,
    capabilities: &[&str],
) -> AppResult<()> {
    let Runtime {
        identity,
        config,
        platform_policy_required,
        ..
    } = local;
    if *platform_policy_required {
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
        platform_policy_required: *platform_policy_required,
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
    /// Legacy clean-roster diagnostic and immediate-recording signal; never an admission gate.
    pub ready: bool,
    pub serving_ready: bool,
    pub serving_with_roster_warnings: bool,
    pub roster_warnings: Vec<String>,
    pub serving_blockers: Vec<String>,
    pub activated: bool,
    pub active_since: Option<DateTime<Utc>>,
    /// Replica-local advisory countdown; never used to authorize serving.
    pub activation_pending_seconds: Option<u64>,
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
            | Self::ServingRosterNotDeclared
            | Self::OldReplicasNotDeclaredDrained => true,
            Self::ExecutingProcessNotObserved
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
            Self::MissingOrDuplicateReplica(name) => {
                format!("missing_or_duplicate_replica:{name}")
            }
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
        self.serving_ready = self.activated && self.serving_blockers.is_empty();
        if !self.activated {
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
        activation_pending_seconds: None,
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
    Ok(report_with_declarations(db, local, capability).await?.0)
}

async fn report_with_declarations(
    db: &Database,
    local: &Runtime,
    capability: &str,
) -> AppResult<(Readiness, bool)> {
    let rows = rows(db).await?;
    // False is the historical default and cannot distinguish an explicit false
    // from an unset env. Only a roster or a positive drain assertion declares
    // something to compare; an entirely undeclared fleet is a supported default.
    let declared = !local.config.expected_replicas.is_empty()
        || local.config.drained
        || rows
            .iter()
            .any(|row| !row.expected_replicas.is_empty() || row.drained);
    let (cutover, activations) = rollout_state(db, &[capability]).await?;
    let mut report = assess(&rows, local, capability, cutover, Utc::now());
    inventory(db, capability, &mut report).await?;
    if report.serving_blockers.is_empty() && !activations.contains_key(capability) {
        report.activation_pending_seconds = local.pending_seconds(capability, Instant::now());
    }
    Ok((
        report.with_activation(activations.get(capability).copied()),
        declared,
    ))
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

#[cfg(test)]
async fn record_activations(
    db: &Database,
    local: &Runtime,
    capabilities: &[&str],
    now: Instant,
) -> AppResult<()> {
    record_activations_with_clock(db, local, capabilities, || now).await
}

async fn record_activations_with_clock(
    db: &Database,
    local: &Runtime,
    capabilities: &[&str],
    clock: impl Fn() -> Instant,
) -> AppResult<()> {
    let rows = match rows(db).await {
        Ok(rows) => rows,
        Err(error) => {
            local.reset_activation(None);
            return Err(error);
        }
    };
    let state = match rollout_state(db, capabilities).await {
        Ok(state) => Some(state),
        // Retry malformed/duplicate evidence per capability in this pass. Only
        // an incomplete individual assessment resets its window; resetting all
        // here would starve healthy capabilities while one record stays corrupt.
        Err(AppError::SecurityReadinessUnavailable) => None,
        Err(error) => {
            local.reset_activation(None);
            return Err(error);
        }
    };
    let mut failure = None;
    let mut inserted = Vec::new();
    let mut warnings = BTreeSet::new();
    let mut live_generations = 0;
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
                    local.reset_activation(Some(capability));
                    tracing::warn!(capability, %error, "Security capability activation evidence unavailable");
                    failure.get_or_insert(error);
                    continue;
                }
            }
        };
        if active.contains_key(capability) {
            local.reset_activation(Some(capability));
            continue;
        }
        let mut assessment = assess(&rows, local, capability, *cutover, Utc::now());
        if !assessment.serving_blockers.is_empty() {
            local.reset_activation(Some(capability));
            continue;
        }
        let result = async {
            inventory(db, capability, &mut assessment).await?;
            // Start/advance the debounce only after the complete assessment.
            // Database latency before observing support must not shorten it.
            if !assessment.serving_blockers.is_empty() {
                local.reset_activation(Some(capability));
            } else if assessment.ready || local.settled(capability, clock()) {
                if insert_activation(db, capability).await? {
                    inserted.push(capability.to_owned());
                    warnings.extend(assessment.roster_warnings);
                    live_generations = assessment.live_generations;
                }
                local.reset_activation(Some(capability));
            }
            Ok::<_, AppError>(())
        }
        .await;
        if let Err(error) = result {
            local.reset_activation(Some(capability));
            tracing::warn!(capability, %error, "Security capability activation recording failed");
            failure.get_or_insert(error);
        }
    }
    if !inserted.is_empty() {
        activation_audit(
            db,
            inserted,
            warnings.into_iter().collect(),
            live_generations,
            &local.config.expected_replicas,
        )
        .await;
    }
    failure.map_or(Ok(()), Err)
}

async fn activation_audit(
    db: &Database,
    mut capabilities: Vec<String>,
    roster_warnings: Vec<String>,
    live_generations: usize,
    expected_replicas: &[String],
) {
    capabilities.sort();
    let roster_clean = roster_warnings.is_empty();
    tracing::info!(
        ?capabilities,
        roster_clean,
        ?roster_warnings,
        live_generations,
        ?expected_replicas,
        "Security capabilities activated"
    );
    let work = super::audit_service::log_system_event(
        db.clone(),
        "security_capability_activated",
        Some(serde_json::json!({
            "capabilities": capabilities, "roster_clean": roster_clean, "roster_warnings": roster_warnings,
            "live_generations": live_generations, "expected_replicas": expected_replicas,
        })),
    );
    match tokio::time::timeout(std::time::Duration::from_secs(5), work).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::debug!(%error, "Security capability activation audit unavailable")
        }
        Err(_) => tracing::debug!("Security capability activation audit timed out"),
    }
}

async fn insert_activation(db: &Database, capability: &str) -> AppResult<bool> {
    let record = SecurityCapabilityActivation {
        id: uuid::Uuid::new_v4().to_string(),
        record_kind: RECORD_KIND.into(),
        capability: capability.into(),
        active_since: Utc::now(),
    };
    // Equal unique-index predicates plus $setOnInsert make concurrent
    // heartbeats converge on one immutable first activation timestamp.
    let result = db
        .collection::<SecurityCapabilityActivation>(ROLLOUTS)
        .update_one(
            doc! {"record_kind": RECORD_KIND, "capability": capability},
            doc! {"$setOnInsert": bson::to_document(&record)
            .map_err(|_| AppError::SecurityReadinessUnavailable)?},
        )
        .upsert(true)
        .await?;
    Ok(result.upserted_id.is_some())
}

/// Serving decisions still read every live generation, including undeclared
/// and duplicate names. Only a recorded activation can tolerate roster churn.
pub async fn serving_report(db: &Database, capability: &str) -> AppResult<Readiness> {
    serving_report_for(db, runtime()?, capability).await
}

async fn serving_report_for(
    db: &Database,
    local: &Runtime,
    capability: &str,
) -> AppResult<Readiness> {
    let (report, declared) = report_with_declarations(db, local, capability).await?;
    if declared && report.serving_with_roster_warnings {
        diagnostics::warn(db, capability, &report);
    }
    Ok(report)
}

fn known_capability(capability: &str) -> bool {
    CAPABILITIES.contains(&capability) || capability == AUTOMATION_COMPLETION
}

/// Execution requires a known capability; unknown contracts fail closed.
/// Flag enables have the deliberate asymmetry documented in require_flag_enable.
pub async fn require_serving(db: &Database, capability: &str) -> AppResult<()> {
    if !known_capability(capability) {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    if !serving_report(db, capability).await?.serving_ready {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    Ok(())
}

/// Gated flag enables require recorded activation and live serving support.
/// Unknown flag keys are ungated and do no readiness I/O: flag writers accept
/// arbitrary ordinary flags, whereas unknown execution contracts must refuse.
pub async fn require_flag_enable(db: &Database, key: &str) -> AppResult<()> {
    use super::platform_model_service as models;
    let individual = [key];
    let model_capabilities = [
        models::REGISTRY_READINESS,
        models::EXECUTION_READINESS,
        models::BILLING_READINESS,
    ];
    let required = if key == models::FLAG {
        if model_capabilities
            .iter()
            .any(|capability| !CAPABILITIES.contains(capability))
        {
            return Err(AppError::SecurityReadinessUnavailable);
        }
        &model_capabilities[..]
    } else if known_capability(key) {
        &individual[..]
    } else {
        return Ok(());
    };
    for capability in required {
        require_serving(db, capability).await?;
    }
    Ok(())
}

/// A replica with the legacy env value must not bypass a sibling's enabled
/// policy during a mixed deployment. The cutover marker remains sticky.
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
        activation_progress: Default::default(),
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
    // Fixtures seed the same clean evidence in one command. Production recording
    // and its failure/recovery tests retain the individual activation upserts.
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

#[cfg(test)]
mod activation_tests;

#[cfg(test)]
mod flag_enable_tests;

#[cfg(test)]
mod convergence_tests;
