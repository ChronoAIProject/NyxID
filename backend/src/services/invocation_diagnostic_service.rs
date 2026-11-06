//! Bounded secondary evidence for a failed invocation, never a replacement error.
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::{
    config::AppConfig,
    errors::{AppError, AppResult},
    models::{
        invocation_diagnostic::{COLLECTION_NAME, InvocationDiagnostic, InvocationDiagnosticState},
        user_endpoint::UserEndpoint,
        user_service::UserService,
    },
    mw::auth::AuthMethod,
    services::{
        coordination_service::TokenBucketStore,
        feature_flag_service,
        monitoring_service::{self, CheckResult, CheckStatus, Client, ProbeResult, Source},
        org_service::{self, OwnerAccess},
        user_service_service,
    },
};

/// Includes feature/ACL reads, cluster admission, both upstream requests and body reads.
/// The probe path includes caller authorization and one live downstream check.
/// Keep it bounded, but allow a normal remote provider round trip to finish.
pub(crate) const BUDGET: Duration = Duration::from_secs(3);
pub(crate) const HANDLE_TTL_SECS: i64 = 60;
pub(crate) const ISSUE_BUDGET: Duration = Duration::from_millis(250);
const DUPLICATE_ISSUE_WINDOW: Duration = Duration::from_secs(10);
const MAX_RECENT_ISSUES: usize = 4096;
const HANDLE_PREFIX: &str = "nyx_diag_";
static IN_FLIGHT: Semaphore = Semaphore::const_new(32);
static RECENT_ISSUES: LazyLock<Mutex<HashMap<IssueFingerprint, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Hash, PartialEq, Eq)]
struct IssueFingerprint {
    actor_id: String,
    auth_method: &'static str,
    api_key_id: Option<String>,
    client_id: Option<String>,
    exact_user_service_id: String,
}

struct IssueReservation {
    fingerprint: Option<IssueFingerprint>,
}

impl IssueReservation {
    fn commit(mut self) {
        self.fingerprint = None;
    }
}

impl Drop for IssueReservation {
    fn drop(&mut self) {
        let Some(fingerprint) = self.fingerprint.take() else {
            return;
        };
        RECENT_ISSUES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&fingerprint);
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    Caller,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    OperationSpecificLikely,
    DependencyUnavailable,
    HealthAssertionFailed,
    BrokerError,
    NodeOffline,
    Timeout,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    NonDelegable,
    NoHealthPath,
    RateLimited,
    Timeout,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Diagnostic {
    #[serde(skip, default)]
    pub(crate) attempted: bool,
    pub observation: Observation,
    pub assessment: Assessment,
    pub outcome: Outcome,
}

#[derive(Clone, Serialize)]
pub(crate) struct IssuedHandle {
    pub handle: String,
    pub expires_in: i64,
}

impl IssuedHandle {
    pub(crate) fn content(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "text",
            "text": serde_json::json!({
                "nyxid_invocation_diagnostic": self,
                "instruction": "Optional secondary evidence. Redeem this handle once with the current NyxID identity. Preserve the original error and do not retry the failed operation automatically."
            }).to_string()
        })
    }
}

#[derive(Debug)]
pub(crate) enum RedeemResult {
    Completed {
        diagnostic: Diagnostic,
        exact_user_service_id: String,
        fresh: bool,
    },
    NotReady,
}

fn auth_method_name(method: &AuthMethod) -> &'static str {
    match method {
        AuthMethod::Session => "session",
        AuthMethod::AccessToken => "access_token",
        AuthMethod::Relay => "relay",
        AuthMethod::ApiKey => "api_key",
        AuthMethod::ServiceAccount => "service_account",
        AuthMethod::Delegated => "delegated",
    }
}

pub(crate) async fn issue(
    db: &Database,
    config: &AppConfig,
    actor_id: &str,
    auth_method: &AuthMethod,
    api_key_id: Option<&str>,
    client_id: Option<&str>,
    exact_user_service_id: &str,
) -> AppResult<Option<IssuedHandle>> {
    monitoring_service::configured_base_url(config)?;
    let Some(reservation) = reserve_issue(
        actor_id,
        auth_method,
        api_key_id,
        client_id,
        exact_user_service_id,
    ) else {
        return Ok(None);
    };
    let raw = format!(
        "{HANDLE_PREFIX}{}",
        crate::crypto::token::generate_random_token()
    );
    let now = Utc::now();
    let row = InvocationDiagnostic {
        id: uuid::Uuid::new_v4().to_string(),
        handle_hash: crate::crypto::token::hash_token(&raw),
        actor_user_id: actor_id.to_owned(),
        original_auth_method: auth_method_name(auth_method).to_owned(),
        original_api_key_id: api_key_id.map(str::to_owned),
        original_client_id: client_id.map(str::to_owned),
        exact_user_service_id: exact_user_service_id.to_owned(),
        state: InvocationDiagnosticState::Pending,
        diagnostic: None,
        claim_id: None,
        claimed_at: None,
        completed_at: None,
        created_at: now,
        expires_at: now + chrono::Duration::seconds(HANDLE_TTL_SECS),
    };
    tokio::time::timeout(
        ISSUE_BUDGET,
        db.collection::<InvocationDiagnostic>(COLLECTION_NAME)
            .insert_one(row),
    )
    .await
    .map_err(|_| AppError::MonitoringUnavailable)??;
    reservation.commit();
    Ok(Some(IssuedHandle {
        handle: raw,
        expires_in: HANDLE_TTL_SECS,
    }))
}

fn reserve_issue(
    actor_id: &str,
    auth_method: &AuthMethod,
    api_key_id: Option<&str>,
    client_id: Option<&str>,
    exact_user_service_id: &str,
) -> Option<IssueReservation> {
    let fingerprint = IssueFingerprint {
        actor_id: actor_id.to_owned(),
        auth_method: auth_method_name(auth_method),
        api_key_id: api_key_id.map(str::to_owned),
        client_id: client_id.map(str::to_owned),
        exact_user_service_id: exact_user_service_id.to_owned(),
    };
    let now = Instant::now();
    let mut recent = RECENT_ISSUES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(previous) = recent.get(&fingerprint) {
        if now.duration_since(*previous) < DUPLICATE_ISSUE_WINDOW {
            return None;
        }
        recent.remove(&fingerprint);
    }
    if recent.len() >= MAX_RECENT_ISSUES {
        recent.retain(|_, issued_at| now.duration_since(*issued_at) < DUPLICATE_ISSUE_WINDOW);
        if recent.len() >= MAX_RECENT_ISSUES {
            return None;
        }
    }
    recent.insert(fingerprint.clone(), now);
    Some(IssueReservation {
        fingerprint: Some(fingerprint),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn redeem(
    db: &Database,
    config: &AppConfig,
    actor_id: &str,
    auth_method: &AuthMethod,
    api_key_id: Option<&str>,
    client_id: Option<&str>,
    allow_all_services: bool,
    allowed_service_ids: &[String],
    handle: &str,
    caller_token: Option<&str>,
) -> AppResult<RedeemResult> {
    if !valid_handle(handle) {
        return Err(not_found());
    }
    let rows = db.collection::<InvocationDiagnostic>(COLLECTION_NAME);
    let identity = identity_filter(actor_id, auth_method, api_key_id, client_id, handle);
    let Some(row) = rows.find_one(identity.clone()).await? else {
        return Err(not_found());
    };
    if row.expires_at <= Utc::now() {
        return Err(not_found());
    }
    if !allow_all_services && !allowed_service_ids.contains(&row.exact_user_service_id) {
        return Err(not_found());
    }
    let Some(service) = authorized_service(db, actor_id, &row.exact_user_service_id).await? else {
        return Err(not_found());
    };
    if let Some(result) = completed(&row)? {
        return Ok(RedeemResult::Completed {
            diagnostic: result,
            exact_user_service_id: row.exact_user_service_id,
            fresh: false,
        });
    }

    let delegable = matches!(auth_method, AuthMethod::AccessToken)
        && client_id != Some("chrono-probe")
        && caller_token.is_some();
    if delegable && has_active_capacity(db, actor_id, &service).await? {
        return Ok(RedeemResult::NotReady);
    }

    let claim_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let mut claim_filter = identity;
    claim_filter.insert("state", "pending");
    let claimed = rows
        .find_one_and_update(
            claim_filter,
            doc! {"$set":{
                "state":"running",
                "claim_id":&claim_id,
                "claimed_at":bson::DateTime::from_chrono(now),
            }},
        )
        .return_document(ReturnDocument::After)
        .await?;
    let Some(claimed) = claimed else {
        let Some(current) = rows
            .find_one(identity_filter(
                actor_id,
                auth_method,
                api_key_id,
                client_id,
                handle,
            ))
            .await?
        else {
            return Err(not_found());
        };
        if let Some(result) = completed(&current)? {
            return Ok(RedeemResult::Completed {
                diagnostic: result,
                exact_user_service_id: current.exact_user_service_id,
                fresh: false,
            });
        }
        return Ok(RedeemResult::NotReady);
    };

    let diagnostic = if delegable {
        diagnose(
            db,
            config,
            actor_id,
            &claimed.exact_user_service_id,
            caller_token,
        )
        .await
        .unwrap_or_else(|| Diagnostic::unknown(Outcome::Unavailable))
    } else {
        Diagnostic::unknown(Outcome::NonDelegable)
    };
    let encoded =
        bson::to_document(&diagnostic).map_err(|error| AppError::Internal(error.to_string()))?;
    let completed_at = Utc::now();
    let update = rows
        .update_one(
            doc! {"_id":&claimed.id,"state":"running","claim_id":&claim_id},
            doc! {"$set":{
                "state":"completed",
                "diagnostic":encoded,
                "completed_at":bson::DateTime::from_chrono(completed_at),
                "claim_id":bson::Bson::Null,
            }},
        )
        .await?;
    if update.matched_count == 0 {
        return Ok(RedeemResult::NotReady);
    }
    Ok(RedeemResult::Completed {
        diagnostic,
        exact_user_service_id: claimed.exact_user_service_id,
        fresh: true,
    })
}

fn valid_handle(handle: &str) -> bool {
    handle.len() == HANDLE_PREFIX.len() + 64
        && handle.starts_with(HANDLE_PREFIX)
        && handle[HANDLE_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn identity_filter(
    actor_id: &str,
    auth_method: &AuthMethod,
    api_key_id: Option<&str>,
    client_id: Option<&str>,
    handle: &str,
) -> bson::Document {
    doc! {
        "handle_hash":crate::crypto::token::hash_token(handle),
        "actor_user_id":actor_id,
        "original_auth_method":auth_method_name(auth_method),
        "original_api_key_id":api_key_id,
        "original_client_id":client_id,
        "expires_at":{"$gt":bson::DateTime::now()},
    }
}

fn completed(row: &InvocationDiagnostic) -> AppResult<Option<Diagnostic>> {
    if row.state != InvocationDiagnosticState::Completed {
        return Ok(None);
    }
    let document = row
        .diagnostic
        .clone()
        .ok_or_else(|| AppError::Internal("Completed diagnostic is missing its result".into()))?;
    bson::from_document(document)
        .map(Some)
        .map_err(|error| AppError::Internal(error.to_string()))
}

fn not_found() -> AppError {
    AppError::NotFound("Invocation diagnostic not found".into())
}

async fn authorized_service(
    db: &Database,
    actor_id: &str,
    exact_id: &str,
) -> AppResult<Option<UserService>> {
    let Some(service) = super::service_history::collection::<UserService>(
        db,
        crate::models::user_service::COLLECTION_NAME,
    )
    .find_one(doc! {"_id":exact_id})
    .await?
    else {
        return Ok(None);
    };
    if !service.is_active || service.deleted_at.is_some() || service.service_type != "http" {
        return Ok(None);
    }
    let access = org_service::resolve_owner_access(db, actor_id, &service.user_id).await?;
    let usable = match &access {
        OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. } => true,
        OwnerAccess::AsOrgMember { role, .. } => {
            user_service_service::role_can_proxy_service(*role, &service)
        }
        OwnerAccess::Forbidden => false,
    };
    Ok((usable && access.allows_resource(exact_id)).then_some(service))
}

async fn has_active_capacity(
    db: &Database,
    actor_id: &str,
    service: &UserService,
) -> AppResult<bool> {
    let Some(catalog_service_id) = service.catalog_service_id.as_deref() else {
        return Ok(false);
    };
    Ok(db
        .collection::<bson::Document>(crate::models::service_concurrency::COLLECTION_NAME)
        .find_one(doc! {
            "service_id":catalog_service_id,
            "person_id":actor_id,
            "leases":{"$elemMatch":{"expires_at":{"$gt":bson::DateTime::now()}}},
        })
        .await?
        .is_some())
}

impl Diagnostic {
    fn unknown(outcome: Outcome) -> Self {
        Self {
            attempted: false,
            observation: Observation::Unknown,
            assessment: Assessment::Unknown,
            outcome,
        }
    }
}

/// Only provider/transport failures qualify. Local validation, authorization,
/// approval, capacity, billing and caller cancellation never trigger a probe.
pub(crate) fn eligible_error(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Internal(_)
            | AppError::NodeOffline(_)
            | AppError::NodeNotFound(_)
            | AppError::NodeProxyTimeout
            | AppError::NodeCredentialMissing(_)
            | AppError::NodeHttpSignatureUnsupported
            | AppError::WsProxyDownstream(_)
            | AppError::ServicePoolAttemptsExhausted { .. }
            | AppError::ServicePoolDeadlineExceeded { .. }
    )
}

/// The ID must come from execution resolution, never a slug/catalog lookup.
/// The original credential is borrowed for this invocation only; no tasks own it.
pub(crate) fn diagnose<'a>(
    db: &'a Database,
    config: &'a AppConfig,
    actor_id: &'a str,
    exact_id: &'a str,
    caller_token: Option<&'a str>,
) -> futures::future::BoxFuture<'a, Option<Diagnostic>> {
    // Seal the shared admission future so each large proxy/MCP handler does
    // not repeatedly expand its feature, ACL and MongoDB futures for Send.
    Box::pin(diagnose_bounded(
        db,
        config,
        actor_id,
        exact_id,
        caller_token,
    ))
}

async fn diagnose_bounded(
    db: &Database,
    config: &AppConfig,
    actor_id: &str,
    exact_id: &str,
    caller_token: Option<&str>,
) -> Option<Diagnostic> {
    monitoring_service::configured_base_url(config).ok()?;
    let _permit = match IN_FLIGHT.try_acquire() {
        Ok(permit) => permit,
        Err(_) => return None,
    };
    let attempted = AtomicBool::new(false);
    let result = match tokio::time::timeout(
        BUDGET,
        diagnose_inner(db, config, actor_id, exact_id, caller_token, &attempted),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Some(Diagnostic::unknown(Outcome::Unavailable)),
        Err(_) => Some(Diagnostic::unknown(Outcome::Timeout)),
    };
    result.map(|mut result| {
        result.attempted = attempted.load(Ordering::Relaxed);
        result
    })
}

async fn diagnose_inner(
    db: &Database,
    config: &AppConfig,
    actor_id: &str,
    exact_id: &str,
    caller_token: Option<&str>,
    attempted: &AtomicBool,
) -> AppResult<Option<Diagnostic>> {
    if !feature_flag_service::personal_flag_enabled(
        db,
        actor_id,
        feature_flag_service::SERVICE_MONITORING_FLAG_KEY,
    )
    .await?
    {
        return Ok(None);
    }
    let Some(service) = super::service_history::collection::<UserService>(
        db,
        crate::models::user_service::COLLECTION_NAME,
    )
    .find_one(doc! {"_id": exact_id})
    .await?
    else {
        return Ok(None);
    };
    if !service.is_active || service.deleted_at.is_some() || service.service_type != "http" {
        return Ok(None);
    }
    let access = org_service::resolve_owner_access(db, actor_id, &service.user_id).await?;
    let usable = match &access {
        OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. } => true,
        OwnerAccess::AsOrgMember { role, .. } => {
            user_service_service::role_can_proxy_service(*role, &service)
        }
        OwnerAccess::Forbidden => false,
    };
    if !usable || !access.allows_resource(exact_id) {
        return Ok(None);
    }
    // No requests to the monitoring service itself, including user-named aliases.
    let Some(endpoint) = super::service_history::collection::<UserEndpoint>(
        db,
        crate::models::user_endpoint::COLLECTION_NAME,
    )
    .find_one(doc! {"_id": &service.endpoint_id})
    .await?
    else {
        return Ok(None);
    };
    if same_origin(
        &endpoint.url,
        config.chrono_probe_url.as_deref().unwrap_or_default(),
    ) {
        return Ok(None);
    }
    let Some(token) = caller_token else {
        // There is deliberately no system-key fallback. The current status API
        // does not bind an observation to this credential, so it cannot prove
        // caller reachability and is not imported as secondary evidence either.
        return Ok(Some(Diagnostic::unknown(Outcome::NonDelegable)));
    };
    let admission = TokenBucketStore::admit_over_period(
        db,
        "invocation-diagnostic",
        &format!("{actor_id}:{exact_id}"),
        1,
        Duration::from_secs(10),
        1,
    )
    .await?;
    if !admission.allowed {
        return Ok(Some(Diagnostic::unknown(Outcome::RateLimited)));
    }
    let client = Client::for_caller(config, token, exact_id, BUDGET)?;
    // The caller probe endpoint can discover paths when none is known. Automatic
    // diagnostics only use an already configured/verified health recipe.
    attempted.store(true, Ordering::Relaxed);
    let status = client.status().await?;
    if !known_health_path(&status.health) {
        return Ok(Some(Diagnostic::unknown(Outcome::NoHealthPath)));
    }
    let result = client.probe(None).await?;
    if result.health.path != status.health.path {
        return Ok(Some(Diagnostic::unknown(Outcome::Unavailable)));
    }
    Ok(Some(project(&result)))
}

fn same_origin(left: &str, right: &str) -> bool {
    match (url::Url::parse(left), url::Url::parse(right)) {
        (Ok(left), Ok(right)) => left.origin() == right.origin(),
        _ => false,
    }
}

fn known_health_path(check: &CheckResult) -> bool {
    !check.path.is_empty()
        && !matches!(check.source, Source::None)
        && check.reason != "no_health_path"
}

fn recent(check: &CheckResult) -> bool {
    let now = Utc::now();
    check.checked_at.is_some_and(|at| {
        at >= now - chrono::Duration::seconds(60) && at <= now + chrono::Duration::seconds(5)
    })
}

fn project(result: &ProbeResult) -> Diagnostic {
    let health = &result.health;
    if !recent(health) {
        return Diagnostic::unknown(Outcome::Unavailable);
    }
    let assessment = match health.reason.as_str() {
        "broker_error" => Assessment::BrokerError,
        "node_offline" => Assessment::NodeOffline,
        "timeout" => Assessment::Timeout,
        "no_health_path" => return Diagnostic::unknown(Outcome::NoHealthPath),
        _ => match health.status {
            CheckStatus::Healthy if health.reachable == Some(true) => {
                match result.readiness.as_ref() {
                    Some(check)
                        if check.configured == Some(false)
                            || matches!(check.status, CheckStatus::NotConfigured) =>
                    {
                        Assessment::OperationSpecificLikely
                    }
                    Some(check)
                        if recent(check)
                            && matches!(check.status, CheckStatus::NotReady)
                            && check.reachable == Some(true) =>
                    {
                        Assessment::DependencyUnavailable
                    }
                    None => Assessment::OperationSpecificLikely,
                    Some(check)
                        if recent(check)
                            && matches!(check.status, CheckStatus::Ready)
                            && check.reachable == Some(true) =>
                    {
                        Assessment::OperationSpecificLikely
                    }
                    Some(_) => Assessment::Unknown,
                }
            }
            CheckStatus::Unhealthy if health.reachable == Some(true) => {
                Assessment::HealthAssertionFailed
            }
            _ => Assessment::Unknown,
        },
    };
    Diagnostic {
        attempted: true,
        observation: if assessment == Assessment::Unknown {
            Observation::Unknown
        } else {
            Observation::Caller
        },
        assessment,
        outcome: Outcome::Completed,
    }
}

#[cfg(test)]
pub(crate) mod tests;
