//! Server-authoritative assistant capability readiness projection.
//!
//! This is a CQRS read model over existing execution authorities. It does not
//! grant access and it never decrypts credentials. A capability is available
//! only when connection, provider-scope, ownership, and canonical MCP
//! callability evidence all agree.

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::errors::AppResult;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service::UserService;
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::services::mcp_service::{self, McpToolSource};
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::user_service_service::{self, CredentialSource};

pub const ASSISTANT_READINESS_REVISION: &str = "nyxid-assistant-readiness.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityStatus {
    Available,
    Missing,
    CannotUse,
    CannotCheck,
}

impl CapabilityStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Missing => "missing",
            Self::CannotUse => "cannot_use",
            Self::CannotCheck => "cannot_check",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    NotConnected,
    Connecting,
    Verifying,
    Connected,
    Expired,
    Revoked,
    Unknown,
}

impl ConnectionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotConnected => "not_connected",
            Self::Connecting => "connecting",
            Self::Verifying => "verifying",
            Self::Connected => "connected",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantState {
    NotRequired,
    Granted,
    Partial,
    Missing,
    Expired,
    Revoked,
    Unknown,
}

impl GrantState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Granted => "granted",
            Self::Partial => "partial",
            Self::Missing => "missing",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasonCode {
    CapabilityEvidenceUnavailable,
    ServiceNotConnected,
    AccessDenied,
    ConnectionInProgress,
    ConnectionVerificationPending,
    ConnectionExpired,
    ConnectionRevoked,
    ConnectionStateUnknown,
    GrantPartial,
    GrantMissing,
    GrantExpired,
    GrantRevoked,
    GrantUnknown,
    ExecutionUnavailable,
    ExecutionStateUnknown,
}

impl ReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CapabilityEvidenceUnavailable => "capability_evidence_unavailable",
            Self::ServiceNotConnected => "service_not_connected",
            Self::AccessDenied => "access_denied",
            Self::ConnectionInProgress => "connection_in_progress",
            Self::ConnectionVerificationPending => "connection_verification_pending",
            Self::ConnectionExpired => "connection_expired",
            Self::ConnectionRevoked => "connection_revoked",
            Self::ConnectionStateUnknown => "connection_state_unknown",
            Self::GrantPartial => "grant_partial",
            Self::GrantMissing => "grant_missing",
            Self::GrantExpired => "grant_expired",
            Self::GrantRevoked => "grant_revoked",
            Self::GrantUnknown => "grant_unknown",
            Self::ExecutionUnavailable => "execution_unavailable",
            Self::ExecutionStateUnknown => "execution_state_unknown",
        }
    }
}

pub struct ReadinessSnapshot {
    pub revision: &'static str,
    pub evaluated_at: DateTime<Utc>,
    pub capabilities: Vec<CapabilityReadiness>,
}

pub struct CapabilityReadiness {
    pub capability_id: &'static str,
    pub label: &'static str,
    pub required: bool,
    pub status: CapabilityStatus,
    pub connection_state: ConnectionState,
    pub grant_state: GrantState,
    pub requested_scopes: &'static [&'static str],
    pub management_path: &'static str,
    pub reason_code: Option<ReasonCode>,
}

struct CapabilityProfile {
    capability_id: &'static str,
    label: &'static str,
    required: bool,
    requested_scopes: &'static [&'static str],
    management_path: &'static str,
}

// The registry is deliberately closed and versioned with the response. Adding
// or changing a row requires a readiness revision and a consumer fixture update.
const CAPABILITY_PROFILES: &[CapabilityProfile] = &[CapabilityProfile {
    capability_id: "api-github",
    label: "GitHub",
    required: false,
    requested_scopes: &["repo"],
    management_path: "/keys",
}];

#[derive(Clone, Copy, Debug)]
struct CapabilityEvidence {
    catalog_available: bool,
    access_allowed: bool,
    connection_state: ConnectionState,
    grant_state: GrantState,
    executable: Option<bool>,
}

impl CapabilityEvidence {
    fn unavailable() -> Self {
        Self {
            catalog_available: false,
            access_allowed: true,
            connection_state: ConnectionState::Unknown,
            grant_state: GrantState::Unknown,
            executable: None,
        }
    }

    fn not_connected() -> Self {
        Self {
            catalog_available: true,
            access_allowed: true,
            connection_state: ConnectionState::NotConnected,
            grant_state: GrantState::Missing,
            executable: Some(false),
        }
    }
}

struct SelectedUserService {
    service: UserService,
    effective_owner_id: String,
    access_allowed: bool,
}

/// Evaluate the fixed assistant capability registry for the verified user.
///
/// Errors from an individual evidence source become `cannot_check` for that
/// capability. This keeps the response total without converting absence or an
/// authority outage into a false `missing` result.
pub async fn evaluate_readiness(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    evaluated_at: DateTime<Utc>,
) -> ReadinessSnapshot {
    let mut capabilities = Vec::with_capacity(CAPABILITY_PROFILES.len());
    for profile in CAPABILITY_PROFILES {
        let evidence =
            match load_capability_evidence(db, node_ws_manager, user_id, profile, evaluated_at)
                .await
            {
                Ok(evidence) => evidence,
                Err(error) => {
                    tracing::warn!(
                        capability_id = profile.capability_id,
                        error = %error,
                        "Assistant readiness evidence could not be evaluated"
                    );
                    CapabilityEvidence::unavailable()
                }
            };
        capabilities.push(evaluate_profile(profile, evidence));
    }

    ReadinessSnapshot {
        revision: ASSISTANT_READINESS_REVISION,
        evaluated_at,
        capabilities,
    }
}

async fn load_capability_evidence(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    profile: &CapabilityProfile,
    evaluated_at: DateTime<Utc>,
) -> AppResult<CapabilityEvidence> {
    let Some(catalog_service) = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! {
            "slug": profile.capability_id,
            "is_active": true,
            "service_type": "http",
        })
        .await?
    else {
        return Ok(CapabilityEvidence::unavailable());
    };

    let visible_services =
        user_service_service::list_user_services_with_sources(db, user_id).await?;
    let mut personal = Vec::new();
    let mut org = Vec::new();
    for item in visible_services {
        if item.service.catalog_service_id.as_deref() != Some(catalog_service.id.as_str()) {
            continue;
        }
        match item.source {
            CredentialSource::Personal => personal.push(SelectedUserService {
                effective_owner_id: user_id.to_string(),
                service: item.service,
                access_allowed: true,
            }),
            CredentialSource::Org {
                org_user_id,
                allowed,
                ..
            } => org.push(SelectedUserService {
                effective_owner_id: org_user_id,
                service: item.service,
                access_allowed: allowed,
            }),
        }
    }

    if let Some(selected) = select_user_service(personal, profile.capability_id) {
        return unified_service_evidence(
            db,
            node_ws_manager,
            user_id,
            profile,
            selected,
            evaluated_at,
        )
        .await;
    }

    // Runtime proxy precedence keeps legacy personal state ahead of org
    // fallback. Preserve that order in the read model during migration.
    if let Some(evidence) = legacy_personal_evidence(
        db,
        node_ws_manager,
        user_id,
        profile,
        &catalog_service,
        evaluated_at,
    )
    .await?
    {
        return Ok(evidence);
    }

    if let Some(selected) = select_user_service(org, profile.capability_id) {
        return unified_service_evidence(
            db,
            node_ws_manager,
            user_id,
            profile,
            selected,
            evaluated_at,
        )
        .await;
    }

    Ok(CapabilityEvidence::not_connected())
}

fn select_user_service(
    mut candidates: Vec<SelectedUserService>,
    capability_id: &str,
) -> Option<SelectedUserService> {
    let exact = candidates
        .iter()
        .position(|candidate| candidate.service.slug == capability_id);
    exact
        .map(|index| candidates.remove(index))
        .or_else(|| candidates.into_iter().next())
}

async fn unified_service_evidence(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    actor_user_id: &str,
    profile: &CapabilityProfile,
    selected: SelectedUserService,
    evaluated_at: DateTime<Utc>,
) -> AppResult<CapabilityEvidence> {
    let executable = if selected.access_allowed {
        execution_state_for_user_service(db, node_ws_manager, actor_user_id, &selected.service.id)
            .await?
    } else {
        Some(false)
    };

    let Some(api_key_id) = selected.service.api_key_id.as_deref() else {
        let connection_state = if selected.service.auth_method == "none" {
            ConnectionState::Connected
        } else if selected
            .service
            .node_id
            .as_deref()
            .is_some_and(|node_id| !node_id.is_empty())
            && executable == Some(true)
        {
            ConnectionState::Connected
        } else {
            ConnectionState::Verifying
        };
        let grant_state = if profile.requested_scopes.is_empty() {
            GrantState::NotRequired
        } else {
            GrantState::Unknown
        };
        return Ok(CapabilityEvidence {
            catalog_available: true,
            access_allowed: selected.access_allowed,
            connection_state,
            grant_state,
            executable,
        });
    };

    let key = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! {
            "_id": api_key_id,
            "user_id": &selected.effective_owner_id,
        })
        .await?;
    let Some(key) = key else {
        return Ok(CapabilityEvidence {
            catalog_available: true,
            access_allowed: selected.access_allowed,
            connection_state: ConnectionState::Verifying,
            grant_state: GrantState::Unknown,
            executable: Some(false),
        });
    };

    let connection_state =
        connection_state_for_key(&key, &selected.service, executable, evaluated_at);
    let grant_state = grant_state_for_key(&key, profile.requested_scopes, evaluated_at);
    Ok(CapabilityEvidence {
        catalog_available: true,
        access_allowed: selected.access_allowed,
        connection_state,
        grant_state,
        executable,
    })
}

async fn execution_state_for_user_service(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    user_service_id: &str,
) -> AppResult<Option<bool>> {
    let tools = mcp_service::load_user_tools_all(db, node_ws_manager, user_id).await?;
    Ok(Some(tools.iter().any(|tool| {
        tool.executable
            && matches!(
                &tool.source,
                McpToolSource::UserManaged { user_service_id: id, .. } if id == user_service_id
            )
    })))
}

async fn execution_state_for_platform_service(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    downstream_service_id: &str,
) -> AppResult<Option<bool>> {
    let tools = mcp_service::load_user_tools_all(db, node_ws_manager, user_id).await?;
    Ok(Some(tools.iter().any(|tool| {
        tool.executable
            && matches!(
                &tool.source,
                McpToolSource::Platform { downstream_service_id: id } if id == downstream_service_id
            )
    })))
}

async fn legacy_personal_evidence(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    profile: &CapabilityProfile,
    catalog_service: &DownstreamService,
    evaluated_at: DateTime<Utc>,
) -> AppResult<Option<CapabilityEvidence>> {
    let connection = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": &catalog_service.id,
        })
        .sort(doc! { "updated_at": -1 })
        .await?;

    // An explicit legacy disconnect blocks the provider-token fallback in the
    // canonical MCP loader and therefore remains a positive missing result.
    if connection.as_ref().is_some_and(|row| !row.is_active) {
        return Ok(Some(CapabilityEvidence::not_connected()));
    }

    let provider_tokens: Vec<UserProviderToken> =
        if let Some(provider_config_id) = catalog_service.provider_config_id.as_deref() {
            db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
                .find(doc! {
                    "user_id": user_id,
                    "provider_config_id": provider_config_id,
                })
                .sort(doc! { "updated_at": -1 })
                .limit(2)
                .await?
                .try_collect()
                .await?
        } else {
            Vec::new()
        };

    // Multiple legacy provider rows have no deterministic connection identity.
    // New multi-connection rows are represented by UserService and never reach
    // this branch, so ambiguity here must fail closed.
    if provider_tokens.len() > 1 {
        return Ok(Some(CapabilityEvidence {
            catalog_available: true,
            access_allowed: true,
            connection_state: ConnectionState::Unknown,
            grant_state: GrantState::Unknown,
            executable: None,
        }));
    }

    if let Some(token) = provider_tokens.first() {
        let executable =
            execution_state_for_platform_service(db, node_ws_manager, user_id, &catalog_service.id)
                .await?;
        return Ok(Some(CapabilityEvidence {
            catalog_available: true,
            access_allowed: true,
            connection_state: connection_state_for_provider_token(token, evaluated_at),
            grant_state: grant_state_for_provider_token(
                token,
                profile.requested_scopes,
                evaluated_at,
            ),
            executable,
        }));
    }

    let Some(connection) = connection else {
        return Ok(None);
    };
    let connected = connection
        .credential_encrypted
        .as_ref()
        .is_some_and(|credential| !credential.is_empty());
    let executable =
        execution_state_for_platform_service(db, node_ws_manager, user_id, &catalog_service.id)
            .await?;
    Ok(Some(CapabilityEvidence {
        catalog_available: true,
        access_allowed: true,
        connection_state: if connected {
            ConnectionState::Connected
        } else {
            ConnectionState::Verifying
        },
        grant_state: if profile.requested_scopes.is_empty() {
            GrantState::NotRequired
        } else {
            GrantState::Unknown
        },
        executable,
    }))
}

fn connection_state_for_key(
    key: &UserApiKey,
    service: &UserService,
    executable: Option<bool>,
    evaluated_at: DateTime<Utc>,
) -> ConnectionState {
    if key
        .expires_at
        .is_some_and(|expires_at| expires_at <= evaluated_at)
    {
        return ConnectionState::Expired;
    }
    match key.status.as_str() {
        "pending_auth" => ConnectionState::Connecting,
        "expired" => ConnectionState::Expired,
        "revoked" => ConnectionState::Revoked,
        "active" => {
            let node_routed = service
                .node_id
                .as_deref()
                .is_some_and(|node_id| !node_id.is_empty());
            if node_routed && executable == Some(true) {
                ConnectionState::Connected
            } else if crate::services::user_api_key_service::has_server_credential(key) {
                ConnectionState::Connected
            } else {
                ConnectionState::Verifying
            }
        }
        "failed" | "refresh_failed" => ConnectionState::Unknown,
        _ => ConnectionState::Unknown,
    }
}

fn connection_state_for_provider_token(
    token: &UserProviderToken,
    evaluated_at: DateTime<Utc>,
) -> ConnectionState {
    if token
        .expires_at
        .is_some_and(|expires_at| expires_at <= evaluated_at)
    {
        return ConnectionState::Expired;
    }
    match token.status.as_str() {
        "active" => {
            let has_credential = token
                .access_token_encrypted
                .as_ref()
                .or(token.api_key_encrypted.as_ref())
                .is_some_and(|credential| !credential.is_empty());
            if has_credential {
                ConnectionState::Connected
            } else {
                ConnectionState::Verifying
            }
        }
        "pending_auth" => ConnectionState::Connecting,
        "expired" => ConnectionState::Expired,
        "revoked" => ConnectionState::Revoked,
        "refresh_failed" | "failed" => ConnectionState::Unknown,
        _ => ConnectionState::Unknown,
    }
}

fn grant_state_for_key(
    key: &UserApiKey,
    requested_scopes: &[&str],
    evaluated_at: DateTime<Utc>,
) -> GrantState {
    grant_state_from_status_and_scopes(
        &key.status,
        key.expires_at,
        key.token_scopes.as_deref(),
        requested_scopes,
        evaluated_at,
    )
}

fn grant_state_for_provider_token(
    token: &UserProviderToken,
    requested_scopes: &[&str],
    evaluated_at: DateTime<Utc>,
) -> GrantState {
    grant_state_from_status_and_scopes(
        &token.status,
        token.expires_at,
        token.token_scopes.as_deref(),
        requested_scopes,
        evaluated_at,
    )
}

fn grant_state_from_status_and_scopes(
    status: &str,
    expires_at: Option<DateTime<Utc>>,
    granted_scopes: Option<&str>,
    requested_scopes: &[&str],
    evaluated_at: DateTime<Utc>,
) -> GrantState {
    if requested_scopes.is_empty() {
        return GrantState::NotRequired;
    }
    if expires_at.is_some_and(|expires_at| expires_at <= evaluated_at) || status == "expired" {
        return GrantState::Expired;
    }
    if status == "revoked" {
        return GrantState::Revoked;
    }
    if status != "active" {
        return GrantState::Unknown;
    }
    let Some(granted_scopes) = granted_scopes.filter(|scopes| !scopes.trim().is_empty()) else {
        return GrantState::Unknown;
    };
    let granted: std::collections::HashSet<&str> = granted_scopes.split_whitespace().collect();
    let matched = requested_scopes
        .iter()
        .filter(|scope| granted.contains(**scope))
        .count();
    if matched == requested_scopes.len() {
        GrantState::Granted
    } else if matched == 0 {
        GrantState::Missing
    } else {
        GrantState::Partial
    }
}

fn evaluate_profile(
    profile: &CapabilityProfile,
    evidence: CapabilityEvidence,
) -> CapabilityReadiness {
    let (status, reason_code) = derive_status(evidence);
    CapabilityReadiness {
        capability_id: profile.capability_id,
        label: profile.label,
        required: profile.required,
        status,
        connection_state: evidence.connection_state,
        grant_state: evidence.grant_state,
        requested_scopes: profile.requested_scopes,
        management_path: profile.management_path,
        reason_code,
    }
}

fn derive_status(evidence: CapabilityEvidence) -> (CapabilityStatus, Option<ReasonCode>) {
    if !evidence.catalog_available {
        return (
            CapabilityStatus::CannotCheck,
            Some(ReasonCode::CapabilityEvidenceUnavailable),
        );
    }
    if evidence.connection_state == ConnectionState::NotConnected {
        return (
            CapabilityStatus::Missing,
            Some(ReasonCode::ServiceNotConnected),
        );
    }
    if !evidence.access_allowed {
        return (CapabilityStatus::CannotUse, Some(ReasonCode::AccessDenied));
    }
    match evidence.connection_state {
        ConnectionState::Connecting => {
            return (
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::ConnectionInProgress),
            );
        }
        ConnectionState::Verifying => {
            return (
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::ConnectionVerificationPending),
            );
        }
        ConnectionState::Expired => {
            return (
                CapabilityStatus::CannotUse,
                Some(ReasonCode::ConnectionExpired),
            );
        }
        ConnectionState::Revoked => {
            return (
                CapabilityStatus::CannotUse,
                Some(ReasonCode::ConnectionRevoked),
            );
        }
        ConnectionState::Unknown => {
            return (
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::ConnectionStateUnknown),
            );
        }
        ConnectionState::NotConnected | ConnectionState::Connected => {}
    }

    match evidence.grant_state {
        GrantState::Partial => {
            return (CapabilityStatus::CannotUse, Some(ReasonCode::GrantPartial));
        }
        GrantState::Missing => {
            return (CapabilityStatus::CannotUse, Some(ReasonCode::GrantMissing));
        }
        GrantState::Expired => {
            return (CapabilityStatus::CannotUse, Some(ReasonCode::GrantExpired));
        }
        GrantState::Revoked => {
            return (CapabilityStatus::CannotUse, Some(ReasonCode::GrantRevoked));
        }
        GrantState::Unknown => {
            return (
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::GrantUnknown),
            );
        }
        GrantState::NotRequired | GrantState::Granted => {}
    }

    match evidence.executable {
        Some(true) => (CapabilityStatus::Available, None),
        Some(false) => (
            CapabilityStatus::CannotUse,
            Some(ReasonCode::ExecutionUnavailable),
        ),
        None => (
            CapabilityStatus::CannotCheck,
            Some(ReasonCode::ExecutionStateUnknown),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: CapabilityProfile = CapabilityProfile {
        capability_id: "test-capability",
        label: "Test",
        required: true,
        requested_scopes: &["read", "write"],
        management_path: "/keys",
    };

    fn evidence(connection_state: ConnectionState, grant_state: GrantState) -> CapabilityEvidence {
        CapabilityEvidence {
            catalog_available: true,
            access_allowed: true,
            connection_state,
            grant_state,
            executable: Some(true),
        }
    }

    #[test]
    fn closed_enums_are_stable() {
        assert_eq!(
            [
                CapabilityStatus::Available,
                CapabilityStatus::Missing,
                CapabilityStatus::CannotUse,
                CapabilityStatus::CannotCheck,
            ]
            .map(CapabilityStatus::as_str),
            ["available", "missing", "cannot_use", "cannot_check"]
        );
        assert_eq!(
            [
                ConnectionState::NotConnected,
                ConnectionState::Connecting,
                ConnectionState::Verifying,
                ConnectionState::Connected,
                ConnectionState::Expired,
                ConnectionState::Revoked,
                ConnectionState::Unknown,
            ]
            .map(ConnectionState::as_str),
            [
                "not_connected",
                "connecting",
                "verifying",
                "connected",
                "expired",
                "revoked",
                "unknown",
            ]
        );
        assert_eq!(
            [
                GrantState::NotRequired,
                GrantState::Granted,
                GrantState::Partial,
                GrantState::Missing,
                GrantState::Expired,
                GrantState::Revoked,
                GrantState::Unknown,
            ]
            .map(GrantState::as_str),
            [
                "not_required",
                "granted",
                "partial",
                "missing",
                "expired",
                "revoked",
                "unknown",
            ]
        );
    }

    #[test]
    fn total_derivation_covers_every_connection_and_grant_state() {
        let cases = [
            (
                evidence(ConnectionState::Connected, GrantState::Granted),
                CapabilityStatus::Available,
                None,
            ),
            (
                evidence(ConnectionState::NotConnected, GrantState::Missing),
                CapabilityStatus::Missing,
                Some(ReasonCode::ServiceNotConnected),
            ),
            (
                evidence(ConnectionState::Connecting, GrantState::Unknown),
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::ConnectionInProgress),
            ),
            (
                evidence(ConnectionState::Verifying, GrantState::Unknown),
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::ConnectionVerificationPending),
            ),
            (
                evidence(ConnectionState::Expired, GrantState::Expired),
                CapabilityStatus::CannotUse,
                Some(ReasonCode::ConnectionExpired),
            ),
            (
                evidence(ConnectionState::Revoked, GrantState::Revoked),
                CapabilityStatus::CannotUse,
                Some(ReasonCode::ConnectionRevoked),
            ),
            (
                evidence(ConnectionState::Unknown, GrantState::Unknown),
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::ConnectionStateUnknown),
            ),
            (
                evidence(ConnectionState::Connected, GrantState::NotRequired),
                CapabilityStatus::Available,
                None,
            ),
            (
                evidence(ConnectionState::Connected, GrantState::Partial),
                CapabilityStatus::CannotUse,
                Some(ReasonCode::GrantPartial),
            ),
            (
                evidence(ConnectionState::Connected, GrantState::Missing),
                CapabilityStatus::CannotUse,
                Some(ReasonCode::GrantMissing),
            ),
            (
                evidence(ConnectionState::Connected, GrantState::Expired),
                CapabilityStatus::CannotUse,
                Some(ReasonCode::GrantExpired),
            ),
            (
                evidence(ConnectionState::Connected, GrantState::Revoked),
                CapabilityStatus::CannotUse,
                Some(ReasonCode::GrantRevoked),
            ),
            (
                evidence(ConnectionState::Connected, GrantState::Unknown),
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::GrantUnknown),
            ),
        ];

        for (evidence, expected_status, expected_reason) in cases {
            let readiness = evaluate_profile(&PROFILE, evidence);
            assert_eq!(readiness.status, expected_status);
            assert_eq!(readiness.reason_code, expected_reason);
        }
    }

    #[test]
    fn available_requires_access_and_executable_runtime_evidence() {
        let mut denied = evidence(ConnectionState::Connected, GrantState::Granted);
        denied.access_allowed = false;
        assert_eq!(
            derive_status(denied),
            (CapabilityStatus::CannotUse, Some(ReasonCode::AccessDenied))
        );

        let mut unavailable = evidence(ConnectionState::Connected, GrantState::Granted);
        unavailable.executable = Some(false);
        assert_eq!(
            derive_status(unavailable),
            (
                CapabilityStatus::CannotUse,
                Some(ReasonCode::ExecutionUnavailable)
            )
        );

        let mut unknown = evidence(ConnectionState::Connected, GrantState::Granted);
        unknown.executable = None;
        assert_eq!(
            derive_status(unknown),
            (
                CapabilityStatus::CannotCheck,
                Some(ReasonCode::ExecutionStateUnknown)
            )
        );
    }

    #[test]
    fn scope_derivation_preserves_granted_partial_missing_and_unknown() {
        let now = Utc::now();
        assert_eq!(
            grant_state_from_status_and_scopes(
                "active",
                None,
                Some("read write"),
                &["read", "write"],
                now,
            ),
            GrantState::Granted
        );
        assert_eq!(
            grant_state_from_status_and_scopes(
                "active",
                None,
                Some("read"),
                &["read", "write"],
                now,
            ),
            GrantState::Partial
        );
        assert_eq!(
            grant_state_from_status_and_scopes(
                "active",
                None,
                Some("profile"),
                &["read", "write"],
                now,
            ),
            GrantState::Missing
        );
        assert_eq!(
            grant_state_from_status_and_scopes("active", None, None, &["read", "write"], now,),
            GrantState::Unknown
        );
    }

    #[test]
    fn registry_identity_is_versioned_and_closed() {
        assert_eq!(ASSISTANT_READINESS_REVISION, "nyxid-assistant-readiness.v1");
        assert_eq!(CAPABILITY_PROFILES.len(), 1);
        let github = &CAPABILITY_PROFILES[0];
        assert_eq!(github.capability_id, "api-github");
        assert_eq!(github.label, "GitHub");
        assert!(!github.required);
        assert_eq!(github.requested_scopes, ["repo"]);
    }
}
