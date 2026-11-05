//! Bounded secondary evidence for a failed invocation, never a replacement error.
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use chrono::Utc;
use mongodb::{Database, bson::doc};
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::{
    config::AppConfig,
    errors::{AppError, AppResult},
    models::{user_endpoint::UserEndpoint, user_service::UserService},
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
static IN_FLIGHT: Semaphore = Semaphore::const_new(32);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    Caller,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    NonDelegable,
    NoHealthPath,
    RateLimited,
    Timeout,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    #[serde(skip)]
    pub(crate) attempted: bool,
    pub observation: Observation,
    pub assessment: Assessment,
    pub outcome: Outcome,
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

    pub(crate) fn content(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "text",
            "text": serde_json::json!({
                "nyxid_service_diagnostic": self,
                "interpretation": "Secondary health evidence only. The original invocation error remains authoritative. A healthy probe does not establish that the failed operation is valid or authorized; unknown provides no attribution."
            }).to_string()
        })
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
    if !service.is_active || service.service_type != "http" {
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
