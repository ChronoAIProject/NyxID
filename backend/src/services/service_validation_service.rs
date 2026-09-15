//! Fenced, expiring observations; probe outcomes never mutate credential health.
//! Only classified provider answers have a five-minute reuse window. Unsupported,
//! transport, and local configuration observations are non-reusable, so the next
//! explicit check re-evaluates node routing and capabilities.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use mongodb::bson::doc;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use super::coordination_service::{self, LeaseStore, LeaseToken, SlotStore, SlotToken};
use super::proxy_service::{self, UserServiceResolution};
use super::validator_profiles::{self, ValidationOutcome, ValidatorProfile};
use super::{execution_authority, node_routing_service, validation_transport};
use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::coordination::{CoordinationLease, LEASE_COLLECTION_NAME};
use crate::models::service_validation_record::{
    COLLECTION_NAME, CallerContext, NodeCredentialBinding, ServiceValidationRecord,
};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

const DISPLAY_WINDOW_SECS: i64 = 300;
const LEASE_TTL: Duration = Duration::from_secs(30);
const FOREGROUND_DEADLINE: Duration = Duration::from_secs(10);
const MIN_PROBE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct ValidationCaller {
    pub user_id: String,
    pub session_id: Option<String>,
    pub context: CallerContext,
    pub allow_all_services: bool,
    pub allowed_service_ids: Vec<String>,
    pub allow_all_nodes: bool,
    pub allowed_node_ids: Vec<String>,
}

struct Materialized(UserServiceResolution);

impl std::ops::Deref for Materialized {
    type Target = UserServiceResolution;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for Materialized {
    fn drop(&mut self) {
        self.0.target.credential.zeroize();
    }
}

async fn materialized_matches(
    state: &AppState,
    target: &UserServiceResolution,
    revision: Option<&str>,
) -> AppResult<bool> {
    if target.master_credential {
        let service = state
            .db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &target.user_service_id })
            .await?
            .ok_or(AppError::ServiceValidationRejected)?;
        let encrypted = master_credential_material(&state.db, &service).await?;
        if Some(master_revision(&encrypted).as_str()) != revision {
            return Ok(false);
        }
        let plaintext = zeroize::Zeroizing::new(state.encryption_keys.decrypt(&encrypted).await?);
        return Ok(plaintext.as_slice() == target.target.credential.as_bytes());
    }
    let Some(id) = target.api_key_id.as_ref() else {
        return Ok(true);
    };
    let Some(key) = state
        .db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": id })
        .await?
    else {
        return Ok(false);
    };
    if Some(credential_revision(&key).as_str()) != revision {
        return Ok(false);
    }
    let encrypted = if matches!(
        key.credential_type.as_str(),
        "oauth2" | "gcp_service_account"
    ) {
        key.access_token_encrypted
    } else {
        key.credential_encrypted
    };
    let Some(encrypted) = encrypted else {
        return Ok(false);
    };
    let plaintext = zeroize::Zeroizing::new(state.encryption_keys.decrypt(&encrypted).await?);
    Ok(plaintext.as_slice() == target.target.credential.as_bytes())
}

struct Snapshot {
    service: UserService,
    resolution: UserServiceResolution,
    digest: String,
    revision: Option<String>,
    credential_type: Option<String>,
    fallback_nodes: Vec<String>,
    node_configured: bool,
    node_credential: Option<NodeCredentialBinding>,
}

async fn snapshot(
    state: &AppState,
    caller: &ValidationCaller,
    service_id: &str,
) -> AppResult<Snapshot> {
    if !caller.allow_all_services && !caller.allowed_service_ids.iter().any(|id| id == service_id) {
        return Err(AppError::Forbidden(
            "Service is outside the caller's grant".into(),
        ));
    }
    let service = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Key not found".into()))?;
    // This call enforces the actual actor's role, service scope, and admin-only
    // rule before any record, refresh, or provider request is made.
    let resolution = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
        &state.db,
        state.encryption_keys.as_ref(),
        &caller.user_id,
        service_id,
        Some(&service.slug),
    )
    .await?
    .ok_or(AppError::ServiceValidationRejected)?;
    if !service.is_active {
        return Err(AppError::ServiceValidationRejected);
    }
    let fallback_nodes = if resolution.master_credential {
        // Match ordinary proxy routing: platform material never leaves the server.
        Vec::new()
    } else {
        node_routing_service::list_configured_binding_node_ids(
            &state.db,
            &service.user_id,
            &resolution.target.service.id,
        )
        .await?
    };
    if !caller.allow_all_nodes
        && resolution
            .node_id
            .as_ref()
            .is_some_and(|node| !caller.allowed_node_ids.contains(node))
    {
        return Err(AppError::Forbidden(
            "Node is outside the caller's grant".into(),
        ));
    }
    if resolution.node_id.is_none()
        && !fallback_nodes.is_empty()
        && !caller.allow_all_nodes
        && !fallback_nodes
            .iter()
            .any(|node| caller.allowed_node_ids.contains(node))
    {
        return Err(AppError::Forbidden(
            "Node is outside the caller's grant".into(),
        ));
    }
    let digest = execution_authority::digest(&execution_authority::build_projection(
        &resolution,
        None,
        fallback_nodes.clone(),
    ));
    let key = match resolution.api_key_id.as_deref() {
        Some(id) => {
            state
                .db
                .collection::<UserApiKey>(USER_API_KEYS)
                .find_one(doc! { "_id": id })
                .await?
        }
        None => None,
    };
    let (node_configured, node_credential) = node_routing_service::validation_route(
        &state.db,
        &state.node_ws_manager,
        resolution.node_id.as_deref(),
        &fallback_nodes,
        &resolution.target.service.slug,
        (!caller.allow_all_nodes).then_some(caller.allowed_node_ids.as_slice()),
    )
    .await?;
    let revision = if resolution.master_credential {
        Some(master_revision(
            &master_credential_material(&state.db, &service).await?,
        ))
    } else {
        key.as_ref().map(credential_revision)
    };
    Ok(Snapshot {
        node_configured,
        node_credential,
        service,
        resolution,
        digest,
        revision,
        credential_type: key.map(|key| key.credential_type),
        fallback_nodes,
    })
}

async fn master_credential_material(
    db: &mongodb::Database,
    service: &UserService,
) -> AppResult<Vec<u8>> {
    use crate::models::downstream_service::{COLLECTION_NAME, DownstreamService};
    let catalog = db
        .collection::<DownstreamService>(COLLECTION_NAME)
        .find_one(doc! { "_id": service.catalog_service_id.as_deref().unwrap_or("") })
        .await?
        .ok_or(AppError::ServiceValidationRejected)?;
    Ok(catalog.credential_encrypted)
}

fn master_revision(encrypted: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"nyxid-validation-master-credential-v1");
    hash.update(encrypted);
    hex::encode(hash.finalize())
}

pub(crate) fn credential_revision(key: &UserApiKey) -> String {
    // Refresh intentionally preserves credential_epoch. Bind refreshed material
    // and scopes separately, so a late rejection cannot outlive a newer token.
    let mut hash = Sha256::new();
    for part in [
        key.access_token_encrypted.as_deref().unwrap_or_default(),
        key.refresh_token_encrypted.as_deref().unwrap_or_default(),
        key.credential_encrypted.as_deref().unwrap_or_default(),
        key.token_scopes.as_deref().unwrap_or("").as_bytes(),
        key.status.as_bytes(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.update(key.credential_epoch.to_be_bytes());
    hex::encode(hash.finalize())
}

fn fresh(record: &ServiceValidationRecord, live: &Snapshot, version: u32) -> bool {
    evidence_is_fresh(
        record,
        &live.digest,
        live.revision.as_deref(),
        live.node_credential.as_ref(),
        version,
        Utc::now(),
    )
}

pub(crate) fn evidence_is_fresh(
    record: &ServiceValidationRecord,
    digest: &str,
    revision: Option<&str>,
    node_credential: Option<&NodeCredentialBinding>,
    version: u32,
    now: chrono::DateTime<Utc>,
) -> bool {
    record.completed
        && record.validator_version == version
        && record.execution_authority_digest == digest
        && record.credential_revision.as_deref() == revision
        && record.node_credential.as_ref() == node_credential
        && node_credential.is_none_or(|node| node.revision.is_some())
        && record.valid_until > now
}

pub async fn validate(
    state: &AppState,
    caller: ValidationCaller,
    service_id: &str,
    force: bool,
) -> AppResult<ServiceValidationRecord> {
    tokio::time::timeout(
        FOREGROUND_DEADLINE,
        validate_round(state, caller, service_id, force),
    )
    .await
    .map_err(|_| AppError::ServiceValidationUnavailable)?
}

async fn validate_round(
    state: &AppState,
    caller: ValidationCaller,
    service_id: &str,
    force: bool,
) -> AppResult<ServiceValidationRecord> {
    loop {
        let live = snapshot(state, &caller, service_id).await?;
        let profile = live
            .resolution
            .catalog_service_slug
            .as_deref()
            .and_then(validator_profiles::for_slug);
        let (profile_id, version) =
            profile.map_or(("unsupported", 0), |profile| (profile.id, profile.version));
        let records = state
            .db
            .collection::<ServiceValidationRecord>(COLLECTION_NAME);
        let previous = records
            .find_one(doc! { "user_service_id": service_id, "validator_id": profile_id })
            .await?;
        if !force
            && let Some(record) = previous
                .as_ref()
                .filter(|record| fresh(record, &live, version))
        {
            return Ok(record.clone());
        }
        let lease_name = format!("service-validation:{service_id}:{profile_id}");
        let holder = &coordination_service::cluster_lease_runtime().holder;
        let lease = LeaseStore::acquire(&state.db, &lease_name, holder, LEASE_TTL).await?;
        let attempt_id = if let Some(lease) = lease {
            let attempt_id = lease.lease_id.clone();
            // Re-read under the lease in case the previous worker just completed.
            let latest = match records
                .find_one(doc! { "user_service_id": service_id, "validator_id": profile_id })
                .await
            {
                Ok(latest) => latest,
                Err(error) => {
                    let _ = LeaseStore::release(&state.db, &lease).await;
                    return Err(error.into());
                }
            };
            let joining = previous.as_ref().is_none_or(|record| !record.completed);
            if let Some(record) = latest
                .as_ref()
                .filter(|record| (!force || joining) && fresh(record, &live, version))
            {
                let _ = LeaseStore::release(&state.db, &lease).await;
                let current = snapshot(state, &caller, service_id).await?;
                return if fresh(record, &current, version) {
                    Ok(record.clone())
                } else {
                    Err(AppError::ServiceValidationUnavailable)
                };
            }
            let started = start_attempt(state, &caller, live, profile, latest, &lease).await;
            match started {
                Ok((record, admission)) => {
                    let state = state.clone();
                    let caller = caller.clone();
                    tokio::spawn(async move {
                        run_attempt(state, caller, profile, record, lease, admission).await;
                    });
                }
                Err(error) => {
                    let _ = LeaseStore::release(&state.db, &lease).await;
                    return Err(error);
                }
            }
            attempt_id
        } else {
            match join_attempt(state, &lease_name).await? {
                Some(attempt_id) => {
                    if previous
                        .as_ref()
                        .is_some_and(|record| record.completed && record.attempt_id == attempt_id)
                    {
                        // A published result can outlive its worker's admission/audit
                        // cleanup. This new request must not join that settled check.
                        wait_for_attempt_release(state, &lease_name, &attempt_id).await?;
                        continue;
                    }
                    attempt_id
                }
                None => {
                    if let Some(record) = previous.as_ref().filter(|record| !record.completed) {
                        return completed_observation(
                            state,
                            &caller,
                            service_id,
                            profile_id,
                            version,
                            Some(&record.attempt_id),
                        )
                        .await?
                        .ok_or(AppError::ServiceValidationUnavailable);
                    }
                    // Cleanup raced the lease read. Re-enter normal admission
                    // rather than reusing a completed record for a new check.
                    continue;
                }
            }
        };
        // Disconnecting does not cancel a rotating OAuth refresh. The detached
        // worker retains its Mongo leases until it settles or loses authority.
        return poll_attempt(
            state,
            &caller,
            service_id,
            profile_id,
            version,
            &lease_name,
            &attempt_id,
        )
        .await;
    }
}

async fn wait_for_attempt_release(
    state: &AppState,
    lease_name: &str,
    attempt_id: &str,
) -> AppResult<()> {
    while join_attempt(state, lease_name).await?.as_deref() == Some(attempt_id) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

async fn join_attempt(state: &AppState, lease_name: &str) -> AppResult<Option<String>> {
    Ok(state
        .db
        .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
        .find_one(doc! { "_id": lease_name, "$expr": { "$gt": ["$expires_at", "$$NOW"] } })
        .await?
        .map(|lease| lease.lease_id))
}

async fn completed_observation(
    state: &AppState,
    caller: &ValidationCaller,
    service_id: &str,
    profile_id: &str,
    version: u32,
    attempt_id: Option<&str>,
) -> AppResult<Option<ServiceValidationRecord>> {
    let mut filter = doc! {
        "user_service_id": service_id, "validator_id": profile_id, "completed": true,
    };
    if let Some(attempt_id) = attempt_id {
        filter.insert("attempt_id", attempt_id);
    }
    let Some(record) = state
        .db
        .collection::<ServiceValidationRecord>(COLLECTION_NAME)
        .find_one(filter)
        .await?
    else {
        return Ok(None);
    };
    // The caller waiting for this exact attempt receives its settled outcome,
    // including an expired abort. Freshness gates only reuse of prior evidence.
    if attempt_id.is_some() {
        return Ok(Some(record));
    }
    if record.valid_until <= Utc::now() {
        return Err(AppError::ServiceValidationUnavailable);
    }
    let current = snapshot(state, caller, service_id).await?;
    if fresh(&record, &current, version) {
        Ok(Some(record))
    } else {
        Err(AppError::ServiceValidationUnavailable)
    }
}

async fn poll_attempt(
    state: &AppState,
    caller: &ValidationCaller,
    service_id: &str,
    profile_id: &str,
    version: u32,
    lease_name: &str,
    attempt_id: &str,
) -> AppResult<ServiceValidationRecord> {
    loop {
        if let Some(record) = completed_observation(
            state,
            caller,
            service_id,
            profile_id,
            version,
            Some(attempt_id),
        )
        .await?
        {
            return Ok(record);
        }
        if join_attempt(state, lease_name).await?.as_deref() != Some(attempt_id) {
            // Settlement may have raced the first read and released the lease.
            // Re-read once so a completed observation is not mistaken for loss.
            return completed_observation(
                state,
                caller,
                service_id,
                profile_id,
                version,
                Some(attempt_id),
            )
            .await?
            .ok_or(AppError::ServiceValidationUnavailable);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct Admission {
    deployment: SlotToken,
    session: SlotToken,
    app: Option<SlotToken>,
    cooldown: Option<LeaseToken>,
}

async fn acquire_admission(state: &AppState, caller: &ValidationCaller) -> AppResult<Admission> {
    let holder = &coordination_service::cluster_lease_runtime().holder;
    // A token without a browser session shares its human subject's two slots.
    let session_scope = caller.session_id.as_deref().unwrap_or(&caller.user_id);
    let deployment = SlotStore::acquire(
        &state.db,
        "service-validation-deployment",
        "deployment",
        32,
        holder,
        LEASE_TTL,
    )
    .await?
    .ok_or(AppError::ServiceValidationRateLimited)?;
    let session = SlotStore::acquire(
        &state.db,
        "service-validation-session",
        session_scope,
        2,
        holder,
        LEASE_TTL,
    )
    .await;
    let session = match session {
        Ok(Some(slot)) => slot,
        other => {
            let _ = SlotStore::release(&state.db, &deployment).await;
            return Err(other
                .err()
                .unwrap_or(AppError::ServiceValidationRateLimited));
        }
    };
    let mut admission = Admission {
        deployment,
        session,
        app: None,
        cooldown: None,
    };
    let result: AppResult<()> = async {
        if let CallerContext::App { client_id } = &caller.context {
            admission.app = Some(
                SlotStore::acquire(
                    &state.db,
                    "service-validation-app",
                    client_id,
                    16,
                    holder,
                    LEASE_TTL,
                )
                .await?
                .ok_or(AppError::ServiceValidationRateLimited)?,
            );
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        release_admission(&state.db, &admission).await;
        return Err(error);
    }
    Ok(admission)
}

async fn start_attempt(
    state: &AppState,
    caller: &ValidationCaller,
    live: Snapshot,
    profile: Option<&ValidatorProfile>,
    previous: Option<ServiceValidationRecord>,
    lease: &LeaseToken,
) -> AppResult<(ServiceValidationRecord, Admission)> {
    let holder = &coordination_service::cluster_lease_runtime().holder;
    let mut admission = acquire_admission(state, caller).await?;
    let result = async {
        if let Some(profile) = profile {
            let identity = live.resolution.api_key_id.as_deref().unwrap_or(&live.service.id);
            let cooldown_name = format!("service-validation-cooldown:{identity}:{}", profile.id);
            admission.cooldown = Some(LeaseStore::acquire(&state.db, &cooldown_name, holder, MIN_PROBE_INTERVAL).await?.ok_or(AppError::ServiceValidationRateLimited)?);
        }
        let now = Utc::now();
        let previous_attempt = previous.as_ref().map(|record| record.attempt_id.clone());
        let record = ServiceValidationRecord {
            id: previous.map_or_else(|| uuid::Uuid::new_v4().to_string(), |record| record.id),
            user_service_id: live.service.id,
            owner_id: live.service.user_id,
            validator_id: profile.map_or("unsupported", |profile| profile.id).into(),
            validator_version: profile.map_or(0, |profile| profile.version),
            execution_authority_digest: live.digest,
            api_key_id: live.resolution.api_key_id.clone(),
            credential_epoch: live.resolution.api_key_id.as_ref().map(|_| live.resolution.credential_epoch),
            attempt_id: lease.lease_id.clone(), completed: false,
            credential_revision: live.revision,
            node_credential: live.node_credential,
            reason_code: "checking".into(), outcome: ValidationOutcome::TransportUnknown,
            checked_at: now, valid_until: now + chrono::Duration::seconds(DISPLAY_WINDOW_SECS),
            caller_context: caller.context.clone(),
        };
        if !LeaseStore::renew(&state.db, lease, LEASE_TTL).await? {
            return Err(AppError::ServiceValidationUnavailable);
        }
        let mut filter = doc! { "user_service_id": &record.user_service_id, "validator_id": &record.validator_id };
        if let Some(attempt) = previous_attempt {
            filter.insert("attempt_id", attempt);
            let replaced = state.db.collection::<ServiceValidationRecord>(COLLECTION_NAME).replace_one(filter, &record).await?;
            if replaced.matched_count != 1 { return Err(AppError::ServiceValidationUnavailable); }
        } else {
            state.db.collection::<ServiceValidationRecord>(COLLECTION_NAME).insert_one(&record).await?;
        }
        Ok(record)
    }.await;
    match result {
        Ok(record) => Ok((record, admission)),
        Err(error) => {
            release_cooldown(&state.db, &admission).await;
            release_admission(&state.db, &admission).await;
            Err(error)
        }
    }
}

async fn release_admission(db: &mongodb::Database, admission: &Admission) {
    if let Some(app) = &admission.app {
        let _ = SlotStore::release(db, app).await;
    }
    let _ = SlotStore::release(db, &admission.session).await;
    let _ = SlotStore::release(db, &admission.deployment).await;
    // A dispatched attempt keeps its provider cooldown after releasing active slots.
}

async fn release_cooldown(db: &mongodb::Database, admission: &Admission) {
    if let Some(cooldown) = &admission.cooldown {
        let _ = LeaseStore::release(db, cooldown).await;
    }
}

#[derive(Clone, Copy)]
enum AttemptFailure {
    Superseded,
    CredentialUnavailable,
    LeaseLost,
    Internal,
}

impl AttemptFailure {
    fn materialization(error: AppError) -> Self {
        match error {
            AppError::BadRequest(_)
            | AppError::NotFound(_)
            | AppError::ServiceValidationRejected => Self::CredentialUnavailable,
            _ => Self::Internal,
        }
    }

    fn reason_code(self) -> &'static str {
        match self {
            Self::Superseded => "attempt_superseded",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::LeaseLost => "lease_lost",
            Self::Internal => "internal_error",
        }
    }
}

impl From<AppError> for AttemptFailure {
    fn from(_: AppError) -> Self {
        Self::Internal
    }
}

async fn run_attempt(
    state: AppState,
    caller: ValidationCaller,
    profile: Option<&'static ValidatorProfile>,
    mut record: ServiceValidationRecord,
    lease: LeaseToken,
    admission: Admission,
) {
    let dispatched = AtomicBool::new(false);
    let work = observe(
        &state,
        &caller,
        profile,
        record.clone(),
        &admission,
        &dispatched,
    );
    tokio::pin!(work);
    let mut renewal = tokio::time::interval(Duration::from_secs(10));
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            _ = renewal.tick() => {
                let renewed = async {
                    Ok::<_, AppError>(LeaseStore::renew(&state.db, &lease, LEASE_TTL).await?
                        && SlotStore::renew(&state.db, &admission.deployment, LEASE_TTL).await?
                        && SlotStore::renew(&state.db, &admission.session, LEASE_TTL).await?
                        && match admission.app.as_ref() {
                            Some(app) => SlotStore::renew(&state.db, app, LEASE_TTL).await?,
                            None => true,
                        }
                        && match admission.cooldown.as_ref() {
                            Some(cooldown) => extend_cooldown(&state.db, cooldown, MIN_PROBE_INTERVAL).await?,
                            None => true,
                        })
                }.await;
                if !matches!(renewed, Ok(true)) { break Err(AttemptFailure::LeaseLost); }
            }
        }
    };
    if !dispatched.load(Ordering::Relaxed) {
        release_cooldown(&state.db, &admission).await;
    }
    if let Err(failure) = result {
        record.completed = true;
        record.outcome = ValidationOutcome::TransportUnknown;
        record.reason_code = failure.reason_code().into();
        record.checked_at = Utc::now();
        // Internal aborts must not become reusable provider evidence.
        record.valid_until = record.checked_at;
        match finish_observation(&state.db, &record).await {
            Ok(true) => {
                if audit_observation(&state, &caller, &record, None)
                    .await
                    .is_err()
                {
                    tracing::warn!(attempt_id = %lease.lease_id, "Could not audit validation abort");
                }
            }
            Ok(false) => {}
            Err(_) => {
                tracing::warn!(attempt_id = %lease.lease_id, "Could not settle validation abort");
            }
        }
    }
    release_admission(&state.db, &admission).await;
    let _ = LeaseStore::release(&state.db, &lease).await;
}

async fn observe(
    state: &AppState,
    caller: &ValidationCaller,
    profile: Option<&ValidatorProfile>,
    mut record: ServiceValidationRecord,
    admission: &Admission,
    dispatched: &AtomicBool,
) -> Result<(), AttemptFailure> {
    let before = snapshot(state, caller, &record.user_service_id).await?;
    if before.digest != record.execution_authority_digest
        || before.revision != record.credential_revision
        || before.node_credential != record.node_credential
    {
        return Err(AttemptFailure::Superseded);
    }
    let unsupported_type = matches!(
        before.credential_type.as_deref(),
        Some("node_managed" | "ssh_certificate")
    );
    let mut status_class = None;
    let (outcome, reason) = if let Some(profile) = profile.filter(|_| !unsupported_type) {
        let slug = before
            .resolution
            .catalog_service_slug
            .as_deref()
            .ok_or(AppError::ServiceValidationRejected)?;
        if validation_transport::profile_url(profile, slug, &before.resolution.target.base_url)
            .is_err()
        {
            (
                ValidationOutcome::ConfigurationError,
                "target_not_allowlisted".to_string(),
            )
        } else if !matches!(
            before.resolution.target.auth_method.as_str(),
            "bearer" | "header" | "query" | "path" | "basic"
        ) {
            (
                ValidationOutcome::Unsupported,
                "unsupported_auth_method".to_string(),
            )
        } else {
            if let Some(cooldown) = admission.cooldown.as_ref()
                && !extend_cooldown(&state.db, cooldown, MIN_PROBE_INTERVAL).await?
            {
                return Err(AttemptFailure::LeaseLost);
            }
            let response = if before.node_configured {
                if let Some(node) = &before.node_credential {
                    validation_transport::send_via_node(
                        state,
                        &node.node_id,
                        profile,
                        slug,
                        &before.resolution.target,
                        dispatched,
                    )
                    .await
                } else {
                    Err(validation_transport::TransportError::Unavailable)
                }
            } else {
                let materialized = Materialized(
                    proxy_service::resolve_proxy_target_by_user_service_id(
                        &state.db,
                        state.encryption_keys.as_ref(),
                        &caller.user_id,
                        &record.user_service_id,
                        Some(&before.service.slug),
                        None,
                        proxy_service::ProxyExecutionContext::new(
                            Some(&state.connection_expiry_notifier),
                            state.platform_user_rate_limit,
                        )
                        .without_usage_touch(),
                    )
                    .await
                    .map_err(AttemptFailure::materialization)?
                    .ok_or(AttemptFailure::CredentialUnavailable)?,
                );
                let materialized_digest =
                    execution_authority::digest(&execution_authority::build_projection(
                        &materialized,
                        None,
                        before.fallback_nodes.clone(),
                    ));
                let current = snapshot(state, caller, &record.user_service_id).await?;
                if materialized_digest != before.digest
                    || current.digest != before.digest
                    || !materialized_matches(state, &materialized, current.revision.as_deref())
                        .await?
                {
                    return Err(AttemptFailure::Superseded);
                }
                record.credential_revision = current.revision;
                validation_transport::send(profile, slug, &materialized.target, dispatched).await
            };
            match response {
                Ok(response) => {
                    status_class = Some(response.status / 100);
                    let outcome = profile.classify_response(&response);
                    let reason = validator_profiles::outcome_code(&outcome).to_string();
                    (outcome, reason)
                }
                Err(validation_transport::TransportError::NodeUpgradeRequired) => (
                    ValidationOutcome::Unsupported,
                    "node_agent_upgrade_required".to_string(),
                ),
                Err(validation_transport::TransportError::Configuration) => (
                    ValidationOutcome::ConfigurationError,
                    "configuration_error".to_string(),
                ),
                Err(validation_transport::TransportError::BodyTooLarge) => (
                    ValidationOutcome::TransportUnknown,
                    "response_too_large".to_string(),
                ),
                Err(validation_transport::TransportError::Unavailable) => (
                    ValidationOutcome::TransportUnknown,
                    "transport_unknown".to_string(),
                ),
            }
        }
    } else {
        (ValidationOutcome::Unsupported, "unsupported".to_string())
    };
    let current = snapshot(state, caller, &record.user_service_id).await?;
    if current.digest != record.execution_authority_digest
        || current.revision != record.credential_revision
        || current.node_credential != record.node_credential
    {
        return Err(AttemptFailure::Superseded);
    }
    if let ValidationOutcome::RateLimited {
        retry_after: Some(retry_after),
    } = &outcome
        && let Some(cooldown) = admission.cooldown.as_ref()
    {
        // Cap arithmetic at chrono's representable range, never shorten a
        // provider cooldown to the display freshness window.
        let duration = (*retry_after)
            .max(MIN_PROBE_INTERVAL)
            .min(Duration::from_secs(i32::MAX as u64));
        if !matches!(
            extend_cooldown(&state.db, cooldown, duration).await,
            Ok(true)
        ) {
            return Err(AttemptFailure::LeaseLost);
        }
    }
    record.checked_at = Utc::now();
    record.valid_until = if status_class.is_some()
        && record
            .node_credential
            .as_ref()
            .is_none_or(|node| node.revision.is_some())
        && !matches!(
            outcome,
            ValidationOutcome::TransportUnknown | ValidationOutcome::Unsupported
        ) {
        record.checked_at + chrono::Duration::seconds(DISPLAY_WINDOW_SECS)
    } else {
        record.checked_at
    };
    record.outcome = outcome;
    record.reason_code = reason;
    record.completed = true;
    if !finish_observation(&state.db, &record).await? {
        return Ok(());
    }
    audit_observation(state, caller, &record, status_class).await?;
    Ok(())
}

async fn audit_observation(
    state: &AppState,
    caller: &ValidationCaller,
    record: &ServiceValidationRecord,
    status_class: Option<u16>,
) -> AppResult<()> {
    let actor = super::audit_service::AuditActor {
        user_id: caller.user_id.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    super::audit_service::log_actor_event(state.db.clone(), &actor, "service_validation_checked", Some(serde_json::json!({
        "user_service_id": record.user_service_id, "owner_id": record.owner_id, "api_key_id": record.api_key_id,
        "attempt_id": record.attempt_id, "validator_id": record.validator_id, "validator_version": record.validator_version,
        "outcome": validator_profiles::outcome_code(&record.outcome), "http_status_class": status_class,
        "reason_code": record.reason_code,
    }))).await?;
    Ok(())
}

async fn finish_observation(
    db: &mongodb::Database,
    record: &ServiceValidationRecord,
) -> AppResult<bool> {
    let result = db
        .collection::<ServiceValidationRecord>(COLLECTION_NAME)
        .replace_one(
            doc! { "_id": &record.id, "attempt_id": &record.attempt_id, "completed": false },
            record,
        )
        .await?;
    Ok(result.modified_count == 1)
}

// Lease renewal must never shorten a provider's Retry-After window, including
// when the worker's keepalive races its final observation/audit write.
async fn extend_cooldown(
    db: &mongodb::Database,
    token: &LeaseToken,
    duration: Duration,
) -> AppResult<bool> {
    let millis = duration.as_millis().min(i64::MAX as u128) as i64;
    let result = db
        .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
        .update_one(
            doc! { "_id": &token.name, "lease_id": &token.lease_id,
            "holder.instance_id": &token.holder.instance_id,
            "holder.generation_id": &token.holder.generation_id,
            "$expr": { "$gt": ["$expires_at", "$$NOW"] } },
            vec![doc! { "$set": { "updated_at": "$$NOW", "expires_at": {
                "$max": ["$expires_at", { "$add": ["$$NOW", millis] }]
            } } }],
        )
        .await?;
    Ok(result.matched_count == 1)
}

#[cfg(test)]
#[path = "service_validation_tests.rs"]
mod tests;
