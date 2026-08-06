//! Server-authoritative assistant capability readiness projection.
//!
//! This is a CQRS read model over existing execution authorities. It does not
//! grant access and it never decrypts credentials. Per-user capabilities reuse
//! the canonical MCP projection; platform capabilities use explicitly named
//! proxy configuration predicates and fail closed where route selection cannot
//! be classified without duplicating the production resolver.

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_pool::{COLLECTION_NAME as SERVICE_POOLS, ServicePool};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::services::mcp_service::{self, McpToolSource};
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::proxy_service;
use crate::services::user_service_service::{self, CredentialSource};

pub const ASSISTANT_READINESS_REVISION: &str = "nyxid-assistant-readiness.v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityStatus {
    Available,
    Missing,
    CannotUse,
    CannotCheck,
}

impl CapabilityStatus {
    #[cfg(test)]
    pub const ALL: [Self; 4] = [
        Self::Available,
        Self::Missing,
        Self::CannotUse,
        Self::CannotCheck,
    ];

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
    #[cfg(test)]
    pub const ALL: [Self; 7] = [
        Self::NotConnected,
        Self::Connecting,
        Self::Verifying,
        Self::Connected,
        Self::Expired,
        Self::Revoked,
        Self::Unknown,
    ];

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
    #[cfg(test)]
    pub const ALL: [Self; 7] = [
        Self::NotRequired,
        Self::Granted,
        Self::Partial,
        Self::Missing,
        Self::Expired,
        Self::Revoked,
        Self::Unknown,
    ];

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
    pub management_path: Option<&'static str>,
    pub reason_code: Option<ReasonCode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // The suffix names the backing lookup key, not the public id.
enum EvidenceSource {
    UserServiceCatalogSlug(&'static str),
    AdminServiceSlug(&'static str),
    PlatformCallbackSlug(&'static str),
}

struct CapabilityProfile {
    capability_id: &'static str,
    evidence_source: EvidenceSource,
    label: &'static str,
    required: bool,
    requested_scopes: &'static [&'static str],
    management_path: Option<&'static str>,
}

// The registry is deliberately closed and versioned with the response. Adding
// or changing a row requires a readiness revision and a consumer fixture update.
const CAPABILITY_PROFILES: &[CapabilityProfile] = &[
    CapabilityProfile {
        capability_id: "api-github",
        evidence_source: EvidenceSource::UserServiceCatalogSlug("api-github"),
        label: "GitHub",
        required: false,
        requested_scopes: &["repo"],
        management_path: Some("/keys"),
    },
    CapabilityProfile {
        capability_id: "model",
        evidence_source: EvidenceSource::PlatformCallbackSlug("chrono-llm-public"),
        label: "Model",
        required: true,
        requested_scopes: &[],
        management_path: Some("/keys"),
    },
    CapabilityProfile {
        capability_id: "runtime",
        evidence_source: EvidenceSource::AdminServiceSlug("aevatar"),
        label: "Runtime",
        required: true,
        requested_scopes: &[],
        management_path: None,
    },
];

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

enum UserServiceSelection {
    None,
    Selected(Box<SelectedUserService>),
    Ambiguous,
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
    match profile.evidence_source {
        EvidenceSource::UserServiceCatalogSlug(catalog_slug) => {
            load_user_service_capability_evidence(
                db,
                node_ws_manager,
                user_id,
                profile,
                catalog_slug,
                evaluated_at,
            )
            .await
        }
        EvidenceSource::AdminServiceSlug(catalog_slug) => {
            load_admin_service_evidence(db, catalog_slug).await
        }
        EvidenceSource::PlatformCallbackSlug(catalog_slug) => {
            load_platform_callback_evidence(db, user_id, catalog_slug, evaluated_at).await
        }
    }
}

async fn load_user_service_capability_evidence(
    db: &mongodb::Database,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    profile: &CapabilityProfile,
    catalog_slug: &str,
    evaluated_at: DateTime<Utc>,
) -> AppResult<CapabilityEvidence> {
    let Some(catalog_service) = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! {
            "slug": catalog_slug,
            "is_active": true,
            "service_type": "http",
        })
        .await?
    else {
        return Ok(CapabilityEvidence::unavailable());
    };

    let visible_services =
        user_service_service::list_user_services_with_sources_for_policy(db, user_id).await?;
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

    match select_user_service(personal, catalog_slug) {
        UserServiceSelection::Selected(selected) => {
            return unified_service_evidence(
                db,
                node_ws_manager,
                user_id,
                profile,
                *selected,
                evaluated_at,
            )
            .await;
        }
        UserServiceSelection::Ambiguous => return Ok(CapabilityEvidence::unavailable()),
        UserServiceSelection::None => {}
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

    match select_user_service(org, catalog_slug) {
        UserServiceSelection::Selected(selected) => {
            return unified_service_evidence(
                db,
                node_ws_manager,
                user_id,
                profile,
                *selected,
                evaluated_at,
            )
            .await;
        }
        UserServiceSelection::Ambiguous => return Ok(CapabilityEvidence::unavailable()),
        UserServiceSelection::None => {}
    }

    Ok(CapabilityEvidence::not_connected())
}

async fn load_admin_service_evidence(
    db: &mongodb::Database,
    catalog_slug: &str,
) -> AppResult<CapabilityEvidence> {
    let Some(service) = load_active_catalog_service(db, catalog_slug).await? else {
        return Ok(CapabilityEvidence::unavailable());
    };

    let resolver_configured = service.service_type == "http"
        && service.service_category != "provider"
        && !service.requires_user_credential;
    let identity_mode_configured =
        matches!(service.identity_propagation_mode.as_str(), "jwt" | "both")
            && service
                .identity_jwt_audience
                .as_deref()
                .is_some_and(|audience| !audience.trim().is_empty())
            && service.inject_delegation_token;
    let auth_chain_configured = service.forward_access_token || identity_mode_configured;

    Ok(platform_evidence(
        stored_catalog_connection_state(&service),
        resolver_configured && auth_chain_configured,
    ))
}

async fn load_platform_callback_evidence(
    db: &mongodb::Database,
    user_id: &str,
    catalog_slug: &str,
    evaluated_at: DateTime<Utc>,
) -> AppResult<CapabilityEvidence> {
    let Some(catalog_service) = load_active_catalog_service(db, catalog_slug).await? else {
        return Ok(CapabilityEvidence::unavailable());
    };
    let config_executable =
        catalog_service.service_type == "http" && catalog_service.service_category != "provider";

    let personal = user_service_service::find_by_slug(db, user_id, catalog_slug).await?;
    if model_route_has_unmirrored_authority(
        db,
        user_id,
        catalog_slug,
        &catalog_service,
        personal.as_ref(),
    )
    .await?
    {
        return Ok(CapabilityEvidence::unavailable());
    }

    if let Some(service) = personal {
        return personal_model_service_evidence(
            db,
            user_id,
            &catalog_service,
            service,
            config_executable,
            evaluated_at,
        )
        .await;
    }

    let legacy_connection = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": &catalog_service.id,
        })
        .await?;

    if catalog_service.requires_user_credential {
        let connected = legacy_connection.as_ref().is_some_and(|connection| {
            connection.is_active
                && connection
                    .credential_encrypted
                    .as_ref()
                    .is_some_and(|credential| !credential.is_empty())
        });
        return Ok(if connected {
            platform_evidence(ConnectionState::Connected, config_executable)
        } else {
            CapabilityEvidence::not_connected()
        });
    }

    if legacy_connection
        .as_ref()
        .is_some_and(|connection| !connection.is_active)
    {
        return Ok(CapabilityEvidence::not_connected());
    }

    let has_platform_backstop = catalog_service.auth_method == "none"
        || proxy_service::is_public_internal_master_credential_service(&catalog_service);
    let connection_state = if has_platform_backstop
        || (!config_executable && !catalog_service.credential_encrypted.is_empty())
    {
        ConnectionState::Connected
    } else {
        ConnectionState::Verifying
    };
    Ok(platform_evidence(
        connection_state,
        config_executable && has_platform_backstop,
    ))
}

async fn load_active_catalog_service(
    db: &mongodb::Database,
    slug: &str,
) -> AppResult<Option<DownstreamService>> {
    Ok(db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?)
}

fn stored_catalog_connection_state(service: &DownstreamService) -> ConnectionState {
    if service.auth_method == "none" || !service.credential_encrypted.is_empty() {
        ConnectionState::Connected
    } else {
        ConnectionState::Verifying
    }
}

fn platform_evidence(connection_state: ConnectionState, executable: bool) -> CapabilityEvidence {
    CapabilityEvidence {
        catalog_available: true,
        access_allowed: true,
        connection_state,
        grant_state: GrantState::NotRequired,
        executable: Some(executable),
    }
}

async fn model_route_has_unmirrored_authority(
    db: &mongodb::Database,
    user_id: &str,
    catalog_slug: &str,
    catalog_service: &DownstreamService,
    personal: Option<&UserService>,
) -> AppResult<bool> {
    if personal.is_some_and(|service| {
        service
            .node_id
            .as_deref()
            .is_some_and(|node_id| !node_id.is_empty())
    }) {
        return Ok(true);
    }

    let memberships =
        crate::services::org_service::list_memberships_for_member(db, user_id, false).await?;
    let org_user_ids: Vec<String> = memberships
        .into_iter()
        .map(|membership| membership.org_user_id)
        .collect();
    let mut route_owner_ids = Vec::with_capacity(org_user_ids.len() + 1);
    route_owner_ids.push(user_id.to_string());
    route_owner_ids.extend(org_user_ids.iter().cloned());

    // Any pool with this bare slug can replace exact/legacy/platform routing.
    // Include organization owners as well as the caller: production resolves
    // org pools during its membership walk.
    if db
        .collection::<ServicePool>(SERVICE_POOLS)
        .count_documents(doc! {
            "user_id": { "$in": &route_owner_ids },
            "slug": catalog_slug,
        })
        .await?
        > 0
    {
        return Ok(true);
    }

    if org_user_ids.is_empty() {
        return Ok(false);
    }

    if db
        .collection::<UserService>(USER_SERVICES)
        .count_documents(doc! {
            "user_id": { "$in": &org_user_ids },
            "$or": [
                { "slug": catalog_slug },
                { "catalog_service_id": &catalog_service.id },
            ],
        })
        .await?
        > 0
    {
        return Ok(true);
    }

    if db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .count_documents(doc! {
            "user_id": { "$in": &org_user_ids },
            "service_id": &catalog_service.id,
        })
        .await?
        > 0
    {
        return Ok(true);
    }

    // The production viewer guard also treats non-revoked org provider tokens
    // as service presence. Missing this query would fail open for catalog rows
    // backed by a provider configuration.
    if let Some(provider_config_id) = catalog_service.provider_config_id.as_deref()
        && db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .count_documents(doc! {
                "user_id": { "$in": &org_user_ids },
                "provider_config_id": provider_config_id,
                "status": { "$in": ["active", "expired", "refresh_failed"] },
            })
            .await?
            > 0
    {
        return Ok(true);
    }

    Ok(false)
}

async fn personal_model_service_evidence(
    db: &mongodb::Database,
    user_id: &str,
    catalog_service: &DownstreamService,
    service: UserService,
    config_executable: bool,
    evaluated_at: DateTime<Utc>,
) -> AppResult<CapabilityEvidence> {
    let mut executable = config_executable;

    if service.source.as_deref() == Some("auto_provision") {
        match proxy_service::verify_auto_provision_eligibility(db, &service, user_id).await {
            Ok(()) => {}
            Err(AppError::NotFound(_)) => executable = false,
            Err(error) => return Err(error),
        }
    }

    let endpoint_exists = db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! { "_id": &service.endpoint_id })
        .await?
        .is_some();
    if !endpoint_exists {
        executable = false;
    }

    if service.auth_method == "none" {
        return Ok(platform_evidence(ConnectionState::Connected, executable));
    }

    if let Some(api_key_id) = service.api_key_id.as_deref() {
        let key = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": api_key_id, "user_id": user_id })
            .await?;
        let Some(key) = key else {
            return Ok(platform_evidence(ConnectionState::Verifying, false));
        };
        let connection_state =
            connection_state_for_key(&key, &service, Some(executable), evaluated_at);
        let grant_state = grant_state_for_key(&key, &[], evaluated_at);
        return Ok(CapabilityEvidence {
            catalog_available: true,
            access_allowed: true,
            connection_state,
            grant_state,
            executable: Some(executable),
        });
    }

    let connection_state = if service.source.as_deref() == Some("auto_provision")
        && proxy_service::is_public_internal_master_credential_service(catalog_service)
    {
        ConnectionState::Connected
    } else {
        ConnectionState::Verifying
    };
    Ok(platform_evidence(connection_state, executable))
}

fn select_user_service(
    mut candidates: Vec<SelectedUserService>,
    catalog_slug: &str,
) -> UserServiceSelection {
    // Runtime org resolution continues past denied memberships. Keep a denied
    // candidate only when none are accessible so readiness can report denial.
    if candidates.iter().any(|candidate| candidate.access_allowed) {
        candidates.retain(|candidate| candidate.access_allowed);
    }
    let (exact, aliases): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|candidate| candidate.service.slug == catalog_slug);
    let mut surviving = if exact.is_empty() { aliases } else { exact };
    match surviving.len() {
        0 => UserServiceSelection::None,
        1 => UserServiceSelection::Selected(Box::new(surviving.remove(0))),
        _ => UserServiceSelection::Ambiguous,
    }
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
        // Connected either because no auth is needed, or because the service
        // is node-routed and the node proved executable.
        let connection_state = if selected.service.auth_method == "none"
            || (selected
                .service
                .node_id
                .as_deref()
                .is_some_and(|node_id| !node_id.is_empty())
                && executable == Some(true))
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
                .find(proxy_service::legacy_personal_provider_token_filter(
                    user_id,
                    provider_config_id,
                ))
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
            // Connected via a proven-executable node route, or via a
            // server-held credential.
            if (node_routed && executable == Some(true))
                || crate::services::user_api_key_service::has_server_credential(key)
            {
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

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct FixtureEvidence {
    pub catalog_available: bool,
    pub access_allowed: bool,
    pub connection_state: ConnectionState,
    pub grant_state: GrantState,
    pub executable: Option<bool>,
}

#[cfg(test)]
pub(crate) fn evaluate_fixture_capability(
    capability_id: &str,
    evidence: FixtureEvidence,
) -> CapabilityReadiness {
    let profile = CAPABILITY_PROFILES
        .iter()
        .find(|profile| profile.capability_id == capability_id)
        .unwrap_or_else(|| panic!("unknown fixture capability '{capability_id}'"));
    evaluate_profile(
        profile,
        CapabilityEvidence {
            catalog_available: evidence.catalog_available,
            access_allowed: evidence.access_allowed,
            connection_state: evidence.connection_state,
            grant_state: evidence.grant_state,
            executable: evidence.executable,
        },
    )
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
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::service_pool::{PoolStrategy, ServicePoolMember};
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::test_utils::{
        connect_test_database, test_membership, test_user, test_user_endpoint, test_user_service,
    };

    const PROFILE: CapabilityProfile = CapabilityProfile {
        capability_id: "test-capability",
        evidence_source: EvidenceSource::UserServiceCatalogSlug("test-capability"),
        label: "Test",
        required: true,
        requested_scopes: &["read", "write"],
        management_path: Some("/keys"),
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

    fn service_candidate(id: &str, access_allowed: bool) -> SelectedUserService {
        SelectedUserService {
            service: test_user_service(id, id, "api-github", id, None, None),
            effective_owner_id: id.to_string(),
            access_allowed,
        }
    }

    fn platform_catalog(slug: &str) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = slug.to_string();
        service.name = slug.to_string();
        service.visibility = "public".to_string();
        service.service_category = "internal".to_string();
        service.service_type = "http".to_string();
        service.auth_method = "none".to_string();
        service.requires_user_credential = false;
        service.is_active = true;
        service
    }

    async fn insert_catalog(db: &mongodb::Database, service: &DownstreamService) {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(service)
            .await
            .expect("insert readiness catalog service");
    }

    async fn evaluated_capability(
        db: &mongodb::Database,
        user_id: &str,
        capability_id: &str,
        evaluated_at: DateTime<Utc>,
    ) -> CapabilityReadiness {
        let profile = CAPABILITY_PROFILES
            .iter()
            .find(|profile| profile.capability_id == capability_id)
            .expect("registered capability");
        let evidence = load_capability_evidence(
            db,
            &NodeWsManager::new(30, 100),
            user_id,
            profile,
            evaluated_at,
        )
        .await
        .expect("load capability evidence");
        evaluate_profile(profile, evidence)
    }

    fn user_api_key(
        id: &str,
        user_id: &str,
        status: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> UserApiKey {
        let now = Utc::now();
        UserApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            label: "Model credential".to_string(),
            credential_type: "bearer".to_string(),
            credential_encrypted: Some(vec![1, 2, 3]),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at,
            provider_config_id: None,
            connection_id: None,
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            credential_source: None,
            status: status.to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn legacy_connection(
        user_id: &str,
        service_id: &str,
        is_active: bool,
        credential_encrypted: Option<Vec<u8>>,
    ) -> UserServiceConnection {
        let now = Utc::now();
        UserServiceConnection {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            service_id: service_id.to_string(),
            credential_encrypted,
            credential_type: Some("bearer".to_string()),
            credential_label: None,
            metadata: None,
            is_active,
            created_at: now,
            updated_at: now,
        }
    }

    fn service_pool(owner_id: &str, slug: &str) -> ServicePool {
        let now = Utc::now();
        ServicePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            slug: slug.to_string(),
            name: "Model pool".to_string(),
            description: None,
            strategy: PoolStrategy::RoundRobin,
            members: vec![ServicePoolMember {
                user_service_id: uuid::Uuid::new_v4().to_string(),
                weight: 1,
                enabled: true,
            }],
            rr_counter: 0,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn provider_token(user_id: &str, provider_config_id: &str) -> UserProviderToken {
        let now = Utc::now();
        UserProviderToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_config_id.to_string(),
            connection_id: None,
            credential_user_id: None,
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn organization_selection_continues_past_denied_candidates() {
        let UserServiceSelection::Selected(selected) = select_user_service(
            vec![
                service_candidate("denied", false),
                service_candidate("accessible", true),
            ],
            "api-github",
        ) else {
            panic!("accessible organization candidate must be selected");
        };
        assert_eq!(selected.service.id, "accessible");
        assert!(selected.access_allowed);

        let UserServiceSelection::Selected(denied) =
            select_user_service(vec![service_candidate("only-denied", false)], "api-github")
        else {
            panic!("denied evidence must remain reportable");
        };
        assert!(!denied.access_allowed);
    }

    #[test]
    fn selection_prefers_one_exact_slug_and_rejects_ambiguous_aliases() {
        let exact = service_candidate("exact", true);
        let mut alias = service_candidate("alias", true);
        alias.service.slug = "github-work".to_string();
        let UserServiceSelection::Selected(selected) =
            select_user_service(vec![alias, exact], "api-github")
        else {
            panic!("one exact slug must disambiguate aliases");
        };
        assert_eq!(selected.service.id, "exact");

        let mut first = service_candidate("first", true);
        first.service.slug = "github-work".to_string();
        let mut second = service_candidate("second", true);
        second.service.slug = "github-personal".to_string();
        assert!(matches!(
            select_user_service(vec![first, second], "api-github"),
            UserServiceSelection::Ambiguous
        ));
    }

    #[tokio::test]
    async fn ambiguous_personal_aliases_fail_closed_as_cannot_check() {
        let Some(db) = connect_test_database("assistant_readiness_alias_ambiguous").await else {
            eprintln!("skipping assistant readiness ambiguity test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let catalog_id = uuid::Uuid::new_v4().to_string();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = catalog_id.clone();
        catalog.slug = "api-github".to_string();
        catalog.service_type = "http".to_string();
        catalog.is_active = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .expect("insert GitHub catalog service");

        for slug in ["github-work", "github-personal"] {
            db.collection::<UserService>(USER_SERVICES)
                .insert_one(test_user_service(
                    &uuid::Uuid::new_v4().to_string(),
                    &user_id,
                    slug,
                    &uuid::Uuid::new_v4().to_string(),
                    Some(&catalog_id),
                    None,
                ))
                .await
                .expect("insert ambiguous GitHub service");
        }

        let profile = &CAPABILITY_PROFILES[0];
        let evidence = load_capability_evidence(
            &db,
            &NodeWsManager::new(30, 100),
            &user_id,
            profile,
            Utc::now(),
        )
        .await
        .expect("load ambiguous GitHub evidence");
        let readiness = evaluate_profile(profile, evidence);

        assert_eq!(readiness.status, CapabilityStatus::CannotCheck);
        assert_eq!(readiness.connection_state, ConnectionState::Unknown);
        assert_eq!(readiness.grant_state, GrantState::Unknown);
        assert_eq!(
            readiness.reason_code,
            Some(ReasonCode::CapabilityEvidenceUnavailable)
        );
    }

    #[tokio::test]
    async fn runtime_evidence_requires_resolver_config_and_an_auth_delivery_chain() {
        let Some(db) = connect_test_database("assistant_readiness_runtime_chain").await else {
            eprintln!("skipping runtime readiness test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let evaluated_at = Utc::now();

        let absent = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(absent.status, CapabilityStatus::CannotCheck);
        assert_eq!(
            absent.reason_code,
            Some(ReasonCode::CapabilityEvidenceUnavailable)
        );

        let mut runtime = platform_catalog("aevatar");
        runtime.is_active = false;
        insert_catalog(&db, &runtime).await;
        let inactive = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(inactive.status, CapabilityStatus::CannotCheck);

        runtime.is_active = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(doc! { "_id": &runtime.id }, &runtime)
            .await
            .expect("activate runtime catalog service");
        let no_chain = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(no_chain.connection_state, ConnectionState::Connected);
        assert_eq!(no_chain.status, CapabilityStatus::CannotUse);
        assert_eq!(no_chain.reason_code, Some(ReasonCode::ExecutionUnavailable));

        runtime.forward_access_token = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(doc! { "_id": &runtime.id }, &runtime)
            .await
            .expect("configure compatibility bridge");
        let bridge = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(bridge.status, CapabilityStatus::Available);

        runtime.forward_access_token = false;
        runtime.identity_propagation_mode = "jwt".to_string();
        runtime.identity_jwt_audience = Some("urn:aevatar:api".to_string());
        runtime.inject_delegation_token = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(doc! { "_id": &runtime.id }, &runtime)
            .await
            .expect("configure identity chain");
        let identity = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(identity.status, CapabilityStatus::Available);

        runtime.auth_method = "bearer".to_string();
        runtime.credential_encrypted.clear();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(doc! { "_id": &runtime.id }, &runtime)
            .await
            .expect("remove runtime credential");
        let credential_missing = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(
            credential_missing.connection_state,
            ConnectionState::Verifying
        );
        assert_eq!(credential_missing.status, CapabilityStatus::CannotCheck);

        runtime.auth_method = "none".to_string();
        runtime.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(doc! { "_id": &runtime.id }, &runtime)
            .await
            .expect("misconfigure runtime credential ownership");
        let per_user = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(per_user.status, CapabilityStatus::CannotUse);

        runtime.requires_user_credential = false;
        runtime.service_category = "provider".to_string();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(doc! { "_id": &runtime.id }, &runtime)
            .await
            .expect("misconfigure runtime category");
        let provider = evaluated_capability(&db, &user_id, "runtime", evaluated_at).await;
        assert_eq!(provider.status, CapabilityStatus::CannotUse);
    }

    #[tokio::test]
    async fn model_backstop_ignores_personal_alias_but_respects_disconnect() {
        let Some(db) = connect_test_database("assistant_readiness_model_backstop").await else {
            eprintln!("skipping model backstop test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let evaluated_at = Utc::now();
        let mut catalog = platform_catalog("chrono-llm-public");
        catalog.auth_method = "bearer".to_string();
        catalog.auth_key_name = "Authorization".to_string();
        catalog.credential_encrypted = vec![1, 2, 3];
        insert_catalog(&db, &catalog).await;

        let backstop = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(backstop.status, CapabilityStatus::Available);

        let alias = test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &user_id,
            "chrono-llm-public-2",
            &uuid::Uuid::new_v4().to_string(),
            Some(&catalog.id),
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(alias)
            .await
            .expect("insert personal catalog alias");
        let alias_ignored = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(alias_ignored.status, CapabilityStatus::Available);

        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .insert_one(legacy_connection(&user_id, &catalog.id, false, None))
            .await
            .expect("insert explicit disconnect");
        let disconnected = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(disconnected.status, CapabilityStatus::Missing);
        assert_eq!(disconnected.connection_state, ConnectionState::NotConnected);
        assert_eq!(disconnected.grant_state, GrantState::Missing);
    }

    #[tokio::test]
    async fn model_auto_provision_reuses_eligibility_and_requires_endpoint() {
        let Some(db) = connect_test_database("assistant_readiness_model_auto").await else {
            eprintln!("skipping model auto-provision test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let evaluated_at = Utc::now();
        let mut catalog = platform_catalog("chrono-llm-public");
        catalog.auth_method = "bearer".to_string();
        catalog.auth_key_name = "Authorization".to_string();
        catalog.credential_encrypted = vec![1, 2, 3];
        insert_catalog(&db, &catalog).await;

        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &user_id,
                "Model",
                "https://llm.example.test",
                None,
                Some(&catalog.id),
            ))
            .await
            .expect("insert model endpoint");
        let service_id = uuid::Uuid::new_v4().to_string();
        let mut service = test_user_service(
            &service_id,
            &user_id,
            "chrono-llm-public",
            &endpoint_id,
            Some(&catalog.id),
            None,
        );
        service.auth_method = catalog.auth_method.clone();
        service.auth_key_name = catalog.auth_key_name.clone();
        service.source = Some("auto_provision".to_string());
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert auto-provisioned model service");

        let available = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(available.status, CapabilityStatus::Available);
        assert_eq!(available.connection_state, ConnectionState::Connected);

        service.auth_key_name = "X-Stale-Key".to_string();
        db.collection::<UserService>(USER_SERVICES)
            .replace_one(doc! { "_id": &service.id }, &service)
            .await
            .expect("drift auto-provision auth snapshot");
        let drifted = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(drifted.status, CapabilityStatus::CannotUse);
        assert_eq!(drifted.reason_code, Some(ReasonCode::ExecutionUnavailable));

        service.auth_key_name = catalog.auth_key_name.clone();
        db.collection::<UserService>(USER_SERVICES)
            .replace_one(doc! { "_id": &service.id }, &service)
            .await
            .expect("restore auto-provision auth snapshot");
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .delete_one(doc! { "_id": &endpoint_id })
            .await
            .expect("remove model endpoint");
        let endpoint_missing = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(endpoint_missing.status, CapabilityStatus::CannotUse);
        assert_eq!(
            endpoint_missing.reason_code,
            Some(ReasonCode::ExecutionUnavailable)
        );
    }

    #[tokio::test]
    async fn model_byok_key_states_and_missing_endpoint_are_truthful() {
        let Some(db) = connect_test_database("assistant_readiness_model_key").await else {
            eprintln!("skipping model BYOK key test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let evaluated_at = Utc::now();
        let catalog = platform_catalog("chrono-llm-public");
        insert_catalog(&db, &catalog).await;
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &user_id,
                "Model BYOK",
                "https://llm.example.test",
                None,
                Some(&catalog.id),
            ))
            .await
            .expect("insert BYOK endpoint");
        let key_id = uuid::Uuid::new_v4().to_string();
        let mut service = test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &user_id,
            "chrono-llm-public",
            &endpoint_id,
            Some(&catalog.id),
            None,
        );
        service.auth_method = "bearer".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.api_key_id = Some(key_id.clone());
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert BYOK model service");
        let mut key = user_api_key(
            &key_id,
            &user_id,
            "active",
            Some(evaluated_at - chrono::Duration::seconds(1)),
        );
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .expect("insert expired BYOK key");

        let expired = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(expired.status, CapabilityStatus::CannotUse);
        assert_eq!(expired.connection_state, ConnectionState::Expired);
        assert_eq!(expired.grant_state, GrantState::NotRequired);

        key.expires_at = None;
        key.status = "revoked".to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .replace_one(doc! { "_id": &key.id }, &key)
            .await
            .expect("revoke BYOK key");
        let revoked = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(revoked.connection_state, ConnectionState::Revoked);

        key.status = "active".to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .replace_one(doc! { "_id": &key.id }, &key)
            .await
            .expect("activate BYOK key");
        let available = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(available.status, CapabilityStatus::Available);

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .delete_one(doc! { "_id": &endpoint_id })
            .await
            .expect("remove BYOK endpoint");
        let endpoint_missing = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(endpoint_missing.status, CapabilityStatus::CannotUse);
        assert_eq!(
            endpoint_missing.reason_code,
            Some(ReasonCode::ExecutionUnavailable)
        );
    }

    #[tokio::test]
    async fn model_org_alias_provider_token_and_pools_fail_closed() {
        let Some(db) = connect_test_database("assistant_readiness_model_guards").await else {
            eprintln!("skipping model routing guard test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let evaluated_at = Utc::now();
        let mut catalog = platform_catalog("chrono-llm-public");
        catalog.provider_config_id = Some(uuid::Uuid::new_v4().to_string());
        insert_catalog(&db, &catalog).await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &user_id, OrgRole::Viewer, None))
            .await
            .expect("insert model org membership");

        let mut inactive_alias = test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &org_id,
            "org-model-alias",
            &uuid::Uuid::new_v4().to_string(),
            Some(&catalog.id),
            None,
        );
        inactive_alias.is_active = false;
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&inactive_alias)
            .await
            .expect("insert inactive org model alias");
        let org_alias = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(org_alias.status, CapabilityStatus::CannotCheck);
        assert_eq!(org_alias.connection_state, ConnectionState::Unknown);

        db.collection::<UserService>(USER_SERVICES)
            .delete_one(doc! { "_id": &inactive_alias.id })
            .await
            .expect("remove org alias");
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(provider_token(
                &org_id,
                catalog
                    .provider_config_id
                    .as_deref()
                    .expect("provider config"),
            ))
            .await
            .expect("insert org provider token");
        let provider_presence = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(provider_presence.status, CapabilityStatus::CannotCheck);

        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .delete_many(doc! {})
            .await
            .expect("remove org provider tokens");
        db.collection::<ServicePool>(SERVICE_POOLS)
            .insert_one(service_pool(&org_id, "chrono-llm-public"))
            .await
            .expect("insert org model pool");
        let org_pool = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(org_pool.status, CapabilityStatus::CannotCheck);

        db.collection::<ServicePool>(SERVICE_POOLS)
            .delete_many(doc! {})
            .await
            .expect("remove org model pool");
        db.collection::<ServicePool>(SERVICE_POOLS)
            .insert_one(service_pool(&user_id, "chrono-llm-public"))
            .await
            .expect("insert personal model pool");
        let personal_pool = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(personal_pool.status, CapabilityStatus::CannotCheck);
    }

    #[tokio::test]
    async fn model_node_pin_fails_closed_before_credential_classification() {
        let Some(db) = connect_test_database("assistant_readiness_model_node").await else {
            eprintln!("skipping model node guard test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let catalog = platform_catalog("chrono-llm-public");
        insert_catalog(&db, &catalog).await;
        let service = test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &user_id,
            "chrono-llm-public",
            &uuid::Uuid::new_v4().to_string(),
            Some(&catalog.id),
            Some("offline-node"),
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service)
            .await
            .expect("insert node-pinned model service");

        let readiness = evaluated_capability(&db, &user_id, "model", Utc::now()).await;
        assert_eq!(readiness.status, CapabilityStatus::CannotCheck);
        assert_eq!(readiness.connection_state, ConnectionState::Unknown);
        assert_eq!(
            readiness.reason_code,
            Some(ReasonCode::CapabilityEvidenceUnavailable)
        );
    }

    #[tokio::test]
    async fn model_byok_deployment_uses_exact_or_legacy_state_only() {
        let Some(db) = connect_test_database("assistant_readiness_model_byok").await else {
            eprintln!("skipping model BYOK deployment test: no local MongoDB available");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let evaluated_at = Utc::now();
        let mut catalog = platform_catalog("chrono-llm-public");
        catalog.auth_method = "bearer".to_string();
        catalog.auth_key_name = "Authorization".to_string();
        catalog.requires_user_credential = true;
        insert_catalog(&db, &catalog).await;

        let absent = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(absent.status, CapabilityStatus::Missing);

        let connection = legacy_connection(&user_id, &catalog.id, true, Some(vec![1, 2, 3]));
        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .insert_one(&connection)
            .await
            .expect("insert legacy BYOK connection");
        let legacy = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(legacy.status, CapabilityStatus::Available);

        let mut disconnected = connection.clone();
        disconnected.is_active = false;
        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .replace_one(doc! { "_id": &connection.id }, &disconnected)
            .await
            .expect("disconnect legacy BYOK connection");
        let missing = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(missing.status, CapabilityStatus::Missing);

        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .delete_one(doc! { "_id": &connection.id })
            .await
            .expect("remove legacy BYOK connection");
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &user_id,
                "Model BYOK",
                "https://llm.example.test",
                None,
                Some(&catalog.id),
            ))
            .await
            .expect("insert exact BYOK endpoint");
        let key_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(user_api_key(&key_id, &user_id, "active", None))
            .await
            .expect("insert exact BYOK key");
        let mut exact = test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            &user_id,
            "chrono-llm-public",
            &endpoint_id,
            Some(&catalog.id),
            None,
        );
        exact.auth_method = "bearer".to_string();
        exact.auth_key_name = "Authorization".to_string();
        exact.api_key_id = Some(key_id);
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(exact)
            .await
            .expect("insert exact BYOK model service");
        let exact_ready = evaluated_capability(&db, &user_id, "model", evaluated_at).await;
        assert_eq!(exact_ready.status, CapabilityStatus::Available);
    }

    #[tokio::test]
    async fn organization_scope_denial_is_preserved_as_cannot_use() {
        let Some(db) = connect_test_database("assistant_readiness_scope_denial").await else {
            eprintln!("skipping assistant readiness scope test: no local MongoDB available");
            return;
        };

        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let catalog_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert readiness users");

        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = catalog_id.clone();
        catalog.slug = "api-github".to_string();
        catalog.service_type = "http".to_string();
        catalog.is_active = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .expect("insert readiness catalog service");

        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &service_id,
                &org_id,
                "api-github",
                &uuid::Uuid::new_v4().to_string(),
                Some(&catalog_id),
                None,
            ))
            .await
            .expect("insert scope-denied organization service");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &org_id,
                &actor_id,
                OrgRole::Member,
                Some(Vec::new()),
            ))
            .await
            .expect("insert scope-denied membership");

        let visible = user_service_service::list_user_services_with_sources(&db, &actor_id)
            .await
            .expect("load public service listing");
        assert!(
            visible.is_empty(),
            "public listing must hide scoped-out rows"
        );

        let node_ws_manager = NodeWsManager::new(30, 100);
        let snapshot = evaluate_readiness(&db, &node_ws_manager, &actor_id, Utc::now()).await;
        let capability = &snapshot.capabilities[0];
        assert_eq!(capability.status, CapabilityStatus::CannotUse);
        assert_eq!(capability.reason_code, Some(ReasonCode::AccessDenied));
    }

    #[test]
    fn closed_enums_are_stable() {
        assert_eq!(
            CapabilityStatus::ALL.map(CapabilityStatus::as_str),
            ["available", "missing", "cannot_use", "cannot_check"]
        );
        assert_eq!(
            ConnectionState::ALL.map(ConnectionState::as_str),
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
            GrantState::ALL.map(GrantState::as_str),
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
        assert_eq!(ASSISTANT_READINESS_REVISION, "nyxid-assistant-readiness.v2");
        assert_eq!(CAPABILITY_PROFILES.len(), 3);
        let github = &CAPABILITY_PROFILES[0];
        assert_eq!(github.capability_id, "api-github");
        assert_eq!(
            github.evidence_source,
            EvidenceSource::UserServiceCatalogSlug("api-github")
        );
        assert_eq!(github.label, "GitHub");
        assert!(!github.required);
        assert_eq!(github.requested_scopes, ["repo"]);
        assert_eq!(github.management_path, Some("/keys"));

        let model = &CAPABILITY_PROFILES[1];
        assert_eq!(model.capability_id, "model");
        assert_eq!(
            model.evidence_source,
            EvidenceSource::PlatformCallbackSlug("chrono-llm-public")
        );
        assert_eq!(model.label, "Model");
        assert!(model.required);
        assert!(model.requested_scopes.is_empty());
        assert_eq!(model.management_path, Some("/keys"));

        let runtime = &CAPABILITY_PROFILES[2];
        assert_eq!(runtime.capability_id, "runtime");
        assert_eq!(
            runtime.evidence_source,
            EvidenceSource::AdminServiceSlug("aevatar")
        );
        assert_eq!(runtime.label, "Runtime");
        assert!(runtime.required);
        assert!(runtime.requested_scopes.is_empty());
        assert_eq!(runtime.management_path, None);
    }
}
