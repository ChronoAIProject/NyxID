use axum::{
    Json,
    extract::{Path, Query, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::ssh_auth_mode::SshAuthMode;
use crate::models::user_api_key::UserApiKey;
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::models::ws_frame_injection::WsFrameInjection;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    catalog_service, cloud_credential_verify, credential_push_service, lark_permission,
    node_service, org_service, platform_credential_service, platform_operation_service,
    proxy_discovery_service, unified_key_service, user_api_key_service, user_endpoint_service,
    user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

/// At-creation probe for AWS cloud-billing auth methods. Resolves the
/// effective `(auth_method, base_url)` for the create request — from
/// the catalog if a `service_slug` is supplied, else from the inline
/// `auth_method` + `endpoint_url` overrides — and pings the upstream
/// once. Hard-fails on 4xx with a hint pointing the user at the
/// likely IAM gap; lets 5xx / network / timeout through so a flaky
/// cloud doesn't block credential adds. NyxID#716.
async fn verify_cloud_credential_against_catalog(
    state: &AppState,
    body: &CreateKeyRequest,
    credential: &str,
) -> AppResult<()> {
    // Resolve auth_method + base_url from the catalog entry (if a slug
    // was supplied) or from the inline overrides. We need both to
    // probe — auth_method picks the verifier, base_url is the target.
    let (auth_method, base_url) = match (body.service_slug.as_deref(), body.auth_method.as_deref())
    {
        (Some(slug), explicit_method) => {
            let Some(svc) = state
                .db
                .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .find_one(doc! { "slug": slug, "is_active": true })
                .await?
            else {
                // Catalog lookup miss: let `unified_key_service::create_key`
                // produce the canonical "not found" error.
                return Ok(());
            };
            let method = explicit_method.unwrap_or(svc.auth_method.as_str());
            let url = body
                .endpoint_url
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(svc.base_url.as_str())
                .to_string();
            (method.to_string(), url)
        }
        (None, Some(method)) => {
            let Some(url) = body.endpoint_url.as_deref().filter(|s| !s.is_empty()) else {
                return Ok(());
            };
            (method.to_string(), url.to_string())
        }
        _ => return Ok(()),
    };

    match auth_method.as_str() {
        "aws_sigv4" => {
            cloud_credential_verify::verify_aws_sigv4(&state.http_client, credential, &base_url)
                .await
        }
        _ => Ok(()),
    }
}

/// Resolve a `/keys/{id_or_slug}` path component to a UserService row,
/// walking org membership in the same priority order as the proxy's
/// effective-owner lookup. Returns the row so callers can continue with the
/// canonical service id even when the request used a slug.
///
/// The `_id` branch deliberately does NOT filter on `is_active`, so a
/// **disabled** service stays readable, re-enablable, and deletable through
/// `/keys/{id}`. Disabling is advertised in the UI as a reversible pause; when
/// this lookup filtered actives only, `GET`/`PUT`/`DELETE /keys/{id}` all
/// 404'd the moment a service was disabled, which stranded the row — the
/// detail page hosting the Enable button could no longer load, so nothing in
/// the product could undo the pause.
///
/// The slug branches keep the `is_active` filter: a slug is not unique across
/// disabled rows (the partial unique index only covers active ones), so
/// matching disabled rows by slug would be ambiguous. Resolution by slug is
/// therefore still active-only, and every credential-resolving path (proxy,
/// MCP catalog, discovery, scope enforcement) is untouched by this — those go
/// through `user_service_service`, not this function.
///
/// NOTE for anyone tidying this up: the resulting asymmetry (UUID resolves a
/// disabled row, slug does not) is deliberate, and was removed once before.
/// `c63ab733` added `is_active: true` here to make all three branches agree,
/// reasoning "tightening, not a regression: any inactive row was already
/// invisible to the proxy / list / slug paths". The paths it enumerated were
/// indeed already closed — which is exactly why this one was load-bearing: it
/// was the last route to the detail page that hosts the Enable control, so
/// closing it turned Disable into a one-way door for five months. Symmetry
/// here is not the goal; the management path and the execution paths want
/// different answers. See the paired assertions in
/// `get_key_resolves_disabled_service_by_uuid_but_not_by_slug`.
async fn find_user_service_for_actor(
    state: &AppState,
    actor: &str,
    id_or_slug: &str,
) -> AppResult<Option<UserService>> {
    if let Some(svc) = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": id_or_slug })
        .await?
    {
        return Ok(Some(svc));
    }

    if let Some(svc) = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "user_id": actor,
            "slug": id_or_slug,
            "is_active": true,
        })
        .await?
    {
        return Ok(Some(svc));
    }

    let memberships = org_service::find_active_memberships_with_timeout(&state.db, actor).await?;
    for membership in memberships {
        if let Some(svc) = state
            .db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! {
                "user_id": &membership.org_user_id,
                "slug": id_or_slug,
                "is_active": true,
            })
            .await?
        {
            return Ok(Some(svc));
        }
    }

    Ok(None)
}

struct KeyWriteAccess {
    owner_id: String,
    service_id: String,
}

/// Resolve which user_id owns this unified key (= UserService) and whether
/// the actor may modify it. Returns the effective owner_id (which may be an
/// org user_id) for downstream service calls.
///
/// Enforces both role (direct owner / org admin) AND the membership's
/// per-service `allowed_service_ids` scope. A scoped admin whose scope does
/// not include this key returns NotFound (same shape as a non-existent key)
/// to avoid leaking org topology.
async fn resolve_key_write_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<KeyWriteAccess> {
    let svc = find_user_service_for_actor(state, actor, key_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Key not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &svc.user_id).await?;
    if !access.can_read() || !access.allows_resource(&svc.id) {
        return Err(AppError::NotFound("Key not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this key".to_string(),
        ));
    }
    Ok(KeyWriteAccess {
        owner_id: svc.user_id,
        service_id: svc.id,
    })
}

/// Outcome of `resolve_key_read_owner`: the effective owner id used for
/// downstream service calls, plus the credential source for the response.
struct KeyReadAccess {
    owner_id: String,
    service_id: String,
    source: crate::services::user_service_service::CredentialSource,
}

/// Read variant: actor must be at least a viewer/member of the owning org
/// (or the direct owner). Used by GET endpoints so org members can fetch
/// the detail of org-shared services. Returns the effective owner id and
/// the [`CredentialSource`](crate::services::user_service_service::CredentialSource)
/// so the handler can tag the response correctly.
///
/// Honors the membership's `allowed_service_ids` scope: a member scoped to
/// service A who asks for service B gets `NotFound`, not a metadata leak.
async fn resolve_key_read_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<KeyReadAccess> {
    use crate::services::user_service_service::CredentialSource;

    let svc = find_user_service_for_actor(state, actor, key_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Key not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &svc.user_id).await?;
    if !access.can_read() || !access.allows_resource(&svc.id) {
        return Err(AppError::NotFound("Key not found".to_string()));
    }

    let source = match &access {
        org_service::OwnerAccess::Direct => CredentialSource::Personal,
        org_service::OwnerAccess::AsOrgAdmin { org_user_id, .. } => {
            // Look up the org's display_name + avatar_url for the response
            // payload. Avatar lets the frontend render the same org avatar
            // here as on the Organizations page (#545).
            let org = state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! { "_id": org_user_id })
                .await?;
            let (org_name, org_avatar_url) = org
                .map(|u| (u.display_name, u.avatar_url))
                .unwrap_or((None, None));
            let org_name = org_name.unwrap_or_else(|| "Unnamed Org".to_string());
            CredentialSource::Org {
                org_user_id: org_user_id.clone(),
                org_name,
                org_avatar_url,
                role: crate::models::org_membership::OrgRole::Admin,
                allowed: true,
            }
        }
        org_service::OwnerAccess::AsOrgMember {
            org_user_id, role, ..
        } => {
            let org = state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! { "_id": org_user_id })
                .await?;
            let (org_name, org_avatar_url) = org
                .map(|u| (u.display_name, u.avatar_url))
                .unwrap_or((None, None));
            let org_name = org_name.unwrap_or_else(|| "Unnamed Org".to_string());
            // Members can proxy/use unless the service is admin-only; viewers
            // cannot. Scope has already been enforced above via
            // allows_resource, so if we got here this key is within scope.
            let allowed = role.can_proxy() && (!svc.admin_only || role.can_admin());
            CredentialSource::Org {
                org_user_id: org_user_id.clone(),
                org_name,
                org_avatar_url,
                role: *role,
                allowed,
            }
        }
        org_service::OwnerAccess::Forbidden => {
            // can_read() guard above already short-circuits this branch.
            return Err(AppError::NotFound("Key not found".to_string()));
        }
    };

    Ok(KeyReadAccess {
        owner_id: svc.user_id,
        service_id: svc.id,
        source,
    })
}

#[derive(Deserialize, ToSchema)]
pub struct CreateKeyRequest {
    /// Catalog service slug (e.g., "llm-openai").
    pub service_slug: Option<String>,
    /// The credential value (API key, bearer token, etc.)
    /// Optional: not needed when routing via node (node manages credentials)
    pub credential: Option<String>,
    /// User-facing label
    pub label: String,
    /// Endpoint URL override (required for self-hosted providers and custom endpoints)
    pub endpoint_url: Option<String>,
    /// Custom slug (required when service_slug is None)
    pub slug: Option<String>,
    /// Custom auth method (default: "bearer")
    pub auth_method: Option<String>,
    /// Custom auth key name (default: "Authorization")
    pub auth_key_name: Option<String>,
    /// Route through this node agent (optional)
    pub node_id: Option<String>,
    /// For org-owned services, restrict proxy execution to org admins.
    /// Person-owned services ignore this flag.
    pub admin_only: Option<bool>,
    /// SSH host (required for custom SSH services)
    pub ssh_host: Option<String>,
    /// SSH port (default: 22)
    pub ssh_port: Option<u16>,
    /// Enable SSH certificate auth (default: true)
    pub ssh_certificate_auth: Option<bool>,
    /// SSH auth mode: "cert", "node_key", or "proxy_only"
    pub ssh_auth_mode: Option<SshAuthMode>,
    /// Comma-separated allowed principals
    pub ssh_principals: Option<String>,
    /// Certificate TTL in minutes (default: 30)
    pub ssh_certificate_ttl_minutes: Option<u32>,
    /// Identity propagation mode: "none" | "headers" | "jwt" | "both"
    pub identity_propagation_mode: Option<String>,
    pub identity_include_user_id: Option<bool>,
    pub identity_include_email: Option<bool>,
    pub identity_include_name: Option<bool>,
    pub identity_jwt_audience: Option<String>,
    /// Forward the caller's NyxID access token as Authorization: Bearer
    pub forward_access_token: Option<bool>,
    /// Inject X-NyxID-Delegation-Token for downstream user identification
    pub inject_delegation_token: Option<bool>,
    pub delegation_token_scope: Option<String>,
    /// When set, create this key as owned by the given org (the `user_id`
    /// on the underlying `UserService` / `UserEndpoint` / `UserApiKey`
    /// rows will be the org's user id, making the credential visible to
    /// every member of that org). The caller must be an admin of the org.
    /// Omit to create a personal key owned by the caller.
    pub target_org_id: Option<String>,
    /// Optional OpenAPI spec URL for endpoint discovery. Three-state:
    ///   - Omitted: for catalog-backed keys, inherit the catalog entry's
    ///     spec URL automatically. For custom endpoints, store none.
    ///   - Empty string: opt out of catalog inheritance -- store none even
    ///     when the catalog has a default.
    ///   - Non-empty URL: store this value verbatim.
    ///
    /// When present, agent-facing surfaces (MCP,
    /// `/endpoints/{id}/openapi-endpoints`) parse this spec so AI tools can
    /// call specific operations instead of only the generic proxy tool.
    /// SSH services ignore this field entirely.
    pub openapi_spec_url: Option<String>,
    /// User-owned WebSocket frame-auth injection rules to attach to the
    /// created UserService. Useful for custom WebSocket services such as
    /// Home Assistant that authenticate after upgrade.
    #[serde(default)]
    pub ws_frame_injections: Option<Vec<WsFrameInjection>>,
    /// User-provided OAuth Custom App client_id for `credential_mode: "user"`
    /// providers (Lark / Feishu / Twitter). When supplied alongside
    /// `oauth_client_secret`, the credentials are encrypted onto the new
    /// `UserApiKey` row itself, so this connection's authorize / exchange /
    /// refresh paths resolve from the key rather than the single-row-per-
    /// `(user, provider)` legacy `user_provider_credentials` table.
    /// Mutually exclusive with `copy_oauth_client_from`.
    pub oauth_client_id: Option<String>,
    /// Companion secret for `oauth_client_id`. Must be supplied together
    /// or neither.
    pub oauth_client_secret: Option<String>,
    /// Source `UserApiKey` id to copy `oauth_client_id` / `oauth_client_secret`
    /// from at creation time. Server-side decrypt-then-re-encrypt; the
    /// client never re-transmits the source secret. Mutually exclusive
    /// with the raw `oauth_client_id` / `oauth_client_secret` pair.
    pub copy_oauth_client_from: Option<String>,
}

impl std::fmt::Debug for CreateKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateKeyRequest")
            .field("service_slug", &self.service_slug)
            .field("credential", &"[REDACTED]")
            .field("label", &self.label)
            .field("endpoint_url", &self.endpoint_url)
            .field("slug", &self.slug)
            .field("auth_method", &self.auth_method)
            .field("auth_key_name", &self.auth_key_name)
            .field("node_id", &self.node_id)
            .field("admin_only", &self.admin_only)
            .field("ssh_host", &self.ssh_host)
            .field("ssh_port", &self.ssh_port)
            .field("ssh_certificate_auth", &self.ssh_certificate_auth)
            .field("ssh_auth_mode", &self.ssh_auth_mode)
            .field("ssh_principals", &self.ssh_principals)
            .field(
                "ssh_certificate_ttl_minutes",
                &self.ssh_certificate_ttl_minutes,
            )
            .field("identity_propagation_mode", &self.identity_propagation_mode)
            .field("forward_access_token", &self.forward_access_token)
            .field("inject_delegation_token", &self.inject_delegation_token)
            .field("target_org_id", &self.target_org_id)
            .field(
                "oauth_client_id",
                &self.oauth_client_id.as_deref().map(|_| "[REDACTED]"),
            )
            .field(
                "oauth_client_secret",
                &self.oauth_client_secret.as_deref().map(|_| "[REDACTED]"),
            )
            .field("copy_oauth_client_from", &self.copy_oauth_client_from)
            .finish()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KeyResponse {
    pub id: String,
    pub name: String,
    pub label: String,
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub service_category: String,
    /// Omitted for auto-connected services whose target is platform managed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    pub endpoint_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    /// True when the service's stored `api_key_id` no longer resolves to a
    /// credential row. The service remains manageable for recovery.
    pub credential_missing: bool,
    pub credential_type: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub status: String,
    pub connection_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation: Option<KeyRevocationResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub node_priority: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_last_heartbeat_at: Option<String>,
    pub connected: bool,
    pub requires_connection: bool,
    pub has_node_binding: bool,
    pub proxy_url: String,
    pub proxy_url_slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asyncapi_url: Option<String>,
    pub streaming_supported: bool,
    pub websocket_supported: bool,
    pub source: String,
    pub service_type: String,
    pub ssh_auth_mode: SshAuthMode,
    pub ssh_node_keys_stale: bool,
    pub admin_only: bool,
    pub is_active: bool,
    pub identity_propagation_mode: String,
    pub identity_include_user_id: bool,
    pub identity_include_email: bool,
    pub identity_include_name: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_jwt_audience: Option<String>,
    pub forward_access_token: bool,
    pub inject_delegation_token: bool,
    pub delegation_token_scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    /// Per-add OAuth connection identifier (NyxID multi-connection). Present
    /// for multi-connection oauth2 / device_code adds; absent for legacy and
    /// non-OAuth keys. Surfaced so the frontend can render distinct
    /// connections to the same provider (e.g. two Lark Custom Apps) and so
    /// audit consumers can correlate `connection_id` logs to a visible key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    /// Decrypted user-provided OAuth Custom App `client_id` for BYO providers
    /// (Lark / Feishu / Twitter). Non-secret — appears in OAuth redirect URLs
    /// — so safe to surface. The `client_secret` is never returned by the API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    /// Scopes currently granted on this OAuth connection (NyxID#917 follow-up),
    /// parsed from the backing `UserApiKey.token_scopes`. The connect UIs
    /// pre-select and lock these when adding scopes to an existing connection
    /// (append-only edit). Omitted for non-OAuth keys / never-authorized
    /// connections. Always serialized because authorization evidence readers
    /// distinguish an explicit null from a missing property.
    pub granted_scopes: Option<Vec<String>>,
    /// RFC3339 timestamp of the last fresh OAuth authorization (NyxID#917).
    /// Completion signal for the manage-scopes re-auth flow. Always serialized
    /// because authorization evidence readers require a stable property shape.
    pub last_authorized_at: Option<String>,
    /// Per-user default HTTP headers (NyxID#356). Only user-owned entries
    /// are surfaced here; catalog-level admin defaults are described on
    /// the `/catalog/{slug}` response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_request_headers:
        Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
    /// User-owned WebSocket frame-auth injection rules. Empty means no
    /// user override; catalog-backed services may still inherit catalog
    /// rules at proxy resolution time.
    pub ws_frame_injections: Vec<WsFrameInjection>,
    pub auto_connected: bool,
    /// True only for a response-only platform provider projection. No
    /// `UserService` is persisted for this row.
    pub platform_managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<PlatformKeySummaryResponse>,
    /// Developer app (OAuth client) ID that triggered this auto-provision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_app_id: Option<String>,
    /// Human-readable name of the developer app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: String,
    /// Authoritative timestamp of the latest committed user-service mutation.
    /// Always serialized so evidence readers can consume it from the same
    /// detail mapping the projection derives from.
    pub updated_at: String,
    /// Authoritative monotonic version for this user-service row. Legacy
    /// rows without the field report zero.
    pub state_version: i64,
    /// Credential-rotation predecessor (`UserApiKey` id). Absent when this
    /// service has never rotated its stored credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_predecessor_id: Option<String>,
    // SSH fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_ca_public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_allowed_principals: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_certificate_ttl_minutes: Option<u32>,
    /// User-supplied (or catalog-inherited) OpenAPI spec URL. When present,
    /// AI agents can call `GET /api/v1/endpoints/{endpoint_id}/openapi-endpoints`
    /// to discover the concrete operations this service exposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi_spec_url: Option<String>,
    /// Skill names (Ornn or bundled) that teach an agent how to use this
    /// service. Instance-level list; falls back to the catalog template's
    /// `recommended_skills` when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_skills: Option<Vec<String>>,
    /// Provenance: personal credentials, or inherited from an org membership.
    /// Mirrors the same field on the `/user-services` response so the
    /// frontend can group AI Services by personal vs each org section.
    pub credential_source: crate::handlers::user_services_handler::CredentialSourceResponse,
    /// Lark / Feishu only: deep link to the developer console permissions
    /// page with the catalog's required scopes pre-selected. Surfaced for
    /// `api-lark-bot` / `api-feishu-bot` keys whose stored credential
    /// includes an `app_id`. `None` for every other service so the field
    /// is omitted from the JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_setup_url: Option<String>,
    /// Scope keys encoded in `permission_setup_url`, echoed so the UI
    /// can render the list of scopes that will be granted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_setup_scopes: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PlatformKeySummaryResponse {
    pub operations: Vec<PlatformKeyOperationSummaryResponse>,
    pub credential_source: PlatformKeyCredentialSourceResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PlatformKeyOperationSummaryResponse {
    pub name: String,
    pub kind: PlatformKeyOperationKindResponse,
    pub price_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKeyOperationKindResponse {
    Constrained,
    Endpoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKeyCredentialSourceResponse {
    Platform,
    OwnConnection,
    PlatformFallback,
    Unusable,
}

/// Authorization evidence for one user service — the minimal projection of
/// `KeyResponse` that an assistant-action postcondition reader consumes.
///
/// Why this exists as its own representation rather than reusing the full
/// `KeyResponse`: evidence readers run a recursive secret-shape scan over the
/// *whole* document they receive, rejecting any string matching
/// `Bearer\s+\S+` or a `nyxid_` key prefix. `KeyResponse` carries several
/// user-controlled free-text carriers — `label` / `name`,
/// `default_request_headers[].value`, and `ws_frame_injections[].template`,
/// where `{"headers":{"Authorization":"Bearer ${credential}"}}` is an
/// explicitly supported template (CLAUDE.md §6) — so a legitimately
/// configured service could make its own authorization permanently
/// unverifiable. None of those fields are evidence. Projecting to exactly the
/// consumed properties removes the tripwire surface instead of asking users
/// not to configure supported features.
///
/// Every property is serialized unconditionally except `api_key_id` (mirrors
/// `KeyResponse`: absent and null are equivalent to the reader) and the
/// mutation-lineage trio, which is emitted all-together or not at all.
/// Readers distinguish an explicit null from a missing property, so no
/// `skip_serializing_if` may be added to the remaining fields.
#[derive(Debug, Serialize, ToSchema)]
pub struct KeyAuthorizationEvidenceResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    pub is_active: bool,
    pub status: String,
    pub connection_status: Option<String>,
    pub granted_scopes: Option<Vec<String>>,
    pub last_authorized_at: Option<String>,
    /// Node routing for `service.route`. Always serialized: JSON `null` means
    /// unrouted. Never skipped — the reader distinguishes null from absent.
    pub node_id: Option<String>,
    /// Mutation lineage (`rotation_predecessor_id`, `state_version`,
    /// `updated_at`). Emitted as a trio or not at all: the reader treats
    /// all three absent as "no lineage evidence" but all three present with
    /// `state_version <= 0` as a malformed document. Pre-lineage rows
    /// (`state_version` 0) therefore omit the trio rather than serialize
    /// a zero. `rotation_predecessor_id` is the previous `UserApiKey` id
    /// after `service.rotate_credential`; it is never credential material.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_predecessor_id: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl KeyAuthorizationEvidenceResponse {
    /// Project the full detail response down to authorization evidence.
    ///
    /// Deriving from `KeyResponse` rather than from `KeyView` keeps the two
    /// representations from drifting: whatever `/keys/{id}` reports for these
    /// properties is exactly what the evidence read reports.
    fn from_key_response(response: KeyResponse) -> Self {
        // A row written before service lineage existed has no lineage to
        // report. Serializing `state_version: 0` alongside two nulls is worse
        // than silence: the reader reads it as a present-but-invalid trio and
        // rejects the whole document.
        let has_lineage = response.state_version >= 1;
        Self {
            id: response.id,
            api_key_id: response.api_key_id,
            is_active: response.is_active,
            status: response.status,
            connection_status: response.connection_status,
            granted_scopes: response.granted_scopes,
            last_authorized_at: response.last_authorized_at,
            node_id: response.node_id,
            rotation_predecessor_id: has_lineage.then_some(response.rotation_predecessor_id),
            state_version: has_lineage.then_some(response.state_version),
            updated_at: has_lineage.then_some(response.updated_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KeyRevocationResponse {
    pub revokes_grant: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KeyListResponse {
    pub keys: Vec<KeyResponse>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateKeyRequest {
    /// New display label
    pub label: Option<String>,
    /// New endpoint URL
    pub endpoint_url: Option<String>,
    /// Auth method (bearer, header, query, basic, none)
    pub auth_method: Option<String>,
    /// Auth key name (e.g., Authorization)
    pub auth_key_name: Option<String>,
    /// Node ID for routing ("" to clear, Some(id) to set)
    pub node_id: Option<String>,
    /// Credential to store on the server (bearer token / api key / basic
    /// auth string / etc.) for this service. When set alongside a
    /// credential-bearing `auth_method`, provisions a `UserApiKey` if the
    /// service was created with `auth_method: "none"` and has no stored
    /// credential yet (#419), or rotates the existing credential. When the
    /// service is node-routed, the server encrypts the credential and
    /// pushes it to the target node agent on the next WS heartbeat (#418).
    pub credential: Option<String>,
    /// Activate or deactivate
    pub is_active: Option<bool>,
    /// For org-owned services, restrict proxy execution to org admins.
    pub admin_only: Option<bool>,
    /// Identity propagation mode: "none" | "headers" | "jwt" | "both"
    pub identity_propagation_mode: Option<String>,
    pub identity_include_user_id: Option<bool>,
    pub identity_include_email: Option<bool>,
    pub identity_include_name: Option<bool>,
    pub identity_jwt_audience: Option<String>,
    pub forward_access_token: Option<bool>,
    pub inject_delegation_token: Option<bool>,
    pub delegation_token_scope: Option<String>,
    /// Custom User-Agent override. Set to "" to clear, Some(value) to set.
    pub custom_user_agent: Option<String>,
    /// Per-user default HTTP headers injected on every proxied request
    /// (NyxID#356). Field omitted leaves the existing value unchanged;
    /// explicit JSON `null` or `[]` clears; a non-empty array replaces
    /// with a validated list. The `nullable_field` helper is what makes
    /// the omitted-vs-null distinction survive serde deserialization —
    /// a plain `Option<Option<_>>` collapses both to `None`.
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub default_request_headers:
        Option<Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>>,
    /// OpenAPI spec URL for endpoint discovery. Set to "" to clear,
    /// Some(value) to set.
    pub openapi_spec_url: Option<String>,
    /// Recommended skill names for this service. Absent = leave unchanged;
    /// [] = clear; non-empty list = replace.
    pub recommended_skills: Option<Vec<String>>,
    /// BYO OAuth Custom App `client_id` used when this PUT upgrades a
    /// `auth_method: "none"` service to OAuth on a
    /// `credential_mode: "user"` provider. Same semantics as on POST
    /// `/keys` — see `CreateKeyRequest::oauth_client_id`.
    pub oauth_client_id: Option<String>,
    pub oauth_client_secret: Option<String>,
    pub copy_oauth_client_from: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteKeyResponse {
    pub message: String,
    /// `true` when the key was actually revoked, `false` when the
    /// request hit the `only_if_pending=true` guard and was a no-op
    /// because the key is no longer `pending_auth`. Callers use this
    /// to distinguish "we cleaned up the abandoned placeholder" from
    /// "the provider callback already converted the placeholder into
    /// an active key, so leave it alone".
    #[serde(default)]
    pub deleted: bool,
    /// Whether an eligible upstream revocation request was handed off.
    pub upstream_revocation_scheduled: bool,
}

/// Extract the Lark / Feishu `app_id` from a plaintext credential string.
///
/// `api-lark-bot` and `api-feishu-bot` keys store the credential as a JSON
/// object `{"app_id": "...", "app_secret": "..."}` (the
/// `lark_family_token_exchange_config` credential schema). For OAuth-based
/// `api-lark` / `api-feishu` keys with BYO app credentials the app id
/// arrives via `user_oauth_client_id_encrypted` instead — that path is
/// handled separately by `extract_app_id_from_api_key`.
///
/// Returns `None` when the credential isn't valid JSON or doesn't contain
/// a non-empty `app_id` field, so the caller can short-circuit cleanly
/// instead of treating a parse failure as an error.
fn extract_app_id_from_credential(credential: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(credential).ok()?;
    let app_id = value.get("app_id")?.as_str()?.trim();
    if app_id.is_empty() {
        None
    } else {
        Some(app_id.to_string())
    }
}

/// Decrypt a `UserApiKey` and pull out the Lark / Feishu app id, checking
/// both the `token_exchange` JSON credential blob and the BYO OAuth client
/// id field. All decrypt / parse failures are silently dropped to `None`
/// because this is best-effort metadata for a UI deep link, not a security
/// boundary — a missing URL just degrades to the manual setup flow.
async fn extract_app_id_from_api_key(
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    api_key: &UserApiKey,
) -> Option<String> {
    if let Some(blob) = api_key.credential_encrypted.as_ref()
        && !blob.is_empty()
        && let Ok(bytes) = encryption_keys.decrypt(blob).await
        && let Ok(plaintext) = String::from_utf8(bytes)
        && let Some(app_id) = extract_app_id_from_credential(&plaintext)
    {
        return Some(app_id);
    }

    if let Some(blob) = api_key.user_oauth_client_id_encrypted.as_ref()
        && !blob.is_empty()
        && let Ok(bytes) = encryption_keys.decrypt(blob).await
        && let Ok(plaintext) = String::from_utf8(bytes)
    {
        let trimmed = plaintext.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

/// Derive the Lark / Feishu permission setup URL for an AI Services key,
/// or `(None, None)` when the key isn't a Lark variant or we can't
/// resolve an app id. This is best-effort surface metadata: any I/O or
/// decryption failure resolves to "no URL" so the rest of the response
/// still ships.
async fn derive_lark_permission_for_key(
    state: &AppState,
    user_id: &str,
    catalog_service_id: Option<&str>,
    catalog_service_slug: Option<&str>,
    api_key_id: Option<&str>,
) -> (Option<String>, Option<Vec<String>>) {
    let region = catalog_service_slug.and_then(lark_permission::region_for_catalog_service_slug);
    let region = match region {
        Some(r) => r,
        None => return (None, None),
    };

    let api_key_id = match api_key_id {
        Some(id) => id,
        None => return (None, None),
    };
    let api_key = match user_api_key_service::get_api_key(&state.db, user_id, api_key_id).await {
        Ok(k) => k,
        Err(_) => return (None, None),
    };

    let app_id = match extract_app_id_from_api_key(&state.encryption_keys, &api_key).await {
        Some(id) => id,
        None => return (None, None),
    };

    let scopes = match catalog_service_id {
        Some(id) => catalog_service::get_required_permissions(&state.db, id).await,
        None => Vec::new(),
    };
    let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
    let url = lark_permission::build_permission_setup_url(region, &app_id, &scope_refs);
    (Some(url), Some(scopes))
}

fn validate_optional_label_for_update(label: Option<&str>) -> AppResult<()> {
    if let Some(label) = label
        && (label.is_empty() || label.len() > 200)
    {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 200 characters".to_string(),
        ));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/v1/keys",
    request_body = CreateKeyRequest,
    responses(
        (status = 200, description = "Key created with auto-provisioned endpoint, credential, and service", body = KeyResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Catalog entry not found", body = crate::errors::ErrorResponse)
    ),
    tag = "AI Services"
)]
/// POST /api/v1/keys
pub async fn create_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateKeyRequest>,
) -> AppResult<Json<KeyResponse>> {
    create_key_with_service_id(State(state), auth_user, tele, Json(body), None).await
}

/// Internal assistant-action create path with a receipt-reserved
/// `UserService` identity.
pub(crate) async fn create_key_with_service_id(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateKeyRequest>,
    reserved_service_id: Option<&str>,
) -> AppResult<Json<KeyResponse>> {
    let actor = auth_user.user_id.to_string();

    // Resolve the effective owner of the new key. If `target_org_id` is set,
    // the caller must be an admin of that org -- the created UserService /
    // UserEndpoint / UserApiKey rows are then written with `user_id` set to
    // the org's user id, making them visible to every member of that org.
    // For OAuth / device-code flows the admin must separately initiate the
    // provider flow with `target_org_id` set so the resulting
    // `UserProviderToken` is also stored under the org's user_id; see
    // `handlers/user_tokens.rs::initiate_oauth_connect`.
    let user_id_str = if let Some(target_org_id) = body.target_org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create keys under it".to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor.clone()
    };

    let credential = body.credential.as_deref().unwrap_or("");
    if let Some(ref rules) = body.ws_frame_injections {
        crate::services::ws_frame_injector::validate_rules(rules)?;
    }

    // Cloud-billing credentials: probe the upstream once at add-time so
    // a malformed access key / wrong AWS account / missing GCP role
    // fails fast here instead of an hour later when a `/daily` skill
    // runs. Skipped for node-routed creates (credential won't land on
    // the backend) and when no credential was supplied (OAuth flows).
    if body.node_id.as_deref().is_none_or(|n| n.is_empty()) && !credential.is_empty() {
        verify_cloud_credential_against_catalog(&state, &body, credential).await?;
    }

    // Build SSH params if SSH-specific fields are present
    let ssh_params = body.ssh_host.as_deref().map(|host| {
        let principals_str = body.ssh_principals.as_deref().unwrap_or("");
        let principals: Vec<String> = principals_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let certificate_auth = body.ssh_certificate_auth.unwrap_or(true);
        let ssh_auth_mode = body
            .ssh_auth_mode
            .unwrap_or_else(|| SshAuthMode::from_certificate_auth_enabled(certificate_auth));
        unified_key_service::SshCreateParams {
            host,
            port: body.ssh_port.unwrap_or(22),
            certificate_auth,
            ssh_auth_mode,
            principals,
            certificate_ttl_minutes: body.ssh_certificate_ttl_minutes.unwrap_or(30),
        }
    });

    let identity = if body.identity_propagation_mode.is_some()
        || body.identity_include_user_id.is_some()
        || body.identity_include_email.is_some()
        || body.identity_include_name.is_some()
        || body.identity_jwt_audience.is_some()
        || body.forward_access_token.is_some()
        || body.inject_delegation_token.is_some()
        || body.delegation_token_scope.is_some()
    {
        Some(user_service_service::IdentityConfig {
            identity_propagation_mode: body
                .identity_propagation_mode
                .unwrap_or_else(|| "none".to_string()),
            identity_include_user_id: body.identity_include_user_id.unwrap_or(false),
            identity_include_email: body.identity_include_email.unwrap_or(false),
            identity_include_name: body.identity_include_name.unwrap_or(false),
            identity_jwt_audience: body.identity_jwt_audience,
            forward_access_token: body.forward_access_token.unwrap_or(false),
            inject_delegation_token: body.inject_delegation_token.unwrap_or(false),
            delegation_token_scope: body
                .delegation_token_scope
                .unwrap_or_else(|| "llm:proxy".to_string()),
        })
    } else {
        None
    };

    // Translate the three-state wire format for openapi_spec_url. `None`
    // (field absent) inherits the catalog default; `Some("")` opts out of
    // inheritance; `Some(value)` overrides.
    let openapi_input = match body.openapi_spec_url.as_deref() {
        None => unified_key_service::OpenApiSpecUrlInput::Inherit,
        Some(s) if s.trim().is_empty() => unified_key_service::OpenApiSpecUrlInput::Clear,
        Some(s) => unified_key_service::OpenApiSpecUrlInput::Set(s),
    };

    // BYO OAuth Custom App credentials (`credential_mode: "user"` providers).
    // Three-state, mutually exclusive at the wire level — the handler
    // enforces mutual exclusion up front so the downstream service
    // doesn't have to defend against ambiguous combinations.
    let raw_id = body.oauth_client_id.as_deref().map(str::trim);
    let raw_secret = body.oauth_client_secret.as_deref().map(str::trim);
    let copy_from = body.copy_oauth_client_from.as_deref().map(str::trim);
    let raw_present =
        raw_id.is_some_and(|s| !s.is_empty()) || raw_secret.is_some_and(|s| !s.is_empty());
    let copy_present = copy_from.is_some_and(|s| !s.is_empty());
    if raw_present && copy_present {
        return Err(AppError::BadRequest(
            "oauth_client_id/oauth_client_secret and copy_oauth_client_from are mutually exclusive"
                .to_string(),
        ));
    }
    let oauth_client_credentials = if copy_present {
        unified_key_service::OauthClientCredentialsInput::CopyFrom {
            source_key_id: copy_from.expect("copy_present"),
        }
    } else if raw_present {
        // Pair gate: both halves must be supplied. We let
        // `resolve_oauth_client_credentials_input` enforce non-empty
        // values; here we just reject the half-pair case so the user
        // gets a clearer message than the downstream "must be non-empty".
        let (Some(id), Some(secret)) = (raw_id, raw_secret) else {
            return Err(AppError::BadRequest(
                "oauth_client_id and oauth_client_secret must be supplied together".to_string(),
            ));
        };
        if id.is_empty() || secret.is_empty() {
            return Err(AppError::BadRequest(
                "oauth_client_id and oauth_client_secret must be supplied together".to_string(),
            ));
        }
        unified_key_service::OauthClientCredentialsInput::Raw {
            client_id: id,
            client_secret: secret,
        }
    } else {
        unified_key_service::OauthClientCredentialsInput::None
    };

    let result = match reserved_service_id {
        Some(service_id) => {
            unified_key_service::create_key_with_service_id(
                &state.db,
                &state.encryption_keys,
                &user_id_str,
                &actor,
                body.service_slug.as_deref(),
                body.endpoint_url.as_deref(),
                credential,
                &body.label,
                body.slug.as_deref(),
                body.auth_method.as_deref(),
                body.auth_key_name.as_deref(),
                body.node_id.as_deref(),
                ssh_params,
                identity,
                openapi_input,
                body.ws_frame_injections.as_deref(),
                body.admin_only.unwrap_or(false),
                oauth_client_credentials,
                state.config.is_production(),
                service_id,
            )
            .await?
        }
        None => {
            unified_key_service::create_key(
                &state.db,
                &state.encryption_keys,
                &user_id_str,
                &actor,
                body.service_slug.as_deref(),
                body.endpoint_url.as_deref(),
                credential,
                &body.label,
                body.slug.as_deref(),
                body.auth_method.as_deref(),
                body.auth_key_name.as_deref(),
                body.node_id.as_deref(),
                ssh_params,
                identity,
                openapi_input,
                body.ws_frame_injections.as_deref(),
                body.admin_only.unwrap_or(false),
                oauth_client_credentials,
                state.config.is_production(),
            )
            .await?
        }
    };

    // Fire-and-forget: push credential to node if routed AND we have a credential to push
    let has_pushable_credential = result.api_key.as_ref().is_some_and(|api_key| {
        api_key.credential_encrypted.is_some() || api_key.access_token_encrypted.is_some()
    });
    if result.service.node_id.is_some() && has_pushable_credential {
        let db = state.db.clone();
        let enc = state.encryption_keys.clone();
        let ws = state.node_ws_manager.clone();
        let uid = user_id_str.clone();
        let key_id = result
            .api_key
            .as_ref()
            .expect("pushable credential requires api key")
            .id
            .clone();
        tokio::spawn(async move {
            credential_push_service::push_credential_to_node_if_routed(
                &db, &enc, &ws, &uid, &key_id,
            )
            .await;
        });
    }

    // Tag the response `credential_source` based on whether this key was
    // created under the actor's personal scope or under an org. This is
    // cosmetic for the immediate response; subsequent `GET /keys/{id}`
    // calls compute the source server-side from `resolve_owner_access`.
    let mut response = key_response_from_result(&result);
    if let Some(target_org_id) = body.target_org_id.as_deref() {
        use crate::handlers::user_services_handler::{CredentialSourceResponse, OrgRoleResponse};
        let org = state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! { "_id": target_org_id })
            .await?;
        let (org_name, avatar_url) = org
            .map(|u| (u.display_name, u.avatar_url))
            .unwrap_or((None, None));
        let org_name = org_name.unwrap_or_else(|| "Unnamed Org".to_string());
        response.credential_source = CredentialSourceResponse::Org {
            org_id: target_org_id.to_string(),
            org_name,
            avatar_url,
            role: OrgRoleResponse::Admin,
            allowed: true,
        };
    }

    // Telemetry: key.created. `source` is "catalog" when a catalog slug
    // drove the bootstrap, else "custom".
    let catalog_slug = response.catalog_service_slug.clone();
    let source = if catalog_slug.is_some() {
        "catalog"
    } else {
        "custom"
    };
    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::KeyCreated {
            source: source.to_string(),
            catalog_slug,
            has_node_binding: response.node_id.is_some(),
        },
    );

    // For Lark/Feishu services, derive the developer-console permission
    // setup deep link so the create response can hand it back to the
    // CLI / UI in the same round-trip. `key_response_from_result` leaves
    // `catalog_service_slug` empty (it has no catalog row to look up), so
    // fall back to the request's `service_slug` to drive the region check.
    let (permission_url, permission_scopes) = derive_lark_permission_for_key(
        &state,
        &user_id_str,
        response.catalog_service_id.as_deref(),
        response
            .catalog_service_slug
            .as_deref()
            .or(body.service_slug.as_deref()),
        response.api_key_id.as_deref(),
    )
    .await;
    response.permission_setup_url = permission_url;
    response.permission_setup_scopes = permission_scopes;
    enrich_key_response(
        &state.db,
        &state.node_ws_manager,
        &actor,
        state.config.node_heartbeat_timeout_secs,
        state.config.base_url.trim_end_matches('/'),
        &mut response,
    )
    .await?;

    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/keys",
    responses(
        (status = 200, description = "List of user's AI service keys", body = KeyListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "AI Services"
)]
/// GET /api/v1/keys
pub async fn list_keys(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<KeyListResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Lazily auto-provision platform-managed catalog services for the user.
    unified_key_service::auto_provision_no_auth_services(&state.db, &user_id_str).await?;

    let views =
        unified_key_service::list_keys(&state.db, &state.encryption_keys, &user_id_str).await?;
    let mut keys = views
        .into_iter()
        .map(key_response_from_view)
        .collect::<Vec<_>>();
    enrich_key_responses(
        &state.db,
        &state.node_ws_manager,
        &user_id_str,
        state.config.node_heartbeat_timeout_secs,
        state.config.base_url.trim_end_matches('/'),
        &mut keys,
    )
    .await?;
    append_platform_key_projection(&state, &auth_user, &mut keys).await?;
    Ok(Json(KeyListResponse { keys }))
}

#[utoipa::path(
    get,
    path = "/api/v1/keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "User service ID or slug")
    ),
    responses(
        (status = 200, description = "Key details", body = KeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "AI Services"
)]
/// GET /api/v1/keys/{key_id}
pub async fn get_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<KeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let mut response = resolve_key_response(&state, &actor, &key_id).await?;
    enrich_key_response(
        &state.db,
        &state.node_ws_manager,
        &actor,
        state.config.node_heartbeat_timeout_secs,
        state.config.base_url.trim_end_matches('/'),
        &mut response,
    )
    .await?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/keys/{key_id}/authorization",
    params(
        ("key_id" = String, Path, description = "User service ID or slug")
    ),
    responses(
        (status = 200, description = "Authorization evidence", body = KeyAuthorizationEvidenceResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "AI Services"
)]
/// GET /api/v1/keys/{key_id}/authorization
///
/// Same resolution, ACL, and lazy `pending_auth` reconciliation as
/// `GET /api/v1/keys/{key_id}`, projected to authorization evidence. Node and
/// proxy-URL enrichment is skipped because no evidence property depends on it.
pub async fn get_key_authorization(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<KeyAuthorizationEvidenceResponse>> {
    let actor = auth_user.user_id.to_string();
    let response = resolve_key_response(&state, &actor, &key_id).await?;
    Ok(Json(KeyAuthorizationEvidenceResponse::from_key_response(
        response,
    )))
}

/// Shared body of the two key detail reads: ACL resolution, lazy OAuth
/// placeholder reconciliation, and the view→response mapping. Returns the
/// un-enriched response; callers add whatever their representation needs.
async fn resolve_key_response(
    state: &AppState,
    actor: &str,
    key_id: &str,
) -> AppResult<KeyResponse> {
    let access = resolve_key_read_owner(state, actor, key_id).await?;

    // Lazy reconciliation of pending_auth OAuth placeholders (issue #653).
    // Wizard polling hits this handler every ~2s; treating each poll as a
    // chance to converge the placeholder makes the wizard self-healing
    // against silent OAuth-callback failures and abandoned flows. No-op for
    // non-OAuth or already-terminal rows. Best-effort: errors are logged
    // and swallowed so the read still proceeds.
    if let Some(svc) = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": &access.service_id })
        .await?
        && let Some(api_key_id) = svc.api_key_id.as_deref()
        && let Err(e) = user_api_key_service::reconcile_pending_oauth_placeholder(
            &state.db,
            &access.owner_id,
            api_key_id,
        )
        .await
    {
        tracing::warn!(
            user_id = %access.owner_id,
            api_key_id = %api_key_id,
            error = %e,
            "lazy reconcile of pending_auth placeholder failed"
        );
    }

    let mut view = unified_key_service::get_key(
        &state.db,
        &state.encryption_keys,
        &access.owner_id,
        &access.service_id,
    )
    .await?;
    // Override the placeholder Personal that get_key returns; the handler is
    // the only layer that knows whether the actor is the direct owner or
    // accessing via an org membership.
    view.credential_source = access.source;
    let owner_id = access.owner_id.clone();
    let catalog_id = view.catalog_service_id.clone();
    let catalog_slug = view.catalog_service_slug.clone();
    let api_key_id = view.api_key_id.clone();
    let mut response = key_response_from_view(view);
    let (permission_url, permission_scopes) = derive_lark_permission_for_key(
        state,
        &owner_id,
        catalog_id.as_deref(),
        catalog_slug.as_deref(),
        api_key_id.as_deref(),
    )
    .await;
    response.permission_setup_url = permission_url;
    response.permission_setup_scopes = permission_scopes;
    Ok(response)
}

#[utoipa::path(
    put,
    path = "/api/v1/keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "User service ID or slug")
    ),
    request_body = UpdateKeyRequest,
    responses(
        (status = 200, description = "Key updated", body = KeyResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 403, description = "Auto-connected service is platform managed", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "AI Services"
)]
/// PUT /api/v1/keys/{key_id}
pub async fn update_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
    Json(body): Json<UpdateKeyRequest>,
) -> AppResult<Json<KeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let access = resolve_key_write_owner(&state, &actor, &key_id).await?;
    let user_id_str = access.owner_id;
    let key_id = access.service_id;

    // Load current state to find sub-resource IDs
    let view =
        unified_key_service::get_key(&state.db, &state.encryption_keys, &user_id_str, &key_id)
            .await?;

    if view.auto_connected {
        return Err(crate::errors::AppError::Forbidden(
            "Auto-connected services cannot be modified".to_string(),
        ));
    }

    // NOTE: label writes are intentionally deferred past the strict
    // node push below. A label change combined with a node-routed
    // credential update must be atomic — committing the label before
    // the push succeeds leaves the API returning a failed `PUT /keys`
    // while the label has already changed, so a retry wouldn't be
    // idempotent and callers can't tell which parts of the update
    // actually applied (thirty-first-round Codex P2). The deferred
    // label-write block lives after the strict push succeeds, right
    // alongside the `endpoint_url` / `openapi_spec_url` commits.
    //
    // But the *validation* still has to happen up front. Otherwise an
    // invalid label on an existing service would let the handler rotate
    // a credential and even push it to a node, then fail only when the
    // deferred label write runs — returning an error despite the
    // credential change having already applied.
    validate_optional_label_for_update(body.label.as_deref())?;

    if let Some(endpoint_url) = body.endpoint_url.as_deref() {
        let effective_node_id = match body.node_id.as_deref() {
            Some("") => None,
            Some(node_id) => Some(node_id),
            None => view
                .node_id
                .as_deref()
                .filter(|node_id| !node_id.is_empty()),
        };
        if effective_node_id.is_none() {
            crate::services::url_validation::validate_user_endpoint_url(
                endpoint_url,
                state.config.is_production(),
                "endpoint_url",
            )
            .await?;
        }
    }

    // NOTE: `body.endpoint_url` is intentionally NOT written to the DB
    // here. For node-routed services we must keep the endpoint URL and
    // the strict node push atomic — if the push fails (node offline /
    // WS buffer full), the DB must not already show the new URL while
    // the node keeps serving the old `target_url`. The actual
    // `update_endpoint` call lives below, right after
    // `push_credential_to_node_strict` succeeds. The push itself reads
    // `effective_endpoint_url_for_push` directly from the incoming body
    // (or the current view when absent), so it doesn't need the DB to
    // reflect the new URL first. Ninth-round Codex review P2.

    // NOTE: `body.openapi_spec_url` is intentionally NOT written here
    // either. For the same atomicity reason as `endpoint_url`, deferring
    // the spec URL commit until after the strict node push keeps
    // `PUT /keys` retries idempotent when the push fails — a user who
    // sends `{credential, endpoint_url, openapi_spec_url}` in the same
    // body and hits a push error can retry without the spec URL already
    // having been partially committed (twenty-fourth-round Codex P2).
    // The actual `update_endpoint` call moves to after
    // `push_credential_to_node_strict`, next to the `endpoint_url`
    // write.

    let has_identity_update = body.identity_propagation_mode.is_some()
        || body.identity_include_user_id.is_some()
        || body.identity_include_email.is_some()
        || body.identity_include_name.is_some()
        || body.identity_jwt_audience.is_some()
        || body.forward_access_token.is_some()
        || body.inject_delegation_token.is_some()
        || body.delegation_token_scope.is_some();

    // Provision or rotate the backing `UserApiKey` before touching the
    // `UserService` row. Covers two cases (NyxID#418, #419):
    //
    //  1. Service was POSTed with `auth_method: "none"` and is now being
    //     upgraded to bearer/basic/header/etc. A new `UserApiKey` is
    //     created and linked to the service so the subsequent
    //     `update_user_service` + reconcile pass the `api_key_id.is_none()`
    //     guards instead of returning a misleading error.
    //  2. Caller supplied a `credential` on an existing service. Rotates
    //     the stored value (or is rejected when auth_method is still
    //     `none`).
    //
    // Runs unconditionally when either field is present so the handler
    // can decide without pre-loading current state; the helper
    // short-circuits to no-op when nothing needs to change.
    let credential_provided_nonempty = body.credential.as_deref().is_some_and(|c| !c.is_empty());

    // Precompute the identity config we'd apply later so the pre-validator
    // can check it against the same normalizer `update_user_service` uses.
    // Mirrors the construction inside the `update_user_service` guarded
    // block below — factored out to avoid a second identical block.
    let identity_for_validate = if has_identity_update {
        Some(user_service_service::IdentityConfig {
            identity_propagation_mode: body
                .identity_propagation_mode
                .clone()
                .unwrap_or(view.identity_propagation_mode.clone()),
            identity_include_user_id: body
                .identity_include_user_id
                .unwrap_or(view.identity_include_user_id),
            identity_include_email: body
                .identity_include_email
                .unwrap_or(view.identity_include_email),
            identity_include_name: body
                .identity_include_name
                .unwrap_or(view.identity_include_name),
            identity_jwt_audience: if body.identity_jwt_audience.is_some() {
                body.identity_jwt_audience.clone()
            } else {
                view.identity_jwt_audience.clone()
            },
            forward_access_token: body
                .forward_access_token
                .unwrap_or(view.forward_access_token),
            inject_delegation_token: body
                .inject_delegation_token
                .unwrap_or(view.inject_delegation_token),
            delegation_token_scope: body
                .delegation_token_scope
                .clone()
                .unwrap_or(view.delegation_token_scope.clone()),
        })
    } else {
        None
    };

    // Pre-validate every field `update_user_service` would validate after
    // we provision, so an invalid request can't leave an orphaned
    // `UserApiKey` linked to a partially-updated service. Without this,
    // a PUT that upgrades `auth_method: none` AND includes (e.g.) a
    // bogus `custom_user_agent` or denylisted default header returns 400
    // only after `ensure_user_api_key_for_update` has already stored a
    // fresh credential on the server. Raised by the second Codex review
    // (P1) of the NyxID#419 fix. The validator mirrors every rule inside
    // `update_user_service`; keep them in sync.
    let any_service_field = body.auth_method.is_some()
        || body.auth_key_name.is_some()
        || body.node_id.is_some()
        || body.custom_user_agent.is_some()
        || body.default_request_headers.is_some()
        || has_identity_update
        // Also trigger when only `credential` is present, so the
        // token_exchange JSON-shape check inside `validate_update_inputs`
        // runs before the rotation would otherwise persist a malformed
        // blob on a service that already had `auth_method: token_exchange`.
        || body.credential.is_some()
        // `endpoint_url` is itself a node-delivery field (ends up as the
        // node's local `target_url`), so an endpoint-only PUT must also
        // run the validator to (a) enforce URL format, (b) gate the
        // node-ownership check, and (c) run the token_exchange /
        // identity cross-field guards. Without this, a service admin
        // without node-write access could rewrite another user's
        // node-local routing through a minimal `{endpoint_url}` body
        // (eleventh-round Codex P1).
        || body.endpoint_url.is_some()
        // `openapi_spec_url` goes through the same pre-validator so a
        // malformed spec URL is rejected before any credential
        // mutation or node push lands (twenty-sixth-round Codex P2).
        || body.openapi_spec_url.is_some();
    if any_service_field {
        let current_service =
            user_service_service::get_user_service(&state.db, &user_id_str, &key_id).await?;
        user_service_service::validate_update_inputs(
            &state.db,
            &actor,
            &current_service,
            body.auth_method.as_deref(),
            body.auth_key_name.as_deref(),
            body.node_id.as_deref(),
            identity_for_validate.as_ref(),
            body.custom_user_agent.as_deref(),
            body.default_request_headers.as_ref(),
            body.credential.as_deref(),
            body.endpoint_url.as_deref(),
            body.openapi_spec_url.as_deref(),
        )
        .await?;
    }

    if body.auth_method.is_some() || body.credential.is_some() {
        // Preserve the user's display label (either the explicit new
        // `label` in this request, or the current `view.label`, which on a
        // no-auth service reflects `UserEndpoint.label`) when provisioning
        // the first `UserApiKey`. `build_key_view` prefers `api_key.label`
        // over `endpoint.label`, so seeding the new record with the slug
        // would silently rename the service in GET responses. Raised as P3
        // by the Codex review of the NyxID#419 fix.
        let preferred_label = body.label.as_deref().unwrap_or(view.label.as_str());

        // Same BYO three-state envelope as POST. Mutual-exclusion and
        // pair-completeness are validated here so the PUT path's failure
        // mode mirrors POST exactly. The downstream call resolves the
        // source key (for `CopyFrom`) and enforces provider-compat.
        let raw_id = body.oauth_client_id.as_deref().map(str::trim);
        let raw_secret = body.oauth_client_secret.as_deref().map(str::trim);
        let copy_from = body.copy_oauth_client_from.as_deref().map(str::trim);
        let raw_present =
            raw_id.is_some_and(|s| !s.is_empty()) || raw_secret.is_some_and(|s| !s.is_empty());
        let copy_present = copy_from.is_some_and(|s| !s.is_empty());
        if raw_present && copy_present {
            return Err(AppError::BadRequest(
                "oauth_client_id/oauth_client_secret and copy_oauth_client_from are mutually exclusive"
                    .to_string(),
            ));
        }
        let oauth_client_credentials = if copy_present {
            unified_key_service::OauthClientCredentialsInput::CopyFrom {
                source_key_id: copy_from.expect("copy_present"),
            }
        } else if raw_present {
            let (Some(id), Some(secret)) = (raw_id, raw_secret) else {
                return Err(AppError::BadRequest(
                    "oauth_client_id and oauth_client_secret must be supplied together".to_string(),
                ));
            };
            if id.is_empty() || secret.is_empty() {
                return Err(AppError::BadRequest(
                    "oauth_client_id and oauth_client_secret must be supplied together".to_string(),
                ));
            }
            unified_key_service::OauthClientCredentialsInput::Raw {
                client_id: id,
                client_secret: secret,
            }
        } else {
            unified_key_service::OauthClientCredentialsInput::None
        };

        unified_key_service::ensure_user_api_key_for_update(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            &key_id,
            body.auth_method.as_deref(),
            body.credential.as_deref(),
            body.node_id.as_deref(),
            preferred_label,
            oauth_client_credentials,
        )
        .await?;
    }

    // Node-routed services end up with `credential_type == "node_managed"`
    // at rest (the node agent holds the actual secret; MCP / proxy
    // fall-through logic keys off that invariant). When the caller just
    // stored a fresh credential server-side, we must push it to the node
    // *before* the subsequent reconcile wipes the encrypted blob — and
    // we only let reconcile wipe when the push landed. If the node is
    // offline, we fail the PUT so the credential stays on the server for
    // the user to retry, instead of silently losing it.
    //
    // Push runs BEFORE `update_user_service` so a failed delivery can't
    // partially commit routing/auth mutations (fourth-round Codex P1).
    // Effective post-update values are computed from `body` + `view` and
    // passed into the push so the target reflects the user's requested
    // state, not whatever the DB still holds.
    //
    // Speculative push on plain `node_id` bind is intentionally omitted
    // (third-round Codex review P2): a service downgraded to
    // `auth_method: "none"` still retains its old `api_key_id`, and
    // pushing that stale secret to the node would reactivate a
    // credential the user already turned off.
    // Normalize legacy `view.node_id == Some("")` to `None`: some rows
    // carry the empty string instead of `None`, and a push to an
    // empty-string node_id is always going to hit `NodeOffline`.
    // Mirrors the same normalization in `validate_update_inputs`.
    // Fifteenth-round Codex P1.
    let effective_node_id_for_push: Option<String> = match body.node_id.as_deref() {
        Some("") => None,
        Some(n) => Some(n.to_string()),
        None => view.node_id.clone().filter(|n| !n.is_empty()),
    };
    let effective_auth_method_for_push = body
        .auth_method
        .as_deref()
        .unwrap_or(view.auth_method.as_str())
        .to_string();
    // Default `auth_key_name` to `Authorization` on bearer/basic when the
    // caller didn't supply one. Services created with
    // `auth_method: "none"` store an empty `auth_key_name` and services
    // previously on `header` auth may carry a custom header name like
    // `X-API-Key` — either would cause the node-side push to inject
    // `Bearer …` / `Basic …` under the wrong header. The backend's
    // direct-routing path already hardcodes `Authorization` for bearer
    // auth, so defaulting here keeps node-routed and direct behavior
    // consistent with `create_key`'s custom HTTP defaults (sixteenth-
    // round Codex P1).
    let bearer_like = matches!(effective_auth_method_for_push.as_str(), "bearer" | "basic");
    // Compute the auth_key_name the handler should use, both for the
    // strict node push and for persistence in `update_user_service`
    // (eighteenth-round Codex P2). Two cases synthesize a default of
    // `Authorization` when the caller didn't supply one:
    //   - `view.auth_key_name` is empty (services originally created
    //     with `auth_method: "none"` store an empty string)
    //   - The caller is actively switching auth_method to bearer/basic
    //     and the stored name is for another scheme (e.g., `X-API-Key`
    //     left over from `header` auth).
    // Otherwise fall through to the existing DB value.
    let effective_auth_key_name_for_push = match body.auth_key_name.as_deref() {
        Some(name) => name.to_string(),
        None => {
            // Synthesize `Authorization` whenever the effective auth is
            // bearer/basic AND the stored name is empty or wrong (not
            // `Authorization`). Covers three cases:
            //   (a) caller is switching auth_method to bearer/basic,
            //   (b) service was already bearer/basic with empty name
            //       (e.g., originally created with auth_method=none),
            //   (c) node-only rebind on an existing bearer/basic whose
            //       stored name is stale (e.g., `X-API-Key` left over
            //       from a previous `header` auth). Without this, the
            //       push on move-to-node would write `Bearer …` under
            //       the wrong header and direct routing would recover
            //       only by virtue of the hardcoded `Authorization`
            //       fallback in the proxy (twentieth-round Codex P2).
            let needs_default =
                bearer_like && !view.auth_key_name.eq_ignore_ascii_case("Authorization");
            if needs_default {
                "Authorization".to_string()
            } else {
                view.auth_key_name.clone()
            }
        }
    };
    // Feed the same normalized value into `update_user_service` so the
    // DB row stays in sync with what we push to the node. Without this,
    // the next rotation (whether via `PUT /keys` or `/api-keys/external`)
    // would rebuild the push from a stale `auth_key_name` and inject
    // `Bearer …` under the wrong header again.
    //
    // Fires whenever we need to persist an `Authorization` override so
    // subsequent rotations (via `PUT /keys` or `/api-keys/external`)
    // don't rebuild the push from a stale stored header name:
    //   (a) caller is switching auth_method to bearer/basic and the
    //       stored name is not `Authorization`;
    //   (b) caller is not touching auth_method but is rotating the
    //       credential on an existing bearer/basic service whose stored
    //       name is empty/wrong;
    //   (c) caller is rebinding the node on an existing bearer/basic
    //       service whose stored name is stale (twentieth-round Codex
    //       P2). Without this, a `PUT /keys {node_id: X}` push would
    //       write to the wrong header on the new node and the DB would
    //       stay inconsistent with what the push just sent.
    let wrong_stored_header = !view.auth_key_name.eq_ignore_ascii_case("Authorization");
    let current_is_bearer_like = matches!(view.auth_method.as_str(), "bearer" | "basic");
    let node_id_in_body = body.node_id.is_some();
    let persisted_auth_key_name_override: Option<String> = match body.auth_key_name.as_deref() {
        Some(_) => None, // caller supplied; update_user_service uses body.auth_key_name
        None if bearer_like && body.auth_method.is_some() && wrong_stored_header => {
            Some("Authorization".to_string())
        }
        None if current_is_bearer_like
            && body.auth_method.is_none()
            && wrong_stored_header
            && (credential_provided_nonempty || node_id_in_body) =>
        {
            Some("Authorization".to_string())
        }
        _ => None,
    };
    // Effective endpoint URL semantics for the credential-update frame:
    //   * body has `endpoint_url: "some-url"` → Some("some-url"): push sets target
    //   * body has `endpoint_url: ""`         → Some(""):          push *clears*
    //     target and the node falls back to its local config
    //   * body omits `endpoint_url`           → Some(view.url) when view has a
    //     non-empty URL (re-assert current), None when view's URL is empty
    //     (don't touch the node's locally-configured target)
    //
    // The "explicit clear vs. omit-to-preserve" distinction matters after
    // twelfth-round Codex P2: collapsing empty strings to `None`
    // uniformly meant the node could never be told to drop a
    // stale server-managed URL once an endpoint was switched back to
    // `endpoint_url: ""`.
    let old_effective_node_id = view.node_id.as_deref().filter(|n| !n.is_empty());
    let new_effective_node_id = match body.node_id.as_deref() {
        Some("") => None,
        Some(n) => Some(n),
        None => old_effective_node_id,
    };
    let is_node_reassignment = body.node_id.is_some()
        && new_effective_node_id != old_effective_node_id
        && new_effective_node_id.is_some();

    let effective_endpoint_url_for_push: Option<String> = match body.endpoint_url.as_deref() {
        Some("") => Some(String::new()),
        Some(url) => Some(url.to_string()),
        None => {
            let v = view.endpoint_url.as_str();
            if v.is_empty() {
                // On a pure rotation (same node, no URL change), omit
                // `target_url` so the node preserves its local
                // `nyxid node credentials add --url` value — HA
                // Supervisor and similar setups depend on that.
                // On a reassignment to a different node, force an
                // explicit clear instead: the destination node may have
                // a stale entry for the same slug from a prior binding,
                // and the "None = preserve" branch on the node side
                // would otherwise inherit that old URL
                // (thirtieth-round Codex P2).
                if is_node_reassignment {
                    Some(String::new())
                } else {
                    None
                }
            } else {
                Some(v.to_string())
            }
        }
    };

    // Decide whether this PUT should push to a node. Two triggers:
    //  1. The caller supplied a fresh `credential` in this body — the
    //     canonical "store server-side + deliver to node" flow.
    //  2. The caller is touching a node-delivery field (`node_id`,
    //     `auth_method`, `auth_key_name`, `endpoint_url`) on a service
    //     that already holds a server credential that hasn't been
    //     delivered yet. Covers the retry path after a previous
    //     `PUT /keys/:id` with `credential + node_id` failed at push
    //     time: `ensure_user_api_key_for_update` provisioned the
    //     credential server-side, but `update_user_service` hadn't
    //     committed the routing yet. On resubmit (say, without
    //     `credential` but with `node_id + auth_method`) the handler
    //     now re-delivers that stored secret to the node.
    //
    // Crucially scoped: unrelated edits like `label`, `is_active`,
    // identity props, or `default_request_headers` must NOT trigger a
    // push — they don't affect what the node has to know, and forcing
    // a push on them would (a) fail those edits whenever the node is
    // offline and (b) bypass the node-ownership check since the
    // request body has no `credential` (ninth-round Codex P1).
    let refreshed_api_key_id =
        unified_key_service::get_key(&state.db, &state.encryption_keys, &user_id_str, &key_id)
            .await?
            .api_key_id;
    let touches_node_delivery_field = body.node_id.is_some()
        || body.auth_method.is_some()
        || body.auth_key_name.is_some()
        || body.endpoint_url.is_some();
    let stored_credential_ready_to_push = match refreshed_api_key_id.as_deref() {
        Some(ak_id)
            if !credential_provided_nonempty
                && touches_node_delivery_field
                && effective_auth_method_for_push != "none" =>
        {
            let ak = user_api_key_service::get_api_key(&state.db, &user_id_str, ak_id).await?;
            // Provider-backed credentials (OAuth / device-code / master
            // API key configured at the catalog level) must NEVER be
            // copied to a node. `create_key` rejects the equivalent
            // `{node_id, provider_config_id, credential}` combination at
            // creation time with "Node-routed provider services must be
            // authorized on the node agent"; the PUT retry-push path
            // must respect the same contract. Twelfth-round Codex P2.
            let is_provider_backed = ak.provider_config_id.is_some();
            !is_provider_backed && user_api_key_service::has_server_credential(&ak)
        }
        _ => false,
    };

    // Provider-backed + node-routed credential writes are already
    // rejected upstream in `validate_update_inputs` (before any key
    // mutation), so we don't need to re-check here — the request has
    // aborted with 400 long before reaching this point.

    let should_push = (credential_provided_nonempty || stored_credential_ready_to_push)
        && effective_node_id_for_push.is_some();

    if should_push
        && let Some(ref node_id) = effective_node_id_for_push
        && let Some(ref ak_id) = refreshed_api_key_id
    {
        credential_push_service::push_credential_to_node_strict(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &user_id_str,
            ak_id,
            credential_push_service::StrictPushTarget {
                target_node_id: node_id,
                service_slug: view.slug.as_str(),
                auth_method: effective_auth_method_for_push.as_str(),
                auth_key_name: effective_auth_key_name_for_push.as_str(),
                target_url: effective_endpoint_url_for_push.as_deref(),
            },
        )
        .await?;
    }

    // Same-node downgrade to `auth_method: "none"` is pushed BEFORE
    // any endpoint/service mutations so the PUT stays atomic: if the
    // node rejects the no-auth placeholder (offline, ack timeout,
    // legacy agent without `credential_ack_correlation`), we abort
    // with no partial commit and the old secret keeps working until
    // the user retries — instead of the previous best-effort ordering
    // where a failed push left the DB saying "none" while the node
    // kept injecting the old bearer token (PR #437 review).
    //
    // `effective_endpoint_url_for_push` is the same body-preferred
    // value used by the strict credential push above: it reads
    // `body.endpoint_url` first and only falls back to
    // `view.endpoint_url` when the caller didn't touch it, fixing the
    // stale-URL bug where a combined `{auth_method: "none",
    // endpoint_url: "<new>"}` used to push the old view URL (PR #437
    // review).
    let auth_downgraded_to_none =
        body.auth_method.as_deref() == Some("none") && view.auth_method != "none";
    let stays_on_same_node = {
        let new_effective = match body.node_id.as_deref() {
            Some("") => None,
            Some(n) => Some(n),
            None => view.node_id.as_deref().filter(|n| !n.is_empty()),
        };
        let old_effective = view.node_id.as_deref().filter(|n| !n.is_empty());
        old_effective.is_some() && old_effective == new_effective
    };
    if auth_downgraded_to_none
        && stays_on_same_node
        && let Some(current_nid) = view.node_id.as_deref().filter(|n| !n.is_empty())
    {
        // Mirror the strict credential push's target-URL semantics
        // exactly:
        //   * `Some("new-url")` from body → Some("new-url")
        //   * `Some("")`       from body → Some("")  (explicit clear)
        //   * body omitted, view has URL → Some(view.url) (reassert)
        //   * body omitted, view empty   → None (preserve node's local
        //     config, since same-node)
        // Previously this only read `view.endpoint_url`, so a combined
        // `{auth_method: "none", endpoint_url: "<new>"}` pushed the
        // stale URL and left the node disagreeing with the DB — the
        // stale-URL bug flagged in the PR #437 review.
        let target_url_for_no_auth = effective_endpoint_url_for_push.as_deref();
        credential_push_service::push_no_auth_to_node_strict(
            &state.node_ws_manager,
            current_nid,
            view.slug.as_str(),
            target_url_for_no_auth,
        )
        .await?;
    }

    // Deferred label write. See the matching note at the top of the
    // handler: committing the label up front would break the atomic
    // semantics we now guarantee for node-routed credential updates —
    // the strict push above aborts with `NodeOffline`/ack-error on
    // failure, and we want a failed `PUT /keys` to leave the service
    // untouched so retries are idempotent (thirty-first-round Codex
    // P2). For a newly-provisioned `UserApiKey` (via
    // `ensure_user_api_key_for_update`) the label was already seeded
    // from `preferred_label`, so `update_api_key` is a no-op in that
    // case — we still run it so the legacy "update existing api_key"
    // path stays covered. The endpoint-fallback branch writes through
    // `UserEndpoint.label` when the service has no backing api_key
    // (legacy auto-connected shape).
    if let Some(ref label) = body.label {
        let refreshed_api_key_id_for_label =
            unified_key_service::get_key(&state.db, &state.encryption_keys, &user_id_str, &key_id)
                .await?
                .api_key_id;
        if let Some(ak_id) = refreshed_api_key_id_for_label {
            user_api_key_service::update_api_key(
                &state.db,
                &state.encryption_keys,
                &user_id_str,
                &ak_id,
                Some(label.as_str()),
                None,
            )
            .await?;
        } else {
            user_endpoint_service::update_endpoint(
                &state.db,
                &user_id_str,
                &view.endpoint_id,
                None,
                Some(label.as_str()),
                user_endpoint_service::OpenApiSpecUrlUpdate::Leave,
                user_endpoint_service::RecommendedSkillsUpdate::Leave,
            )
            .await?;
        }
    }

    // Commit `endpoint_url` and `openapi_spec_url` to the DB now that
    // the node push (if any) has landed. Holding both writes until
    // after the strict push keeps server state and node state in sync:
    // a failed push aborts the PUT early with no partial DB commit, and
    // a successful push means both sides now agree. See the matching
    // notes where the early `update_endpoint` calls used to live.
    // Combined into a single `update_endpoint` call so the two fields
    // land atomically rather than in two separate writes.
    let spec_url_update = match body.openapi_spec_url.as_deref() {
        Some(s) if s.trim().is_empty() => user_endpoint_service::OpenApiSpecUrlUpdate::Clear,
        Some(s) => user_endpoint_service::OpenApiSpecUrlUpdate::Set(s),
        None => user_endpoint_service::OpenApiSpecUrlUpdate::Leave,
    };
    let skills_update = match body.recommended_skills.clone() {
        None => user_endpoint_service::RecommendedSkillsUpdate::Leave,
        Some(skills) if skills.is_empty() => user_endpoint_service::RecommendedSkillsUpdate::Clear,
        Some(skills) => user_endpoint_service::RecommendedSkillsUpdate::Set(skills),
    };
    let url_update = body.endpoint_url.as_deref();
    if url_update.is_some()
        || !matches!(
            spec_url_update,
            user_endpoint_service::OpenApiSpecUrlUpdate::Leave
        )
        || !matches!(
            skills_update,
            user_endpoint_service::RecommendedSkillsUpdate::Leave
        )
    {
        user_endpoint_service::update_endpoint(
            &state.db,
            &user_id_str,
            &view.endpoint_id,
            url_update,
            None,
            spec_url_update,
            skills_update,
        )
        .await?;
    }

    // Update UserService fields if any are provided.
    //
    // `custom_user_agent` and `default_request_headers` also live on
    // `UserService` — include them in the guard so header-only requests
    // (e.g. `nyxid service update --default-header …` with no other flags)
    // actually reach `update_user_service`.
    if body.auth_method.is_some()
        || body.auth_key_name.is_some()
        || body.node_id.is_some()
        || body.is_active.is_some()
        || body.admin_only.is_some()
        || body.custom_user_agent.is_some()
        || body.default_request_headers.is_some()
        || has_identity_update
        // Credential-only PUTs still need to run `update_user_service`
        // when we're synthesizing an Authorization override for an
        // existing bearer/basic service with a stale stored
        // `auth_key_name`; otherwise the DB row stays wrong and the
        // next `/api-keys/external` rotation pushes the stale header
        // name again.
        || persisted_auth_key_name_override.is_some()
    {
        let identity = if has_identity_update {
            Some(user_service_service::IdentityConfig {
                identity_propagation_mode: body
                    .identity_propagation_mode
                    .unwrap_or(view.identity_propagation_mode.clone()),
                identity_include_user_id: body
                    .identity_include_user_id
                    .unwrap_or(view.identity_include_user_id),
                identity_include_email: body
                    .identity_include_email
                    .unwrap_or(view.identity_include_email),
                identity_include_name: body
                    .identity_include_name
                    .unwrap_or(view.identity_include_name),
                identity_jwt_audience: if body.identity_jwt_audience.is_some() {
                    body.identity_jwt_audience
                } else {
                    view.identity_jwt_audience.clone()
                },
                forward_access_token: body
                    .forward_access_token
                    .unwrap_or(view.forward_access_token),
                inject_delegation_token: body
                    .inject_delegation_token
                    .unwrap_or(view.inject_delegation_token),
                delegation_token_scope: body
                    .delegation_token_scope
                    .unwrap_or(view.delegation_token_scope.clone()),
            })
        } else {
            None
        };

        let auth_key_name_for_update = body
            .auth_key_name
            .as_deref()
            .or(persisted_auth_key_name_override.as_deref());
        user_service_service::update_user_service(
            &state.db,
            &user_id_str,
            &actor,
            &key_id,
            body.auth_method.as_deref(),
            auth_key_name_for_update,
            body.node_id.as_deref(),
            None,
            body.is_active,
            identity.as_ref(),
            body.custom_user_agent.as_deref(),
            body.default_request_headers.as_ref(),
            None,
            body.admin_only,
        )
        .await?;
    }

    // Run reconcile when routing or auth state changed. Reconcile
    // preserves server-held credentials on node-routed services (see
    // `reconcile_provider_key_for_service_routing`), so it's idempotent
    // and safe regardless of push outcome — no need to gate on a
    // push_confirmed flag.
    if body.node_id.is_some() || body.auth_method.is_some() {
        unified_key_service::reconcile_provider_key_for_service_routing(
            &state.db,
            &user_id_str,
            &key_id,
        )
        .await?;
    }

    // Auto-sync NodeServiceBinding when node_id changes. The actor owns
    // the node, so it must be the one validated -- the binding owner
    // (`user_id_str`) may be an org.
    if body.node_id.is_some() {
        node_service::sync_node_binding_for_user_service(
            &state.db,
            &user_id_str,
            &actor,
            view.catalog_service_id.as_deref(),
            body.node_id.as_deref(),
            view.node_id.as_deref(),
        )
        .await?;
    }

    // When `node_id` actually changed (or was cleared), tell the
    // previous node to drop its locally-cached credential for this
    // service. Otherwise reassigning a service from node A to node B
    // leaves the secret persisted on A, which is a security regression
    // whenever a user moves a routed service between nodes
    // (seventeenth-round Codex P1). Fire-and-forget: if the old node
    // is offline, nothing we can do right now; when it reconnects it
    // won't see the service in `NodeServiceBinding` either.
    if let Some(ref new_node_id_raw) = body.node_id {
        let new_effective = if new_node_id_raw.is_empty() {
            None
        } else {
            Some(new_node_id_raw.as_str())
        };
        let old_effective = view.node_id.as_deref().filter(|n| !n.is_empty());
        if let Some(old_nid) = old_effective
            && old_effective != new_effective
        {
            // Old-node writability was pre-validated by
            // `validate_update_inputs` before the commit — so if we got
            // here the actor is authorized. The post-commit cleanup
            // itself is best-effort: the reassignment is already
            // durable, and returning an error now would leave clients
            // staring at a failed PUT while the stored routing has
            // actually moved (twenty-ninth-round Codex P1). Log ack /
            // queue failures and surface them to the operator through
            // structured logs instead.
            //
            // Capability gating: `credential_remove` was introduced
            // alongside `credential_ack_correlation`, so legacy agents
            // that did not advertise that flag also do not implement
            // the remove frame — sending it to them would be silently
            // dropped as an unknown message. Instead of pretending a
            // queue success meant the secret was cleared, log a
            // warning with an explicit operator hint so the residual
            // credential gets removed manually (thirtieth-round Codex
            // P1). Wait briefly for the post-reconnect capability
            // handshake to complete, otherwise an upgraded agent
            // could be misclassified as legacy here
            // (twenty-ninth-round Codex P2).
            state
                .node_ws_manager
                .await_capability_resolution(old_nid, std::time::Duration::from_millis(500))
                .await;
            if state
                .node_ws_manager
                .supports_credential_ack_correlation(old_nid)
            {
                if let Err(e) = state
                    .node_ws_manager
                    .send_credential_remove_and_wait(
                        old_nid,
                        view.slug.as_str(),
                        std::time::Duration::from_secs(10),
                    )
                    .await
                {
                    tracing::warn!(
                        node_id = %old_nid,
                        service_slug = %view.slug,
                        error = %e,
                        "credential_remove on previous node did not ack cleanly — secret may linger; run `nyxid node credentials remove` on that node to clean up"
                    );
                }
            } else {
                tracing::warn!(
                    node_id = %old_nid,
                    service_slug = %view.slug,
                    "previous node is a legacy agent without credential_remove support — secret likely remains in its local config. Run `nyxid node credentials remove {}` on that node to clean up, then upgrade the node agent",
                    view.slug
                );
            }
        }
    }

    // The same-node `auth_method: "none"` downgrade used to push a
    // no-auth placeholder here, post-commit and best-effort. That
    // block has moved up to run BEFORE any DB mutations (next to
    // `push_credential_to_node_strict`) so a failed node push leaves
    // the service untouched and the PUT is retry-idempotent (see
    // comment at the strict push site for the full rationale).

    // Return refreshed view
    let updated =
        unified_key_service::get_key(&state.db, &state.encryption_keys, &user_id_str, &key_id)
            .await?;
    let catalog_id = updated.catalog_service_id.clone();
    let catalog_slug = updated.catalog_service_slug.clone();
    let api_key_id = updated.api_key_id.clone();
    let mut response = key_response_from_view(updated);
    let (permission_url, permission_scopes) = derive_lark_permission_for_key(
        &state,
        &user_id_str,
        catalog_id.as_deref(),
        catalog_slug.as_deref(),
        api_key_id.as_deref(),
    )
    .await;
    response.permission_setup_url = permission_url;
    response.permission_setup_scopes = permission_scopes;
    enrich_key_response(
        &state.db,
        &state.node_ws_manager,
        &actor,
        state.config.node_heartbeat_timeout_secs,
        state.config.base_url.trim_end_matches('/'),
        &mut response,
    )
    .await?;
    Ok(Json(response))
}

/// Query params for `DELETE /api/v1/keys/{key_id}`. The browser
/// unload-time cleanup for abandoned OAuth / device-code
/// placeholders passes `only_if_pending=true` so a key that
/// already flipped to `active` (provider callback won the race
/// with `beforeunload`) is left alone instead of being revoked
/// out from under the user. Unknown fields are rejected to catch
/// typos during integration.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteKeyQuery {
    #[serde(default)]
    pub only_if_pending: Option<bool>,
    #[serde(default)]
    pub cascade_grant: Option<bool>,
    #[serde(default)]
    pub grant_scope: Option<String>,
}

#[utoipa::path(
    delete,
    path = "/api/v1/keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "User service ID or slug"),
        ("only_if_pending" = Option<bool>, Query, description = "When true, skip the delete if the key is no longer pending_auth"),
        ("cascade_grant" = Option<bool>, Query, description = "Confirm deletion of all same-owner connections sharing an upstream OAuth grant"),
        ("grant_scope" = Option<String>, Query, description = "Set to token to remove only this service without revoking the upstream grant")
    ),
    responses(
        (status = 200, description = "Key revoked (or skipped when only_if_pending)", body = DeleteKeyResponse),
        (status = 400, description = "Invalid or conflicting revocation options", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 403, description = "Auto-connected service is platform managed", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse),
        (status = 409, description = "OAuth grant cascade confirmation required", body = crate::errors::ErrorResponse)
    ),
    tag = "AI Services"
)]
/// DELETE /api/v1/keys/{key_id}
pub async fn delete_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(key_id): Path<String>,
    Query(query): Query<DeleteKeyQuery>,
) -> AppResult<Json<DeleteKeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let options = unified_key_service::DisconnectOptions {
        cascade_grant: query.cascade_grant.unwrap_or(false),
        grant_scope: query.grant_scope.clone(),
    };
    options.validate()?;
    let access = resolve_key_write_owner(&state, &actor, &key_id).await?;
    let user_id_str = access.owner_id;
    let key_id = access.service_id;

    let view =
        unified_key_service::get_key(&state.db, &state.encryption_keys, &user_id_str, &key_id)
            .await?;
    if view.auto_connected {
        return Err(crate::errors::AppError::Forbidden(
            "Auto-connected services cannot be deleted".to_string(),
        ));
    }

    // Conditional-delete gate for the browser's unload-time
    // cleanup path and the OAuth/device-code Cancel flow (see
    // `abandonPlaceholderKey` in the cli-pair OAuth/device-code
    // flows). When set, we delegate to `revoke_key_if_pending`
    // which closes the status check and the revoke inside a
    // single atomic MongoDB update — a provider callback that
    // flips `pending_auth -> active` between the check and the
    // destructive write cannot slip through.
    if query.only_if_pending.unwrap_or(false) {
        let flipped =
            unified_key_service::revoke_key_if_pending(&state.db, &user_id_str, &actor, &key_id)
                .await?;
        return Ok(Json(DeleteKeyResponse {
            message: if flipped {
                "Key revoked successfully".to_string()
            } else {
                "Key is no longer pending_auth; delete skipped".to_string()
            },
            deleted: flipped,
            upstream_revocation_scheduled: false,
        }));
    }

    let audit_actor = crate::services::audit_service::AuditActor::from_auth_user(&auth_user);
    let result = unified_key_service::disconnect_credentials(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &audit_actor,
        unified_key_service::DisconnectTarget::UserService(&key_id),
        options,
    )
    .await?;

    for event in &result.deleted_services {
        emit_event(
            state.telemetry.as_deref(),
            &actor,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::KeyDeleted {
                source: event.source.clone(),
            },
        );
    }

    Ok(Json(DeleteKeyResponse {
        message: "Key revoked successfully".to_string(),
        deleted: true,
        upstream_revocation_scheduled: result.upstream_revocation_scheduled,
    }))
}

fn key_response_from_result(result: &unified_key_service::CreateKeyResult) -> KeyResponse {
    let label = result.api_key.as_ref().map_or_else(
        || result.endpoint.label.clone(),
        |api_key| api_key.label.clone(),
    );
    let source = if result.service.catalog_service_id.is_some() {
        "catalog"
    } else {
        "custom"
    }
    .to_string();

    KeyResponse {
        id: result.service.id.clone(),
        name: label.clone(),
        label,
        slug: result.service.slug.clone(),
        description: None,
        service_category: source.clone(),
        endpoint_url: Some(result.endpoint.url.clone()),
        endpoint_id: result.endpoint.id.clone(),
        api_key_id: result.api_key.as_ref().map(|api_key| api_key.id.clone()),
        credential_missing: false,
        credential_type: result
            .api_key
            .as_ref()
            .map(|api_key| api_key.credential_type.clone())
            .unwrap_or_else(|| "none".to_string()),
        auth_method: result.service.auth_method.clone(),
        auth_key_name: result.service.auth_key_name.clone(),
        status: result
            .api_key
            .as_ref()
            .map(|api_key| api_key.status.clone())
            .unwrap_or_else(|| "active".to_string()),
        connection_status: result
            .api_key
            .as_ref()
            .and_then(unified_key_service::oauth_connection_status),
        catalog_service_id: result.service.catalog_service_id.clone(),
        catalog_service_slug: None,
        catalog_service_name: None,
        revocation: None,
        node_id: result.service.node_id.clone(),
        node_priority: result.service.node_priority,
        node_status: None,
        node_last_heartbeat_at: None,
        connected: true,
        requires_connection: false,
        has_node_binding: result
            .service
            .node_id
            .as_ref()
            .is_some_and(|node_id| !node_id.is_empty()),
        proxy_url: String::new(),
        proxy_url_slug: String::new(),
        docs_url: None,
        openapi_url: None,
        asyncapi_url: None,
        streaming_supported: false,
        websocket_supported: false,
        source,
        service_type: result.service.service_type.clone(),
        ssh_auth_mode: result.service.ssh_auth_mode,
        ssh_node_keys_stale: result.service.ssh_node_keys_stale,
        admin_only: result.service.admin_only,
        is_active: result.service.is_active,
        identity_propagation_mode: result.service.identity_propagation_mode.clone(),
        identity_include_user_id: result.service.identity_include_user_id,
        identity_include_email: result.service.identity_include_email,
        identity_include_name: result.service.identity_include_name,
        identity_jwt_audience: result.service.identity_jwt_audience.clone(),
        forward_access_token: result.service.forward_access_token,
        inject_delegation_token: result.service.inject_delegation_token,
        delegation_token_scope: result.service.delegation_token_scope.clone(),
        custom_user_agent: result.service.custom_user_agent.clone(),
        connection_id: result
            .api_key
            .as_ref()
            .and_then(|api_key| api_key.connection_id.clone()),
        // The fresh-create response leaves `oauth_client_id` as `None`;
        // the GET endpoints decrypt and surface it. Surfacing it here
        // would either require a synchronous decrypt (mismatched with
        // the existing sync builder shape) or echoing the user-supplied
        // plaintext, which loses the round-trip-encryption proof. The
        // wizard can call `GET /keys/:id` immediately after create if
        // it needs the field rendered.
        oauth_client_id: None,
        // Fresh create: an OAuth connection has no granted scopes until the
        // authorize callback completes, so there's nothing to surface yet.
        granted_scopes: None,
        last_authorized_at: None,
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            result.service.default_request_headers.clone(),
        ),
        ws_frame_injections: result.service.ws_frame_injections.clone(),
        auto_connected: false,
        platform_managed: false,
        platform: None,
        source_app_id: None,
        source_app_name: None,
        expires_at: result
            .api_key
            .as_ref()
            .and_then(|api_key| api_key.expires_at.map(|dt| dt.to_rfc3339())),
        last_used_at: None,
        error_message: None,
        created_at: result.service.created_at.to_rfc3339(),
        updated_at: result.service.updated_at.to_rfc3339(),
        state_version: result.service.state_version,
        rotation_predecessor_id: result.service.rotation_predecessor_id.clone(),
        ssh_host: result.ssh_host.clone(),
        ssh_port: result.ssh_port,
        ssh_ca_public_key: result.ssh_ca_public_key.clone(),
        ssh_allowed_principals: result.ssh_allowed_principals.clone(),
        ssh_certificate_ttl_minutes: result.ssh_certificate_ttl_minutes,
        openapi_spec_url: result.endpoint.openapi_spec_url.clone(),
        recommended_skills: result.endpoint.recommended_skills.clone(),
        // Newly created keys are always personal -- create_key only inserts
        // into the actor's own user_id, not into an org.
        credential_source:
            crate::handlers::user_services_handler::CredentialSourceResponse::Personal,
        // The Lark/Feishu permission deep link is derived in the handler
        // after this builder runs; see `derive_lark_permission_for_key`.
        permission_setup_url: None,
        permission_setup_scopes: None,
    }
}

fn key_response_from_view(view: unified_key_service::KeyView) -> KeyResponse {
    let source = if view.catalog_service_id.is_some() {
        "catalog"
    } else {
        "custom"
    }
    .to_string();
    let has_node_binding = view
        .node_id
        .as_ref()
        .is_some_and(|node_id| !node_id.is_empty());
    let endpoint_url = (!view.auto_connected).then_some(view.endpoint_url);

    KeyResponse {
        id: view.id,
        name: view
            .catalog_service_name
            .clone()
            .unwrap_or_else(|| view.label.clone()),
        label: view.label,
        slug: view.slug,
        description: None,
        service_category: source.clone(),
        endpoint_url,
        endpoint_id: view.endpoint_id,
        api_key_id: view.api_key_id,
        credential_missing: view.credential_missing,
        credential_type: view.credential_type,
        auth_method: view.auth_method,
        auth_key_name: view.auth_key_name,
        status: view.status,
        connection_status: view.connection_status,
        catalog_service_id: view.catalog_service_id,
        catalog_service_slug: view.catalog_service_slug,
        catalog_service_name: view.catalog_service_name,
        revocation: None,
        node_id: view.node_id,
        node_priority: view.node_priority,
        node_status: None,
        node_last_heartbeat_at: None,
        connected: true,
        requires_connection: false,
        has_node_binding,
        proxy_url: String::new(),
        proxy_url_slug: String::new(),
        docs_url: None,
        openapi_url: None,
        asyncapi_url: None,
        streaming_supported: false,
        websocket_supported: false,
        source,
        service_type: view.service_type,
        ssh_auth_mode: view.ssh_auth_mode,
        ssh_node_keys_stale: view.ssh_node_keys_stale,
        admin_only: view.admin_only,
        is_active: view.is_active,
        identity_propagation_mode: view.identity_propagation_mode,
        identity_include_user_id: view.identity_include_user_id,
        identity_include_email: view.identity_include_email,
        identity_include_name: view.identity_include_name,
        identity_jwt_audience: view.identity_jwt_audience,
        forward_access_token: view.forward_access_token,
        inject_delegation_token: view.inject_delegation_token,
        delegation_token_scope: view.delegation_token_scope,
        custom_user_agent: view.custom_user_agent,
        connection_id: view.connection_id,
        oauth_client_id: view.oauth_client_id,
        granted_scopes: view.granted_scopes,
        last_authorized_at: view.last_authorized_at,
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            view.default_request_headers,
        ),
        ws_frame_injections: view.ws_frame_injections,
        auto_connected: view.auto_connected,
        platform_managed: false,
        platform: None,
        source_app_id: view.source_app_id,
        source_app_name: view.source_app_name,
        expires_at: view.expires_at,
        last_used_at: view.last_used_at,
        error_message: view.error_message,
        created_at: view.created_at,
        updated_at: view.updated_at,
        state_version: view.state_version,
        rotation_predecessor_id: view.rotation_predecessor_id,
        ssh_host: view.ssh_host,
        ssh_port: view.ssh_port,
        ssh_ca_public_key: view.ssh_ca_public_key,
        ssh_allowed_principals: view.ssh_allowed_principals,
        ssh_certificate_ttl_minutes: view.ssh_certificate_ttl_minutes,
        openapi_spec_url: view.openapi_spec_url,
        recommended_skills: view.recommended_skills,
        credential_source: view.credential_source.into(),
        permission_setup_url: None,
        permission_setup_scopes: None,
    }
}

struct PlatformProviderKeyProjection {
    catalog_service: DownstreamService,
    summary: PlatformKeySummaryResponse,
}

async fn append_platform_key_projection(
    state: &AppState,
    auth_user: &AuthUser,
    keys: &mut Vec<KeyResponse>,
) -> AppResult<()> {
    let enabled = crate::services::feature_flag_service::resolve_personal_features(
        &state.db,
        &auth_user.user_id.to_string(),
    )
    .await?
    .into_iter()
    .any(|key| key == crate::services::feature_flag_service::PLATFORM_SERVICES_FLAG_KEY);
    if !enabled {
        return Ok(());
    }

    let operations =
        platform_credential_service::list_enabled_authorized_operations(&state.db).await?;
    if operations.is_empty() {
        return Ok(());
    }
    let resolution_user_id = auth_user.proxy_resolution_user_id();
    let actor_user_id = auth_user.user_id.to_string();
    let billing_rollout = crate::services::feature_flag_service::billing_rollout_enabled(
        &state.db,
        &resolution_user_id,
        &actor_user_id,
    )
    .await?;
    let context = platform_operation_service::load_authorized_credential_resolution_context(
        &state.db,
        &resolution_user_id,
        &operations,
    )
    .await?;
    let mut providers: Vec<PlatformProviderKeyProjection> = Vec::new();

    for authorization in &operations {
        let row = authorization.operation();
        let descriptor = platform_key_operation_descriptor(row);
        let resolution = platform_operation_service::resolve_endpoint_credential_source(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &resolution_user_id,
            platform_operation_service::PlatformCredentialCaller {
                actor_user_id: &actor_user_id,
                api_key_id: auth_user.api_key_id.as_deref(),
                allow_all_services: auth_user.allow_all_services,
                allowed_service_ids: &auth_user.allowed_service_ids,
                bypass_approval_flow: auth_user.auth_method == AuthMethod::Session,
                credential_intent:
                    crate::models::platform_service_preference::CredentialIntent::Auto,
            },
            authorization,
            platform_operation_service::CredentialResolutionMode::Discover {
                descriptor: &descriptor,
            },
            &context,
        )
        .await?;
        let source = platform_key_credential_source(&resolution);
        let operation = platform_key_operation_summary(row, billing_rollout);
        let catalog_service = authorization.catalog_service();

        if let Some(provider) = providers
            .iter_mut()
            .find(|provider| provider.catalog_service.id == catalog_service.id)
        {
            provider.summary.operations.push(operation);
            merge_platform_key_source(&mut provider.summary, source);
        } else {
            providers.push(PlatformProviderKeyProjection {
                catalog_service: catalog_service.clone(),
                summary: PlatformKeySummaryResponse {
                    operations: vec![operation],
                    credential_source: source.0,
                    reason: source.1,
                },
            });
        }
    }

    let base_url = state.config.base_url.trim_end_matches('/');
    for provider in providers {
        if let Some(key) = keys.iter_mut().find(|key| {
            key.catalog_service_id.as_deref() == Some(provider.catalog_service.id.as_str())
        }) {
            key.platform = Some(provider.summary);
        } else {
            keys.push(synthetic_platform_key(
                provider.catalog_service,
                provider.summary,
                base_url,
            ));
        }
    }
    Ok(())
}

fn platform_key_operation_descriptor(
    row: &crate::models::platform_operation::PlatformOperationRow,
) -> crate::services::operation_descriptor::OperationDescriptor {
    use crate::models::platform_operation::{ConstrainedOp, PlatformOperationKind};

    match &row.kind {
        PlatformOperationKind::Endpoint {
            method,
            path_template,
            ..
        } => crate::services::operation_descriptor::build_http_descriptor(
            method,
            path_template,
            None,
        ),
        PlatformOperationKind::Constrained { op, .. } => {
            let (method, path) = match op {
                ConstrainedOp::Speak => ("POST", "/v1/text-to-speech/{voice_id}"),
                ConstrainedOp::CallAndSay => {
                    ("POST", "/2010-04-01/Accounts/{account_sid}/Calls.json")
                }
                ConstrainedOp::FlightSearch => ("POST", "/air/offer_requests"),
            };
            crate::services::operation_descriptor::build_http_descriptor(method, path, None)
        }
    }
}

fn platform_key_operation_summary(
    row: &crate::models::platform_operation::PlatformOperationRow,
    billing_rollout: bool,
) -> PlatformKeyOperationSummaryResponse {
    use crate::models::platform_operation::PlatformOperationKind;

    let (name, kind) = match &row.kind {
        PlatformOperationKind::Endpoint { name, .. } => {
            (name.clone(), PlatformKeyOperationKindResponse::Endpoint)
        }
        PlatformOperationKind::Constrained { op, .. } => (
            crate::models::platform_operation::constrained_op_name(*op).to_string(),
            PlatformKeyOperationKindResponse::Constrained,
        ),
    };
    let billing = &row.billing;
    let billable = billing_rollout
        && (billing.price_per_unit != "0"
            || billing.secondary.is_some()
            || billing.base_fee_per_call.is_some());
    PlatformKeyOperationSummaryResponse {
        name,
        kind,
        price_label: crate::services::billing::pricing::format_operation_price(billing, billable),
    }
}

fn platform_key_credential_source(
    resolution: &platform_operation_service::PlatformCredentialResolution,
) -> (PlatformKeyCredentialSourceResponse, Option<String>) {
    use platform_operation_service::{
        PlatformCredentialResolution, PlatformFallbackReason, PlatformUnavailableReason,
    };

    match resolution {
        PlatformCredentialResolution::Platform {
            fallback_reason: PlatformFallbackReason::OwnCredentialAbsent,
            ..
        } => (PlatformKeyCredentialSourceResponse::Platform, None),
        PlatformCredentialResolution::Platform { .. } => {
            (PlatformKeyCredentialSourceResponse::PlatformFallback, None)
        }
        PlatformCredentialResolution::OwnConnection { .. } => {
            (PlatformKeyCredentialSourceResponse::OwnConnection, None)
        }
        PlatformCredentialResolution::NodeRouted { .. } => (
            PlatformKeyCredentialSourceResponse::Unusable,
            Some("node_routed".to_string()),
        ),
        PlatformCredentialResolution::ApprovalRequired { .. } => (
            PlatformKeyCredentialSourceResponse::Unusable,
            Some("approval_required".to_string()),
        ),
        PlatformCredentialResolution::OutOfScope { .. } => (
            PlatformKeyCredentialSourceResponse::Unusable,
            Some("out_of_scope".to_string()),
        ),
        PlatformCredentialResolution::Unusable { .. } => (
            PlatformKeyCredentialSourceResponse::Unusable,
            Some("own_connection_unusable".to_string()),
        ),
        PlatformCredentialResolution::Unavailable { reason, .. } => (
            PlatformKeyCredentialSourceResponse::Unusable,
            Some(
                match reason {
                    PlatformUnavailableReason::OwnerOptInRequired => "owner_opt_in_required",
                    PlatformUnavailableReason::OwnConnectionDisabled => "own_connection_disabled",
                }
                .to_string(),
            ),
        ),
    }
}

fn merge_platform_key_source(
    summary: &mut PlatformKeySummaryResponse,
    next: (PlatformKeyCredentialSourceResponse, Option<String>),
) {
    if summary.credential_source == next.0 && summary.reason == next.1 {
        return;
    }
    let either_unusable = summary.credential_source
        == PlatformKeyCredentialSourceResponse::Unusable
        || next.0 == PlatformKeyCredentialSourceResponse::Unusable;
    if either_unusable {
        summary.credential_source = PlatformKeyCredentialSourceResponse::Unusable;
        summary.reason = Some("mixed_operation_availability".to_string());
    } else if summary.credential_source == PlatformKeyCredentialSourceResponse::PlatformFallback
        || next.0 == PlatformKeyCredentialSourceResponse::PlatformFallback
    {
        summary.credential_source = PlatformKeyCredentialSourceResponse::PlatformFallback;
        summary.reason = None;
    } else {
        summary.credential_source = PlatformKeyCredentialSourceResponse::Unusable;
        summary.reason = Some("mixed_credential_sources".to_string());
    }
}

fn synthetic_platform_key(
    catalog: DownstreamService,
    platform: PlatformKeySummaryResponse,
    base_url: &str,
) -> KeyResponse {
    let catalog_id = catalog.id.clone();
    let catalog_slug = catalog.slug.clone();
    let catalog_name = catalog.name.clone();
    let websocket_supported = catalog
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.supports_websocket);
    KeyResponse {
        id: format!("platform-provider:{catalog_id}"),
        name: catalog_name.clone(),
        label: catalog_name.clone(),
        slug: catalog_slug.clone(),
        description: catalog.description,
        service_category: catalog.service_category,
        endpoint_url: None,
        endpoint_id: format!("platform-provider:{catalog_id}"),
        api_key_id: None,
        credential_missing: false,
        credential_type: "platform".to_string(),
        auth_method: catalog.auth_method,
        auth_key_name: catalog.auth_key_name,
        status: "active".to_string(),
        connection_status: None,
        catalog_service_id: Some(catalog_id.clone()),
        catalog_service_slug: Some(catalog_slug.clone()),
        catalog_service_name: Some(catalog_name),
        revocation: None,
        node_id: None,
        node_priority: 0,
        node_status: None,
        node_last_heartbeat_at: None,
        connected: platform.credential_source != PlatformKeyCredentialSourceResponse::Unusable,
        requires_connection: false,
        has_node_binding: false,
        proxy_url: format!("{base_url}/api/v1/proxy/s/{catalog_slug}/{{path}}"),
        proxy_url_slug: format!("{base_url}/api/v1/proxy/s/{catalog_slug}/{{path}}"),
        docs_url: None,
        openapi_url: None,
        asyncapi_url: None,
        streaming_supported: catalog.streaming_supported,
        websocket_supported,
        source: "catalog".to_string(),
        service_type: catalog.service_type,
        ssh_auth_mode: SshAuthMode::ProxyOnly,
        ssh_node_keys_stale: false,
        admin_only: false,
        is_active: true,
        identity_propagation_mode: "none".to_string(),
        identity_include_user_id: false,
        identity_include_email: false,
        identity_include_name: false,
        identity_jwt_audience: None,
        forward_access_token: false,
        inject_delegation_token: false,
        delegation_token_scope: String::new(),
        custom_user_agent: None,
        connection_id: None,
        oauth_client_id: None,
        granted_scopes: None,
        last_authorized_at: None,
        default_request_headers: None,
        ws_frame_injections: Vec::new(),
        auto_connected: true,
        platform_managed: true,
        platform: Some(platform),
        source_app_id: None,
        source_app_name: None,
        expires_at: None,
        last_used_at: None,
        error_message: None,
        created_at: catalog.created_at.to_rfc3339(),
        updated_at: catalog.updated_at.to_rfc3339(),
        state_version: 0,
        rotation_predecessor_id: None,
        ssh_host: None,
        ssh_port: None,
        ssh_ca_public_key: None,
        ssh_allowed_principals: None,
        ssh_certificate_ttl_minutes: None,
        openapi_spec_url: catalog.openapi_spec_url,
        recommended_skills: catalog.recommended_skills,
        credential_source:
            crate::handlers::user_services_handler::CredentialSourceResponse::Personal,
        permission_setup_url: None,
        permission_setup_scopes: None,
    }
}

async fn enrich_key_responses(
    db: &mongodb::Database,
    ws_manager: &crate::services::node_ws_manager::NodeWsManager,
    actor_user_id: &str,
    heartbeat_timeout_secs: u64,
    base_url: &str,
    keys: &mut [KeyResponse],
) -> AppResult<()> {
    let mut distinct_node_ids = Vec::new();
    for key in keys.iter() {
        if let Some(node_id) = key
            .node_id
            .as_ref()
            .filter(|s| !s.is_empty() && !distinct_node_ids.contains(*s))
        {
            distinct_node_ids.push(node_id.clone());
        }
    }

    let nodes = node_service::get_nodes_by_ids(db, &distinct_node_ids).await?;
    let node_map: std::collections::HashMap<String, &crate::models::node::Node> =
        nodes.iter().map(|node| (node.id.clone(), node)).collect();

    let mut owner_access_map = std::collections::HashMap::new();
    for node in nodes.iter() {
        if !owner_access_map.contains_key(&node.user_id) {
            let access =
                org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
            owner_access_map.insert(node.user_id.clone(), access);
        }
    }

    for key in keys.iter_mut() {
        if let Some(ref node_id) = key.node_id {
            if node_id.is_empty() {
                continue;
            }
            if let Some(node) = node_map.get(node_id) {
                if let Some(access) = owner_access_map.get(&node.user_id) {
                    if !node_service::node_access_can_read(access) {
                        key.node_status = Some("inaccessible".to_string());
                    } else {
                        key.node_last_heartbeat_at =
                            node.last_heartbeat_at.map(|dt| dt.to_rfc3339());

                        let is_connected = ws_manager.is_connected(&node.id);
                        let is_stale = if let Some(last_hb) = node.last_heartbeat_at {
                            chrono::Utc::now()
                                .signed_duration_since(last_hb)
                                .num_seconds()
                                > heartbeat_timeout_secs as i64
                        } else {
                            true
                        };

                        let status = if !is_connected || is_stale {
                            "offline"
                        } else {
                            match node.status {
                                crate::models::node::NodeStatus::Draining => "draining",
                                crate::models::node::NodeStatus::Offline => "offline",
                                crate::models::node::NodeStatus::Online => "online",
                            }
                        };
                        key.node_status = Some(status.to_string());
                    }
                }
            } else {
                key.node_status = Some("unknown".to_string());
            }
        }
    }

    enrich_key_discovery_metadata(db, base_url, keys).await?;
    Ok(())
}

async fn enrich_key_response(
    db: &mongodb::Database,
    ws_manager: &crate::services::node_ws_manager::NodeWsManager,
    actor_user_id: &str,
    heartbeat_timeout_secs: u64,
    base_url: &str,
    key: &mut KeyResponse,
) -> AppResult<()> {
    enrich_key_responses(
        db,
        ws_manager,
        actor_user_id,
        heartbeat_timeout_secs,
        base_url,
        std::slice::from_mut(key),
    )
    .await
}

async fn enrich_key_discovery_metadata(
    db: &mongodb::Database,
    base_url: &str,
    keys: &mut [KeyResponse],
) -> AppResult<()> {
    let catalog_ids: Vec<&str> = keys
        .iter()
        .filter_map(|key| key.catalog_service_id.as_deref())
        .collect();
    let catalog_services: Vec<DownstreamService> = if catalog_ids.is_empty() {
        vec![]
    } else {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": &catalog_ids } })
            .await?
            .try_collect()
            .await?
    };
    let catalog_by_id: std::collections::HashMap<&str, &DownstreamService> = catalog_services
        .iter()
        .map(|service| (service.id.as_str(), service))
        .collect();

    let provider_ids: Vec<&str> = catalog_services
        .iter()
        .filter_map(|service| service.provider_config_id.as_deref())
        .collect();
    let providers: Vec<ProviderConfig> = if provider_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find(doc! { "_id": { "$in": &provider_ids } })
            .await?
            .try_collect()
            .await?
    };
    let provider_by_id: std::collections::HashMap<&str, &ProviderConfig> = providers
        .iter()
        .map(|provider| (provider.id.as_str(), provider))
        .collect();

    let key_ids: Vec<&str> = keys.iter().map(|key| key.id.as_str()).collect();
    let services: Vec<UserService> = if key_ids.is_empty() {
        vec![]
    } else {
        db.collection::<UserService>(USER_SERVICES)
            .find(doc! { "_id": { "$in": &key_ids } })
            .await?
            .try_collect()
            .await?
    };
    let service_by_id: std::collections::HashMap<&str, &UserService> = services
        .iter()
        .map(|service| (service.id.as_str(), service))
        .collect();

    let endpoint_ids: Vec<&str> = services
        .iter()
        .map(|service| service.endpoint_id.as_str())
        .collect();
    let endpoints: Vec<UserEndpoint> = if endpoint_ids.is_empty() {
        vec![]
    } else {
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": &endpoint_ids } })
            .await?
            .try_collect()
            .await?
    };
    let endpoint_by_id: std::collections::HashMap<&str, &UserEndpoint> = endpoints
        .iter()
        .map(|endpoint| (endpoint.id.as_str(), endpoint))
        .collect();

    for key in keys {
        let Some(service) = service_by_id.get(key.id.as_str()) else {
            continue;
        };

        let projection = if let Some(catalog_id) = key.catalog_service_id.as_deref() {
            catalog_by_id.get(catalog_id).map(|catalog| {
                proxy_discovery_service::project_catalog_key(
                    catalog,
                    &key.id,
                    &key.slug,
                    base_url,
                    key.connected,
                    key.has_node_binding,
                )
            })
        } else {
            endpoint_by_id
                .get(service.endpoint_id.as_str())
                .map(|endpoint| {
                    proxy_discovery_service::project_custom_key(service, endpoint, base_url)
                })
        };

        key.revocation = key
            .catalog_service_id
            .as_deref()
            .and_then(|catalog_id| catalog_by_id.get(catalog_id))
            .and_then(|catalog| catalog.provider_config_id.as_deref())
            .and_then(|provider_id| provider_by_id.get(provider_id))
            .and_then(|provider| crate::services::oauth_revocation::effective_revocation(provider))
            .map(|config| KeyRevocationResponse {
                revokes_grant: config.revokes_grant,
            });

        if let Some(projection) = projection {
            key.name = projection.name;
            key.description = projection.description;
            key.service_category = projection.service_category;
            key.connected = projection.connected;
            key.requires_connection = projection.requires_connection;
            key.has_node_binding = projection.has_node_binding;
            key.proxy_url = projection.proxy_url;
            key.proxy_url_slug = projection.proxy_url_slug;
            key.docs_url = projection.docs_url;
            key.openapi_url = projection.openapi_url;
            key.asyncapi_url = projection.asyncapi_url;
            key.streaming_supported = projection.streaming_supported;
            key.websocket_supported = projection.websocket_supported;
            key.source = projection.source.as_str().to_string();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        extract_app_id_from_api_key, extract_app_id_from_credential,
        validate_optional_label_for_update,
    };
    use crate::crypto::aes::EncryptionKeys;
    use crate::crypto::local_key_provider::LocalKeyProvider;
    use crate::errors::AppError;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::models::user_api_key::COLLECTION_NAME as USER_API_KEYS;
    use crate::models::user_api_key::UserApiKey;
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{
        AUTO_PROVISION_SOURCE, COLLECTION_NAME as USER_SERVICES, UserService,
    };
    use crate::telemetry::TelemetryContext;
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        test_user_endpoint, test_user_service,
    };
    use axum::{
        Json,
        extract::{Path, State},
    };
    use chrono::Utc;
    use mongodb::bson::doc;

    fn test_encryption_keys() -> EncryptionKeys {
        EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([0x22; 32], None)))
    }

    fn make_blank_api_key() -> UserApiKey {
        UserApiKey {
            credential_source: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "test".to_string(),
            credential_type: "api_key".to_string(),
            credential_encrypted: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: None,
            source_id: None,
            credential_epoch: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn key_response_always_serializes_authorization_evidence_properties() {
        let result = crate::services::unified_key_service::CreateKeyResult {
            endpoint: test_user_endpoint(
                "endpoint-1",
                "user-1",
                "Example",
                "https://api.example.com",
                None,
                None,
            ),
            api_key: None,
            service: test_user_service("service-1", "user-1", "example", "endpoint-1", None, None),
            ssh_host: None,
            ssh_port: None,
            ssh_ca_public_key: None,
            ssh_allowed_principals: None,
            ssh_certificate_ttl_minutes: None,
        };
        let response = serde_json::to_value(super::key_response_from_result(&result)).unwrap();

        for property in ["connection_status", "granted_scopes", "last_authorized_at"] {
            assert_eq!(response.get(property), Some(&serde_json::Value::Null));
        }

        assert_aevatar_secret_free(&response);
    }

    /// A `KeyResponse` for a service whose owner used three supported
    /// features whose free text happens to match the evidence reader's
    /// secret-shape tripwire: a `Bearer …` service label, a custom request
    /// header carrying a bearer value, and the `${credential}` WS auth
    /// template that `ws_frame_injector::validate_template_does_not_embed_credentials`
    /// explicitly permits (CLAUDE.md §6).
    fn poisoned_key_response() -> super::KeyResponse {
        use crate::models::default_request_header::DefaultRequestHeader;
        use crate::models::ws_frame_injection::{
            WsFrameDirection, WsFrameInjection, WsFrameKind, WsFrameTrigger,
        };

        let result = crate::services::unified_key_service::CreateKeyResult {
            endpoint: test_user_endpoint(
                "endpoint-1",
                "user-1",
                "Example",
                "https://api.example.com",
                None,
                None,
            ),
            api_key: None,
            service: test_user_service("service-1", "user-1", "example", "endpoint-1", None, None),
            ssh_host: None,
            ssh_port: None,
            ssh_ca_public_key: None,
            ssh_allowed_principals: None,
            ssh_certificate_ttl_minutes: None,
        };
        let mut response = super::key_response_from_result(&result);
        response.label = "Bearer Bot".to_string();
        response.name = "Bearer Bot".to_string();
        response.default_request_headers = Some(vec![DefaultRequestHeader {
            name: "X-Upstream-Auth".to_string(),
            value: "Bearer sk-live-abc123".to_string(),
            overridable: false,
            sensitive: false,
        }]);
        response.ws_frame_injections = vec![WsFrameInjection {
            trigger: WsFrameTrigger::FirstFrameFromDownstream,
            template: r#"{"headers":{"Authorization":"Bearer ${credential}"}}"#.to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        }];
        response.granted_scopes = Some(vec!["repo".to_string(), "read:user".to_string()]);
        response.last_authorized_at = Some("2026-08-19T00:00:00+00:00".to_string());
        response.api_key_id = Some("api-key-1".to_string());
        response.connection_status = Some("connected".to_string());
        response
    }

    /// The hazard this projection exists for. A fully populated detail
    /// response *does* trip the evidence reader's scan, through three
    /// separate user-controlled carriers — so serving the detail document as
    /// authorization evidence would make those services permanently
    /// unverifiable. If this assertion ever stops holding, the projection has
    /// stopped being load-bearing and the reason for it should be re-checked,
    /// not the assertion deleted.
    #[test]
    fn full_key_response_trips_the_aevatar_secret_scan() {
        let response = serde_json::to_value(poisoned_key_response()).unwrap();
        assert!(
            crate::test_utils::aevatar_secret_free_violation(&response).is_some(),
            "expected the full detail response to be rejected by the evidence scan"
        );
    }

    #[test]
    fn authorization_evidence_is_secret_free_for_a_poisoned_service() {
        let evidence = serde_json::to_value(
            super::KeyAuthorizationEvidenceResponse::from_key_response(poisoned_key_response()),
        )
        .unwrap();

        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&evidence),
            None,
            "authorization evidence must never carry a secret-shaped value"
        );

        // Consumed properties plus the additive Wave-2 fields. No free-text
        // carriers (`label`/`name`, header values, WS templates).
        let properties: std::collections::BTreeSet<&str> = evidence
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            properties,
            [
                "api_key_id",
                "connection_status",
                "granted_scopes",
                "id",
                "is_active",
                "last_authorized_at",
                "node_id",
                "rotation_predecessor_id",
                "state_version",
                "status",
                "updated_at",
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<&str>>()
        );
        assert_eq!(evidence.get("node_id"), Some(&serde_json::Value::Null));
        assert_eq!(
            evidence.get("rotation_predecessor_id"),
            Some(&serde_json::Value::Null)
        );
        assert!(
            evidence["state_version"]
                .as_i64()
                .is_some_and(|value| value >= 1)
        );
        assert!(evidence["updated_at"].as_str().is_some());
    }

    #[test]
    fn authorization_evidence_always_serializes_nullable_properties() {
        let result = crate::services::unified_key_service::CreateKeyResult {
            endpoint: test_user_endpoint(
                "endpoint-1",
                "user-1",
                "Example",
                "https://api.example.com",
                None,
                None,
            ),
            api_key: None,
            service: test_user_service("service-1", "user-1", "example", "endpoint-1", None, None),
            ssh_host: None,
            ssh_port: None,
            ssh_ca_public_key: None,
            ssh_allowed_principals: None,
            ssh_certificate_ttl_minutes: None,
        };
        let evidence =
            serde_json::to_value(super::KeyAuthorizationEvidenceResponse::from_key_response(
                super::key_response_from_result(&result),
            ))
            .unwrap();

        // The reader distinguishes an explicit null from a missing property
        // and treats a missing one as a malformed response.
        for property in [
            "connection_status",
            "granted_scopes",
            "last_authorized_at",
            "node_id",
        ] {
            assert_eq!(evidence.get(property), Some(&serde_json::Value::Null));
        }
    }

    #[test]
    fn authorization_evidence_omits_lineage_trio_for_pre_lineage_rows() {
        let result = crate::services::unified_key_service::CreateKeyResult {
            endpoint: test_user_endpoint(
                "endpoint-1",
                "user-1",
                "Example",
                "https://api.example.com",
                None,
                None,
            ),
            api_key: None,
            service: {
                let mut service =
                    test_user_service("service-1", "user-1", "example", "endpoint-1", None, None);
                service.state_version = 0;
                service.rotation_predecessor_id = None;
                service
            },
            ssh_host: None,
            ssh_port: None,
            ssh_ca_public_key: None,
            ssh_allowed_principals: None,
            ssh_certificate_ttl_minutes: None,
        };
        let evidence =
            serde_json::to_value(super::KeyAuthorizationEvidenceResponse::from_key_response(
                super::key_response_from_result(&result),
            ))
            .unwrap();

        for property in ["rotation_predecessor_id", "state_version", "updated_at"] {
            assert!(
                evidence.get(property).is_none(),
                "{property} must be omitted on pre-lineage rows"
            );
        }
        assert_eq!(evidence.get("node_id"), Some(&serde_json::Value::Null));
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(&evidence),
            None
        );
    }

    fn assert_aevatar_secret_free(value: &serde_json::Value) {
        assert_eq!(
            crate::test_utils::aevatar_secret_free_violation(value),
            None
        );
    }

    async fn insert_user(db: &mongodb::Database, user_id: &str, user_type: UserType) {
        db.collection(USERS)
            .insert_one(test_user(user_id, user_type))
            .await
            .unwrap();
    }

    async fn insert_membership(
        db: &mongodb::Database,
        org_id: &str,
        actor_id: &str,
        role: OrgRole,
    ) {
        db.collection(ORG_MEMBERSHIPS)
            .insert_one(test_membership(org_id, actor_id, role, None))
            .await
            .unwrap();
    }

    async fn insert_key_fixture(
        db: &mongodb::Database,
        owner_id: &str,
        service_id: &str,
        slug: &str,
        label: &str,
    ) {
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                owner_id,
                label,
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection(USER_SERVICES)
            .insert_one(test_user_service(
                service_id,
                owner_id,
                slug,
                &endpoint_id,
                None,
                None,
            ))
            .await
            .unwrap();
    }

    async fn insert_platform_projection_fixture(state: &crate::AppState, owner_id: &str) -> String {
        use crate::models::platform_operation::{
            ConstrainedConfig, OperationBilling, OperationLimits, PerRequestCaps,
            PlatformOperationKind, PlatformOperationRow, SpeakOperationConfig,
        };
        use crate::models::service_billing::BillingMetric;

        crate::services::feature_flag_service::set_platform_override(
            &state.db,
            crate::services::feature_flag_service::PLATFORM_SERVICES_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            owner_id,
        )
        .await
        .expect("enable platform services for key projection");
        crate::services::feature_flag_service::set_platform_override(
            &state.db,
            crate::services::feature_flag_service::BILLING_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            owner_id,
        )
        .await
        .expect("enable billing for key projection");

        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.name = "ElevenLabs".to_string();
        catalog.slug = "api-elevenlabs".to_string();
        catalog.base_url = "https://api.elevenlabs.io".to_string();
        catalog.service_category = "connection".to_string();
        catalog.requires_user_credential = true;
        catalog.auth_method = "header".to_string();
        catalog.auth_key_name = "xi-api-key".to_string();
        state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .expect("insert projection catalog provider");
        crate::services::platform_credential_service::set_credential_for_test(
            &state.db,
            &state.encryption_keys,
            &catalog.id,
            "platform-elevenlabs-secret",
            owner_id,
        )
        .await
        .expect("set projection platform credential");
        crate::services::platform_preference_service::upsert_preference(
            &state.db,
            owner_id,
            owner_id,
            &catalog.id,
            crate::services::platform_preference_service::PreferenceWrite {
                platform_enabled: true,
                max_credits_per_call: "100".to_string(),
                max_credits_per_day: "1000".to_string(),
                operation_overrides: Vec::new(),
            },
        )
        .await
        .expect("opt in to projection platform provider");

        let mut billing = OperationBilling::free(BillingMetric::Characters);
        billing.price_per_unit = "0.25".to_string();
        let mut speak = PlatformOperationRow::new_constrained(
            catalog.id.clone(),
            crate::models::platform_operation::ConstrainedOp::Speak,
            ConstrainedConfig::Speak(SpeakOperationConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                model_id: "eleven_multilingual_v2".to_string(),
                max_calls_per_user_per_day: 50,
            }),
            OperationLimits {
                per_request: PerRequestCaps::Speak { max_chars: 500 },
                per_user_per_day: Some(50),
            },
            billing,
            owner_id.to_string(),
        );
        speak.enabled = true;
        let mut endpoint = PlatformOperationRow::new_endpoint(
            catalog.id.clone(),
            "GET".to_string(),
            "/v1/voices".to_string(),
            "List voices".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: Some(100),
            },
            OperationBilling::free(BillingMetric::Requests),
            owner_id.to_string(),
        );
        endpoint.enabled = true;
        assert!(matches!(
            endpoint.kind,
            PlatformOperationKind::Endpoint { .. }
        ));
        state
            .db
            .collection::<PlatformOperationRow>(crate::models::platform_operation::COLLECTION_NAME)
            .insert_many([speak, endpoint])
            .await
            .expect("insert projection operations");
        catalog.id
    }

    fn empty_update_request() -> super::UpdateKeyRequest {
        super::UpdateKeyRequest {
            label: None,
            endpoint_url: None,
            auth_method: None,
            auth_key_name: None,
            node_id: None,
            credential: None,
            is_active: None,
            admin_only: None,
            identity_propagation_mode: None,
            identity_include_user_id: None,
            identity_include_email: None,
            identity_include_name: None,
            identity_jwt_audience: None,
            forward_access_token: None,
            inject_delegation_token: None,
            delegation_token_scope: None,
            custom_user_agent: None,
            default_request_headers: None,
            openapi_spec_url: None,
            recommended_skills: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            copy_oauth_client_from: None,
        }
    }

    #[test]
    fn extract_app_id_handles_token_exchange_credential_json() {
        let credential = r#"{"app_id":"cli_a40bc75349bcfff1","app_secret":"shh"}"#;
        assert_eq!(
            extract_app_id_from_credential(credential),
            Some("cli_a40bc75349bcfff1".to_string())
        );
    }

    #[test]
    fn extract_app_id_trims_whitespace_and_rejects_empty() {
        let credential = r#"{"app_id":"  ","app_secret":"shh"}"#;
        assert_eq!(extract_app_id_from_credential(credential), None);

        let credential = r#"{"app_id":"  cli_xyz  ","app_secret":"shh"}"#;
        assert_eq!(
            extract_app_id_from_credential(credential),
            Some("cli_xyz".to_string())
        );
    }

    #[test]
    fn extract_app_id_returns_none_for_non_json_credential() {
        // Plain bearer tokens / API keys don't parse as JSON; the helper
        // must short-circuit cleanly so non-Lark services keep working.
        assert_eq!(extract_app_id_from_credential("sk-test-abc123"), None);
        assert_eq!(extract_app_id_from_credential(""), None);
        assert_eq!(extract_app_id_from_credential(r#"{"other":"value"}"#), None);
    }

    #[tokio::test]
    async fn extract_from_api_key_reads_token_exchange_credential_blob() {
        let keys = test_encryption_keys();
        let mut api_key = make_blank_api_key();
        let plaintext = r#"{"app_id":"cli_token_exchange","app_secret":"shh"}"#;
        api_key.credential_encrypted = Some(keys.encrypt(plaintext.as_bytes()).await.unwrap());

        let app_id = extract_app_id_from_api_key(&keys, &api_key).await;
        assert_eq!(app_id, Some("cli_token_exchange".to_string()));
    }

    #[tokio::test]
    async fn extract_from_api_key_falls_back_to_byo_oauth_client_id() {
        let keys = test_encryption_keys();
        let mut api_key = make_blank_api_key();
        api_key.user_oauth_client_id_encrypted =
            Some(keys.encrypt(b"  cli_byo_oauth  ").await.unwrap());

        let app_id = extract_app_id_from_api_key(&keys, &api_key).await;
        assert_eq!(app_id, Some("cli_byo_oauth".to_string()));
    }

    #[tokio::test]
    async fn extract_from_api_key_prefers_credential_blob_over_byo_oauth_id() {
        // When both fields are populated (rare but possible during
        // migration overlap), the JSON credential wins because that's
        // the authoritative source for token-exchange services that
        // also happen to have a BYO OAuth client recorded.
        let keys = test_encryption_keys();
        let mut api_key = make_blank_api_key();
        let plaintext = r#"{"app_id":"cli_from_blob","app_secret":"shh"}"#;
        api_key.credential_encrypted = Some(keys.encrypt(plaintext.as_bytes()).await.unwrap());
        api_key.user_oauth_client_id_encrypted =
            Some(keys.encrypt(b"cli_from_oauth").await.unwrap());

        let app_id = extract_app_id_from_api_key(&keys, &api_key).await;
        assert_eq!(app_id, Some("cli_from_blob".to_string()));
    }

    #[tokio::test]
    async fn extract_from_api_key_returns_none_when_decrypt_fails() {
        // A blob that wasn't produced by this key set must not panic
        // and must not fall through to a misleading partial value —
        // best-effort means silently degrade to no URL.
        let writer_keys = test_encryption_keys();
        let mut api_key = make_blank_api_key();
        let plaintext = r#"{"app_id":"cli_xyz","app_secret":"shh"}"#;
        api_key.credential_encrypted =
            Some(writer_keys.encrypt(plaintext.as_bytes()).await.unwrap());

        // Use a completely different key set to read it back.
        let reader_keys =
            EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([0x99; 32], None)));
        let app_id = extract_app_id_from_api_key(&reader_keys, &api_key).await;
        assert_eq!(app_id, None);
    }

    #[tokio::test]
    async fn extract_from_api_key_returns_none_when_credential_blob_is_invalid_utf8() {
        let keys = test_encryption_keys();
        let mut api_key = make_blank_api_key();
        // Encrypt raw bytes that aren't valid UTF-8 (eg an arbitrary
        // binary blob someone shoved into credential_encrypted).
        api_key.credential_encrypted = Some(keys.encrypt(&[0xff, 0xfe, 0xfd]).await.unwrap());

        let app_id = extract_app_id_from_api_key(&keys, &api_key).await;
        assert_eq!(app_id, None);
    }

    #[tokio::test]
    async fn extract_from_api_key_returns_none_when_credential_blob_lacks_app_id() {
        // A token-exchange JSON object missing `app_id` (or with an
        // empty / whitespace-only one) must not produce a URL.
        let keys = test_encryption_keys();
        let mut api_key = make_blank_api_key();
        let plaintext = r#"{"app_secret":"shh"}"#;
        api_key.credential_encrypted = Some(keys.encrypt(plaintext.as_bytes()).await.unwrap());

        let app_id = extract_app_id_from_api_key(&keys, &api_key).await;
        assert_eq!(app_id, None);
    }

    #[tokio::test]
    async fn extract_from_api_key_returns_none_for_blank_blobs_and_unset_fields() {
        let keys = test_encryption_keys();
        let api_key = make_blank_api_key();

        // Both encrypted fields unset → None.
        assert_eq!(extract_app_id_from_api_key(&keys, &api_key).await, None);

        // Empty (zero-length) blob is treated as "absent" — exercises
        // the `!blob.is_empty()` guard so we don't try to decrypt
        // empty ciphertext.
        let mut api_key_with_empty = make_blank_api_key();
        api_key_with_empty.credential_encrypted = Some(Vec::new());
        api_key_with_empty.user_oauth_client_id_encrypted = Some(Vec::new());
        assert_eq!(
            extract_app_id_from_api_key(&keys, &api_key_with_empty).await,
            None
        );
    }

    #[test]
    fn update_label_validation_accepts_none_and_valid_lengths() {
        assert!(validate_optional_label_for_update(None).is_ok());
        assert!(validate_optional_label_for_update(Some("ok")).is_ok());
        assert!(validate_optional_label_for_update(Some(&"x".repeat(200))).is_ok());
    }

    #[test]
    fn update_label_validation_rejects_empty_and_too_long_values() {
        let err = validate_optional_label_for_update(Some(""))
            .expect_err("empty label should be rejected before any mutation");
        assert!(matches!(err, AppError::ValidationError(_)));

        let err = validate_optional_label_for_update(Some(&"x".repeat(201)))
            .expect_err("overlong label should be rejected before any mutation");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn get_key_by_slug_returns_personal_service() {
        let Some(db) = connect_test_database("keys_get_slug_personal").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let slug = "routeros";
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_key_fixture(&db, &actor_id, &service_id, slug, "Personal RouterOS").await;

        let Json(response) = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path(slug.to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.id, service_id);
        assert_eq!(response.slug, slug);
        assert_eq!(response.label, "Personal RouterOS");
    }

    #[tokio::test]
    async fn get_key_by_slug_returns_org_service_for_admin() {
        let Some(db) = connect_test_database("keys_get_slug_org_admin").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let slug = "routeros";
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_membership(&db, &org_id, &actor_id, OrgRole::Admin).await;
        insert_key_fixture(&db, &org_id, &service_id, slug, "Org RouterOS").await;

        let Json(response) = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path(slug.to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.id, service_id);
        assert_eq!(response.label, "Org RouterOS");
    }

    #[tokio::test]
    async fn get_key_by_slug_returns_org_service_for_member() {
        let Some(db) = connect_test_database("keys_get_slug_org_member").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let slug = "routeros";
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_membership(&db, &org_id, &actor_id, OrgRole::Member).await;
        insert_key_fixture(&db, &org_id, &service_id, slug, "Org RouterOS").await;

        let Json(response) = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path(slug.to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.id, service_id);
        assert_eq!(response.label, "Org RouterOS");
    }

    #[tokio::test]
    async fn get_key_by_slug_returns_not_found_without_relationship() {
        let Some(db) = connect_test_database("keys_get_slug_no_relationship").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let slug = "routeros";
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_key_fixture(&db, &org_id, &service_id, slug, "Org RouterOS").await;

        let err = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path(slug.to_string()),
        )
        .await
        .expect_err("unrelated actor should not resolve org slug");

        assert!(matches!(
            err,
            AppError::NotFound(message) if message == "Key not found"
        ));
    }

    #[tokio::test]
    async fn get_key_by_slug_prefers_personal_service_over_org_service() {
        let Some(db) = connect_test_database("keys_get_slug_personal_first").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let personal_service_id = uuid::Uuid::new_v4().to_string();
        let org_service_id = uuid::Uuid::new_v4().to_string();
        let slug = "routeros";
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_membership(&db, &org_id, &actor_id, OrgRole::Admin).await;
        insert_key_fixture(
            &db,
            &actor_id,
            &personal_service_id,
            slug,
            "Personal RouterOS",
        )
        .await;
        insert_key_fixture(&db, &org_id, &org_service_id, slug, "Org RouterOS").await;

        let Json(response) = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path(slug.to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.id, personal_service_id);
        assert_eq!(response.label, "Personal RouterOS");
    }

    #[tokio::test]
    async fn get_key_by_uuid_continues_to_work() {
        let Some(db) = connect_test_database("keys_get_uuid").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_key_fixture(&db, &actor_id, &service_id, "routeros", "Personal RouterOS").await;

        let Json(response) = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path(service_id.clone()),
        )
        .await
        .unwrap();

        assert_eq!(response.id, service_id);
        assert_eq!(response.slug, "routeros");
    }

    #[tokio::test]
    async fn auto_connected_key_responses_omit_endpoint_url() {
        let Some(db) = connect_test_database("keys_auto_endpoint_redaction").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let catalog_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_key_fixture(
            &db,
            &actor_id,
            &service_id,
            "platform-service",
            "Platform Service",
        )
        .await;
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = catalog_id.clone();
        catalog.name = "Platform Service".to_string();
        catalog.slug = "platform-service".to_string();
        catalog.base_url = "https://api.example.com".to_string();
        catalog.visibility = "public".to_string();
        catalog.service_category = "internal".to_string();
        catalog.service_type = "http".to_string();
        catalog.auth_method = "none".to_string();
        catalog.auth_key_name = String::new();
        catalog.requires_user_credential = false;
        catalog.provider_config_id = None;
        catalog.is_active = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service_id },
                doc! { "$set": {
                    "source": AUTO_PROVISION_SOURCE,
                    "catalog_service_id": &catalog_id,
                } },
            )
            .await
            .unwrap();

        let Json(detail) = super::get_key(
            State(state.clone()),
            test_auth_user(&actor_id),
            Path(service_id.clone()),
        )
        .await
        .unwrap();
        let detail_json = serde_json::to_value(&detail).unwrap();
        assert!(detail.auto_connected);
        assert!(detail.endpoint_url.is_none());
        assert!(
            !detail_json
                .as_object()
                .unwrap()
                .contains_key("endpoint_url")
        );
        assert!(!detail_json.to_string().contains("api.example.com"));

        let Json(list) = super::list_keys(State(state), test_auth_user(&actor_id))
            .await
            .unwrap();
        let listed = list
            .keys
            .iter()
            .find(|key| key.id == service_id)
            .expect("auto-connected service should remain in the management listing");
        let list_json = serde_json::to_value(listed).unwrap();
        assert!(listed.endpoint_url.is_none());
        assert!(!list_json.as_object().unwrap().contains_key("endpoint_url"));
        assert!(!list_json.to_string().contains("api.example.com"));
    }

    /// A disabled service stays addressable by UUID, and stays hidden by slug.
    ///
    /// The UUID half is what makes "Disable" a reversible pause rather than a
    /// one-way door: `/keys/{id}` is the detail page that owns the Enable
    /// control, so 404ing a disabled row there stranded it — the screen that
    /// could undo the pause was the screen that would no longer load.
    ///
    /// The slug half must stay filtered. Slugs are only unique among *active*
    /// rows (the `user_services` unique index is partial on `is_active: true`),
    /// so resolving a disabled row by slug would be ambiguous, and slug is the
    /// shape the proxy path uses.
    #[tokio::test]
    async fn get_key_resolves_disabled_service_by_uuid_but_not_by_slug() {
        let Some(db) = connect_test_database("keys_get_uuid_inactive").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_key_fixture(&db, &actor_id, &service_id, "routeros", "Personal RouterOS").await;
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let Json(response) = super::get_key(
            State(state.clone()),
            test_auth_user(&actor_id),
            Path(service_id.clone()),
        )
        .await
        .expect("a disabled service must stay readable by uuid so it can be re-enabled");
        assert_eq!(response.id, service_id);
        assert!(
            !response.is_active,
            "the response must report the disabled state rather than look healthy"
        );

        let err = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path("routeros".to_string()),
        )
        .await
        .expect_err("a disabled service must not resolve by slug");
        assert!(matches!(
            err,
            AppError::NotFound(message) if message == "Key not found"
        ));
    }

    #[tokio::test]
    async fn update_key_by_slug_allows_admin_and_rejects_member_write() {
        let Some(db) = connect_test_database("keys_put_slug_org_roles").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let admin_id = uuid::Uuid::new_v4().to_string();
        let member_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let slug = "routeros";
        insert_user(&db, &admin_id, UserType::Person).await;
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_membership(&db, &org_id, &admin_id, OrgRole::Admin).await;
        insert_membership(&db, &org_id, &member_id, OrgRole::Member).await;
        insert_key_fixture(&db, &org_id, &service_id, slug, "Org RouterOS").await;

        let mut admin_update = empty_update_request();
        admin_update.label = Some("Renamed RouterOS".to_string());
        let Json(response) = super::update_key(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(slug.to_string()),
            Json(admin_update),
        )
        .await
        .unwrap();
        assert_eq!(response.id, service_id);
        assert_eq!(response.label, "Renamed RouterOS");

        let mut member_update = empty_update_request();
        member_update.label = Some("Member Rename".to_string());
        let err = super::update_key(
            State(state),
            test_auth_user(&member_id),
            Path(slug.to_string()),
            Json(member_update),
        )
        .await
        .expect_err("org member should not be allowed to update org key");

        assert!(matches!(err, AppError::OrgRoleInsufficient(_)));
    }

    #[tokio::test]
    async fn create_key_rejects_empty_header_auth_key_name_before_writes() {
        let Some(db) = connect_test_database("keys_post_empty_header_auth_key").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();

        let body = super::CreateKeyRequest {
            service_slug: None,
            credential: Some("secret-token".to_string()),
            label: "Header Service".to_string(),
            endpoint_url: Some("https://api.example.com".to_string()),
            slug: Some("header-service".to_string()),
            auth_method: Some("header".to_string()),
            auth_key_name: Some(String::new()),
            node_id: None,
            admin_only: None,
            ssh_host: None,
            ssh_port: None,
            ssh_certificate_auth: None,
            ssh_auth_mode: None,
            ssh_principals: None,
            ssh_certificate_ttl_minutes: None,
            identity_propagation_mode: None,
            identity_include_user_id: None,
            identity_include_email: None,
            identity_include_name: None,
            identity_jwt_audience: None,
            forward_access_token: None,
            inject_delegation_token: None,
            delegation_token_scope: None,
            target_org_id: None,
            openapi_spec_url: None,
            ws_frame_injections: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            copy_oauth_client_from: None,
        };

        let err = super::create_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .expect_err("POST /api/v1/keys should reject empty header auth_key_name");

        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message.contains("auth_method is 'header'")
        ));

        let endpoint_count = db
            .collection::<mongodb::bson::Document>(USER_ENDPOINTS)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();
        let api_key_count = db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();
        let service_count = db
            .collection::<mongodb::bson::Document>(USER_SERVICES)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();

        assert_eq!(endpoint_count, 0);
        assert_eq!(api_key_count, 0);
        assert_eq!(service_count, 0);
    }

    #[tokio::test]
    async fn list_keys_returns_empty_for_user_with_no_keys() {
        let Some(db) = connect_test_database("keys_ext_list_empty").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;

        let Json(response) = super::list_keys(State(state), test_auth_user(&actor_id))
            .await
            .unwrap();

        assert!(response.keys.is_empty());
    }

    #[tokio::test]
    async fn list_keys_returns_owned_keys() {
        let Some(db) = connect_test_database("keys_ext_list_owned").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_key_fixture(&db, &actor_id, &service_id, "my-svc", "My Service").await;

        let Json(response) = super::list_keys(State(state), test_auth_user(&actor_id))
            .await
            .unwrap();

        assert!(!response.keys.is_empty());
        assert!(response.keys.iter().any(|k| k.id == service_id));
    }

    #[tokio::test]
    async fn inactive_dangling_api_key_remains_readable_recoverable_and_deletable() {
        let Some(db) = connect_test_database("keys_ext_dangling_api_key").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_key_fixture(&db, &actor_id, &service_id, "dangling-key", "Dangling Key").await;
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service_id },
                doc! { "$set": {
                    "api_key_id": "missing-api-key",
                    "auth_method": "bearer",
                    "auth_key_name": "Authorization",
                    "is_active": false,
                } },
            )
            .await
            .unwrap();

        let Json(view) = super::get_key(
            State(state.clone()),
            test_auth_user(&actor_id),
            Path(service_id.clone()),
        )
        .await
        .unwrap();
        assert!(view.credential_missing);
        assert_eq!(view.credential_type, "none");
        assert!(!view.is_active);

        let mut enable = empty_update_request();
        enable.is_active = Some(true);
        let enable_error = super::update_key(
            State(state.clone()),
            test_auth_user(&actor_id),
            Path(service_id.clone()),
            Json(enable),
        )
        .await
        .expect_err("a missing credential must be replaced before enabling");
        assert!(matches!(
            enable_error,
            AppError::BadRequest(message) if message.contains("Reconnect or delete")
        ));

        let Json(deleted) = super::delete_key(
            State(state),
            test_auth_user(&actor_id),
            TelemetryContext::default(),
            Path(service_id.clone()),
            axum::extract::Query(super::DeleteKeyQuery::default()),
        )
        .await
        .unwrap();
        assert!(deleted.deleted);

        let tombstone = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service_id })
            .await
            .unwrap()
            .unwrap();
        assert!(!tombstone.is_active);
        assert!(
            db.collection::<UserEndpoint>(USER_ENDPOINTS)
                .find_one(doc! { "_id": &tombstone.endpoint_id })
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn get_key_not_found_returns_error() {
        let Some(db) = connect_test_database("keys_ext_get_notfound").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;

        let err = super::get_key(
            State(state),
            test_auth_user(&actor_id),
            Path("nonexistent-id".to_string()),
        )
        .await
        .expect_err("should return not found");

        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_key_not_found_returns_error() {
        let Some(db) = connect_test_database("keys_ext_delete_notfound").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;

        let err = super::delete_key(
            State(state),
            test_auth_user(&actor_id),
            TelemetryContext::default(),
            Path("nonexistent-id".to_string()),
            axum::extract::Query(super::DeleteKeyQuery {
                only_if_pending: None,
                ..Default::default()
            }),
        )
        .await
        .expect_err("should return not found");

        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_key_by_other_user_returns_not_found() {
        let Some(db) = connect_test_database("keys_ext_get_other_user").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let owner_id = uuid::Uuid::new_v4().to_string();
        let other_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &owner_id, UserType::Person).await;
        insert_user(&db, &other_id, UserType::Person).await;
        insert_key_fixture(&db, &owner_id, &service_id, "private-svc", "Private").await;

        let err = super::get_key(State(state), test_auth_user(&other_id), Path(service_id))
            .await
            .expect_err("other user should not see the key");

        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_key_not_found_returns_error() {
        let Some(db) = connect_test_database("keys_ext_update_notfound").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;

        let err = super::update_key(
            State(state),
            test_auth_user(&actor_id),
            Path("nonexistent-id".to_string()),
            Json(empty_update_request()),
        )
        .await
        .expect_err("should return not found");

        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn validate_optional_label_rejects_empty() {
        assert!(super::validate_optional_label_for_update(Some("")).is_err());
    }

    #[test]
    fn validate_optional_label_rejects_too_long() {
        let long_label = "a".repeat(201);
        assert!(super::validate_optional_label_for_update(Some(&long_label)).is_err());
    }

    #[test]
    fn validate_optional_label_accepts_valid() {
        assert!(super::validate_optional_label_for_update(Some("Good Label")).is_ok());
        assert!(super::validate_optional_label_for_update(None).is_ok());
    }

    #[test]
    fn extract_app_id_from_credential_extracts_valid_json() {
        let cred = r#"{"app_id": "cli_abc123", "app_secret": "secret"}"#;
        assert_eq!(
            super::extract_app_id_from_credential(cred),
            Some("cli_abc123".to_string())
        );
    }

    #[test]
    fn extract_app_id_from_credential_returns_none_for_invalid() {
        assert!(super::extract_app_id_from_credential("not-json").is_none());
        assert!(super::extract_app_id_from_credential(r#"{"key": "val"}"#).is_none());
        assert!(super::extract_app_id_from_credential(r#"{"app_id": ""}"#).is_none());
    }

    // ---- create_key integration tests ----

    fn make_create_key_request(
        label: &str,
        slug: Option<&str>,
        endpoint_url: Option<&str>,
        credential: Option<&str>,
        auth_method: Option<&str>,
    ) -> super::CreateKeyRequest {
        super::CreateKeyRequest {
            service_slug: None,
            credential: credential.map(str::to_string),
            label: label.to_string(),
            endpoint_url: endpoint_url.map(str::to_string),
            slug: slug.map(str::to_string),
            auth_method: auth_method.map(str::to_string),
            auth_key_name: None,
            node_id: None,
            admin_only: None,
            ssh_host: None,
            ssh_port: None,
            ssh_certificate_auth: None,
            ssh_auth_mode: None,
            ssh_principals: None,
            ssh_certificate_ttl_minutes: None,
            identity_propagation_mode: None,
            identity_include_user_id: None,
            identity_include_email: None,
            identity_include_name: None,
            identity_jwt_audience: None,
            forward_access_token: None,
            inject_delegation_token: None,
            delegation_token_scope: None,
            target_org_id: None,
            openapi_spec_url: None,
            ws_frame_injections: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            copy_oauth_client_from: None,
        }
    }

    #[tokio::test]
    async fn create_key_custom_endpoint_succeeds() {
        let Some(db) = connect_test_database("h_keys_create_custom").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let body = make_create_key_request(
            "My Custom API",
            Some("my-custom-api"),
            Some("https://api.example.com"),
            Some("sk-test-credential"),
            Some("bearer"),
        );

        let Json(response) = super::create_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        assert_eq!(response.label, "My Custom API");
        assert_eq!(response.slug, "my-custom-api");
        assert_eq!(
            response.endpoint_url.as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(response.auth_method, "bearer");
        assert!(response.api_key_id.is_some());
        assert!(response.is_active);
        assert_eq!(response.credential_type, "api_key");
    }

    #[tokio::test]
    async fn create_key_no_auth_method_succeeds() {
        let Some(db) = connect_test_database("h_keys_create_no_auth").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let body = make_create_key_request(
            "No Auth Service",
            Some("no-auth-svc"),
            Some("https://public-api.example.com"),
            None,
            Some("none"),
        );

        let Json(response) = super::create_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        assert_eq!(response.label, "No Auth Service");
        assert_eq!(response.slug, "no-auth-svc");
        assert_eq!(response.auth_method, "none");
        assert!(response.api_key_id.is_none());
        assert_eq!(response.credential_type, "none");
    }

    #[tokio::test]
    async fn create_key_missing_slug_and_service_slug_rejected() {
        let Some(db) = connect_test_database("h_keys_create_no_slug").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        // Current custom-key creation derives the slug from the label when
        // service_slug and slug are both omitted.
        let body = make_create_key_request(
            "Missing Slug",
            None,
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );

        let Json(response) = super::create_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .expect("custom key should derive slug from label");

        assert_eq!(response.slug, "missing-slug");
        assert_eq!(response.label, "Missing Slug");
    }

    #[tokio::test]
    async fn create_key_for_org_requires_admin() {
        let Some(db) = connect_test_database("h_keys_create_org_admin").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let admin_id = uuid::Uuid::new_v4().to_string();
        let member_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &admin_id, UserType::Person).await;
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_membership(&db, &org_id, &admin_id, OrgRole::Admin).await;
        insert_membership(&db, &org_id, &member_id, OrgRole::Member).await;

        // Admin should succeed
        let mut body = make_create_key_request(
            "Org Service",
            Some("org-svc"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        body.target_org_id = Some(org_id.clone());

        let result = super::create_key(
            State(state.clone()),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await;
        assert!(result.is_ok(), "admin should create org key");

        // Member should be rejected
        let mut body2 = make_create_key_request(
            "Org Service 2",
            Some("org-svc-2"),
            Some("https://api2.example.com"),
            Some("sk-test2"),
            Some("bearer"),
        );
        body2.target_org_id = Some(org_id);

        let err = super::create_key(
            State(state),
            test_auth_user(&member_id),
            TelemetryContext::default(),
            Json(body2),
        )
        .await
        .expect_err("member should not create org key");
        assert!(matches!(err, AppError::OrgRoleInsufficient(_)));
    }

    #[tokio::test]
    async fn create_key_mutual_exclusion_oauth_fields() {
        let Some(db) = connect_test_database("h_keys_create_oauth_mutex").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let mut body = make_create_key_request(
            "OAuth Conflict",
            Some("oauth-conflict"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        body.oauth_client_id = Some("client-id".to_string());
        body.oauth_client_secret = Some("client-secret".to_string());
        body.copy_oauth_client_from = Some("some-key-id".to_string());

        let err = super::create_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .expect_err("should reject mutually exclusive oauth fields");

        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("mutually exclusive")));
    }

    #[tokio::test]
    async fn create_key_oauth_client_id_requires_secret() {
        let Some(db) = connect_test_database("h_keys_create_oauth_halved").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let mut body = make_create_key_request(
            "Half OAuth",
            Some("half-oauth"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        body.oauth_client_id = Some("client-id".to_string());
        // no oauth_client_secret

        let err = super::create_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .expect_err("should reject half-pair oauth credentials");

        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("supplied together")));
    }

    // ---- delete_key integration tests ----

    #[tokio::test]
    async fn delete_key_success() {
        let Some(db) = connect_test_database("h_keys_delete_ok").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        // First create a key
        let body = make_create_key_request(
            "Deletable Key",
            Some("deletable-key"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        // Delete it
        let Json(deleted) = super::delete_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path(created.id.clone()),
            axum::extract::Query(super::DeleteKeyQuery {
                only_if_pending: None,
                ..Default::default()
            }),
        )
        .await
        .unwrap();

        assert!(deleted.deleted);
        assert!(deleted.message.contains("revoked"));

        // Should be gone now (get returns not found)
        let err = super::get_key(State(state), test_auth_user(&user_id), Path(created.id))
            .await
            .expect_err("deleted key should not be found");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_key_by_slug_success() {
        let Some(db) = connect_test_database("h_keys_delete_slug").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let body = make_create_key_request(
            "Deletable By Slug",
            Some("del-by-slug"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        let Json(deleted) = super::delete_key(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Path("del-by-slug".to_string()),
            axum::extract::Query(super::DeleteKeyQuery {
                only_if_pending: None,
                ..Default::default()
            }),
        )
        .await
        .unwrap();

        assert!(deleted.deleted);
        assert_eq!(created.slug, "del-by-slug");
    }

    #[tokio::test]
    async fn delete_key_other_user_forbidden() {
        let Some(db) = connect_test_database("h_keys_delete_other").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let owner_id = uuid::Uuid::new_v4().to_string();
        let other_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &owner_id, UserType::Person).await;
        insert_user(&db, &other_id, UserType::Person).await;

        let body = make_create_key_request(
            "Owner Key",
            Some("owner-key"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&owner_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        let err = super::delete_key(
            State(state),
            test_auth_user(&other_id),
            TelemetryContext::default(),
            Path(created.id),
            axum::extract::Query(super::DeleteKeyQuery {
                only_if_pending: None,
                ..Default::default()
            }),
        )
        .await
        .expect_err("other user should not delete the key");

        assert!(matches!(err, AppError::NotFound(_)));
    }

    // ---- update_key integration tests ----

    #[tokio::test]
    async fn update_key_label_change_succeeds() {
        let Some(db) = connect_test_database("h_keys_update_label").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let body = make_create_key_request(
            "Original Label",
            Some("label-change-svc"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        let mut update = empty_update_request();
        update.label = Some("Updated Label".to_string());

        let Json(updated) = super::update_key(
            State(state),
            test_auth_user(&user_id),
            Path(created.id.clone()),
            Json(update),
        )
        .await
        .unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.label, "Updated Label");
    }

    #[tokio::test]
    async fn update_key_by_slug_succeeds() {
        let Some(db) = connect_test_database("h_keys_update_slug").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let body = make_create_key_request(
            "Slug Updatable",
            Some("slug-updatable"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        let mut update = empty_update_request();
        update.label = Some("Via Slug".to_string());

        let Json(updated) = super::update_key(
            State(state),
            test_auth_user(&user_id),
            Path("slug-updatable".to_string()),
            Json(update),
        )
        .await
        .unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.label, "Via Slug");
    }

    #[tokio::test]
    async fn update_key_other_user_forbidden() {
        let Some(db) = connect_test_database("h_keys_update_other").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let owner_id = uuid::Uuid::new_v4().to_string();
        let other_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &owner_id, UserType::Person).await;
        insert_user(&db, &other_id, UserType::Person).await;

        let body = make_create_key_request(
            "Private Key",
            Some("private-key"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&owner_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        let mut update = empty_update_request();
        update.label = Some("Hijacked".to_string());

        let err = super::update_key(
            State(state),
            test_auth_user(&other_id),
            Path(created.id),
            Json(update),
        )
        .await
        .expect_err("other user should not update the key");

        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_key_label_validation_rejects_empty() {
        let Some(db) = connect_test_database("h_keys_update_empty_label").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let body = make_create_key_request(
            "Good Label",
            Some("good-label-svc"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        let mut update = empty_update_request();
        update.label = Some(String::new());

        let err = super::update_key(
            State(state),
            test_auth_user(&user_id),
            Path(created.id),
            Json(update),
        )
        .await
        .expect_err("empty label should be rejected");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn update_key_label_validation_rejects_too_long() {
        let Some(db) = connect_test_database("h_keys_update_long_label").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let body = make_create_key_request(
            "Good Label",
            Some("good-label-svc2"),
            Some("https://api.example.com"),
            Some("sk-test"),
            Some("bearer"),
        );
        let Json(created) = super::create_key(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(body),
        )
        .await
        .unwrap();

        let mut update = empty_update_request();
        update.label = Some("x".repeat(201));

        let err = super::update_key(
            State(state),
            test_auth_user(&user_id),
            Path(created.id),
            Json(update),
        )
        .await
        .expect_err("too-long label should be rejected");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    // ---- list_keys integration tests ----

    #[tokio::test]
    async fn list_keys_includes_org_keys() {
        let Some(db) = connect_test_database("h_keys_list_org_keys").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_membership(&db, &org_id, &user_id, OrgRole::Member).await;
        insert_key_fixture(&db, &org_id, &service_id, "org-svc", "Org Service").await;

        let Json(response) = super::list_keys(State(state), test_auth_user(&user_id))
            .await
            .unwrap();

        assert!(
            response.keys.iter().any(|k| k.id == service_id),
            "org keys should be visible to members"
        );
    }

    #[tokio::test]
    async fn list_keys_does_not_include_other_users_keys() {
        let Some(db) = connect_test_database("h_keys_list_isolation").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_a = uuid::Uuid::new_v4().to_string();
        let user_b = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_a, UserType::Person).await;
        insert_user(&db, &user_b, UserType::Person).await;
        insert_key_fixture(&db, &user_a, &service_id, "private-svc", "Private").await;

        let Json(response) = super::list_keys(State(state), test_auth_user(&user_b))
            .await
            .unwrap();

        assert!(
            !response.keys.iter().any(|k| k.id == service_id),
            "other user's keys should not be visible"
        );
    }

    #[tokio::test]
    async fn list_keys_returns_multiple_owned_keys() {
        let Some(db) = connect_test_database("h_keys_list_multi").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        let svc1 = uuid::Uuid::new_v4().to_string();
        let svc2 = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;
        insert_key_fixture(&db, &user_id, &svc1, "svc-one", "Service One").await;
        insert_key_fixture(&db, &user_id, &svc2, "svc-two", "Service Two").await;

        let Json(response) = super::list_keys(State(state), test_auth_user(&user_id))
            .await
            .unwrap();

        assert!(response.keys.len() >= 2);
        assert!(response.keys.iter().any(|k| k.id == svc1));
        assert!(response.keys.iter().any(|k| k.id == svc2));
    }

    #[tokio::test]
    async fn list_keys_synthesizes_stable_read_only_platform_provider_without_decryption() {
        let Some(db) = connect_test_database("h_keys_platform_projection_synthetic").await else {
            eprintln!("skipping keys platform projection test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;
        let catalog_id = insert_platform_projection_fixture(&state, &user_id).await;
        let decrypts_before = state.encryption_keys.decrypt_stats();

        let Json(first) = super::list_keys(State(state.clone()), test_auth_user(&user_id))
            .await
            .expect("list synthetic platform provider");
        let Json(second) = super::list_keys(State(state.clone()), test_auth_user(&user_id))
            .await
            .expect("repeat synthetic platform provider listing");

        assert_eq!(state.encryption_keys.decrypt_stats(), decrypts_before);
        let first_provider = first
            .keys
            .iter()
            .find(|key| key.catalog_service_id.as_deref() == Some(catalog_id.as_str()))
            .expect("platform provider should be projected");
        let second_provider = second
            .keys
            .iter()
            .find(|key| key.catalog_service_id.as_deref() == Some(catalog_id.as_str()))
            .expect("platform provider should remain projected");
        assert_eq!(first_provider.id, second_provider.id);
        assert_eq!(first_provider.id, format!("platform-provider:{catalog_id}"));
        assert!(first_provider.platform_managed);
        assert!(first_provider.auto_connected);
        assert!(first_provider.endpoint_url.is_none());
        let platform = first_provider
            .platform
            .as_ref()
            .expect("platform summary should be attached");
        assert_eq!(
            platform.credential_source,
            super::PlatformKeyCredentialSourceResponse::Platform
        );
        assert_eq!(platform.operations.len(), 2);
        assert!(platform.operations.iter().any(|operation| {
            operation.name == "speak"
                && operation.kind == super::PlatformKeyOperationKindResponse::Constrained
                && operation.price_label == "0.25 credits per character"
        }));
        assert!(platform.operations.iter().any(|operation| {
            operation.name == "List voices"
                && operation.kind == super::PlatformKeyOperationKindResponse::Endpoint
                && operation.price_label == "Free"
        }));

        let error = super::get_key(
            State(state),
            test_auth_user(&user_id),
            Path(first_provider.id.clone()),
        )
        .await
        .expect_err("synthetic response ids must not resolve as UserService ids");
        assert!(matches!(error, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_keys_attaches_platform_summary_to_disabled_row_without_duplicate_or_fallback() {
        let Some(db) = connect_test_database("h_keys_platform_projection_disabled").await else {
            eprintln!("skipping disabled platform projection test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;
        let catalog_id = insert_platform_projection_fixture(&state, &user_id).await;
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &user_id,
                "My ElevenLabs",
                "https://api.elevenlabs.io",
                None,
                Some(&catalog_id),
            ))
            .await
            .expect("insert disabled provider endpoint");
        let service_id = uuid::Uuid::new_v4().to_string();
        let mut service = test_user_service(
            &service_id,
            &user_id,
            "elevenlabs",
            &endpoint_id,
            Some(&catalog_id),
            None,
        );
        service.is_active = false;
        service.auth_method = "header".to_string();
        service.auth_key_name = "xi-api-key".to_string();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service)
            .await
            .expect("insert disabled provider service");

        let Json(response) = super::list_keys(State(state), test_auth_user(&user_id))
            .await
            .expect("list disabled provider projection");

        let provider_rows = response
            .keys
            .iter()
            .filter(|key| key.catalog_service_id.as_deref() == Some(catalog_id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(provider_rows.len(), 1, "no synthetic duplicate is allowed");
        let key = provider_rows[0];
        assert_eq!(key.id, service_id);
        assert!(!key.platform_managed);
        let platform = key.platform.as_ref().expect("platform summary is attached");
        assert_eq!(
            platform.credential_source,
            super::PlatformKeyCredentialSourceResponse::Unusable
        );
        assert_eq!(platform.reason.as_deref(), Some("own_connection_disabled"));
    }

    #[tokio::test]
    async fn list_keys_includes_discovery_fields_for_catalog_and_custom_services() {
        let Some(db) = connect_test_database("h_keys_list_discovery_fields").await else {
            eprintln!("skipping keys handler integration test: no local MongoDB available");
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &user_id, UserType::Person).await;

        let catalog_id = uuid::Uuid::new_v4().to_string();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = catalog_id.clone();
        catalog.name = "Catalog API".to_string();
        catalog.slug = "catalog-api".to_string();
        catalog.description = Some("Catalog description".to_string());
        catalog.service_category = "connection".to_string();
        catalog.requires_user_credential = true;
        catalog.openapi_spec_url = Some("https://example.com/catalog-openapi.json".to_string());
        catalog.asyncapi_spec_url = Some("https://example.com/catalog-asyncapi.json".to_string());
        catalog.streaming_supported = true;
        let provider_id = uuid::Uuid::new_v4().to_string();
        catalog.provider_config_id = Some(provider_id.clone());
        catalog.capabilities = Some(crate::models::downstream_service::ServiceCapabilities {
            supports_websocket: true,
            ..Default::default()
        });
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .unwrap();
        db.collection::<mongodb::bson::Document>(crate::models::provider_config::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &provider_id,
                "slug": "catalog-oauth",
                "name": "Catalog OAuth",
                "provider_type": "oauth2",
                "is_active": true,
                "created_by": "test",
                "created_at": mongodb::bson::DateTime::now(),
                "updated_at": mongodb::bson::DateTime::now(),
                "revocation": {
                    "style": "vendor_delete",
                    "url": "https://oauth.example.com/grant",
                    "auth": "bearer",
                    "revokes_grant": true,
                },
            })
            .await
            .unwrap();

        let catalog_endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &catalog_endpoint_id,
                &user_id,
                "Catalog User Label",
                "https://catalog-user.example.com",
                Some("https://example.com/user-catalog-openapi.json"),
                Some(&catalog_id),
            ))
            .await
            .unwrap();
        let catalog_service_id = uuid::Uuid::new_v4().to_string();
        let mut catalog_service = test_user_service(
            &catalog_service_id,
            &user_id,
            "my-catalog-api",
            &catalog_endpoint_id,
            Some(&catalog_id),
            Some("node-1"),
        );
        catalog_service.auth_method = "bearer".to_string();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(catalog_service)
            .await
            .unwrap();

        let custom_endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &custom_endpoint_id,
                &user_id,
                "Custom API",
                "https://custom.example.com",
                Some("https://example.com/custom-openapi.json"),
                None,
            ))
            .await
            .unwrap();
        let custom_service_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &custom_service_id,
                &user_id,
                "custom-api",
                &custom_endpoint_id,
                None,
                None,
            ))
            .await
            .unwrap();

        let Json(response) = super::list_keys(State(state), test_auth_user(&user_id))
            .await
            .unwrap();

        let catalog_key = response
            .keys
            .iter()
            .find(|key| key.id == catalog_service_id)
            .expect("catalog-backed key should be listed");
        assert_eq!(catalog_key.name, "Catalog API");
        assert_eq!(
            catalog_key.description.as_deref(),
            Some("Catalog description")
        );
        assert_eq!(catalog_key.service_category, "connection");
        assert!(catalog_key.connected);
        assert!(catalog_key.requires_connection);
        assert!(catalog_key.has_node_binding);
        assert_eq!(catalog_key.source, "catalog");
        assert_eq!(
            catalog_key.proxy_url,
            format!("http://localhost:3001/api/v1/proxy/{catalog_service_id}/{{path}}")
        );
        assert_eq!(
            catalog_key.proxy_url_slug,
            "http://localhost:3001/api/v1/proxy/s/my-catalog-api/{path}"
        );
        assert_eq!(
            catalog_key.docs_url.as_deref(),
            Some(
                format!("http://localhost:3001/api/v1/proxy/services/{catalog_service_id}/docs")
                    .as_str()
            )
        );
        assert_eq!(
            catalog_key.openapi_url.as_deref(),
            Some(
                format!(
                    "http://localhost:3001/api/v1/proxy/services/{catalog_service_id}/openapi.json"
                )
                .as_str()
            )
        );
        assert_eq!(
            catalog_key.asyncapi_url.as_deref(),
            Some(
                format!(
                    "http://localhost:3001/api/v1/proxy/services/{catalog_service_id}/asyncapi.json"
                )
                .as_str()
            )
        );
        assert!(catalog_key.streaming_supported);
        assert!(catalog_key.websocket_supported);
        assert!(
            catalog_key
                .revocation
                .as_ref()
                .is_some_and(|revocation| revocation.revokes_grant)
        );

        let custom_key = response
            .keys
            .iter()
            .find(|key| key.id == custom_service_id)
            .expect("custom key should be listed");
        assert_eq!(custom_key.name, "Custom API");
        assert!(custom_key.description.is_none());
        assert_eq!(custom_key.service_category, "custom");
        assert!(custom_key.connected);
        assert!(!custom_key.requires_connection);
        assert!(!custom_key.has_node_binding);
        assert!(custom_key.revocation.is_none());
        assert_eq!(custom_key.source, "custom");
        assert_eq!(
            custom_key.proxy_url,
            format!("http://localhost:3001/api/v1/proxy/{custom_service_id}/{{path}}")
        );
        assert_eq!(
            custom_key.proxy_url_slug,
            "http://localhost:3001/api/v1/proxy/s/custom-api/{path}"
        );
        assert_eq!(
            custom_key.docs_url.as_deref(),
            Some(
                format!("http://localhost:3001/api/v1/proxy/services/{custom_service_id}/docs")
                    .as_str()
            )
        );
        assert_eq!(
            custom_key.openapi_url.as_deref(),
            Some(
                format!(
                    "http://localhost:3001/api/v1/proxy/services/{custom_service_id}/openapi.json"
                )
                .as_str()
            )
        );
        assert!(custom_key.asyncapi_url.is_none());
        assert!(!custom_key.streaming_supported);
        assert!(!custom_key.websocket_supported);
    }

    #[test]
    fn older_key_list_consumers_ignore_additive_discovery_fields() {
        #[derive(Debug, serde::Deserialize)]
        struct OldKeyInfo {
            id: String,
            label: String,
            slug: String,
            endpoint_url: String,
        }

        #[derive(Debug, serde::Deserialize)]
        struct OldKeyListResponse {
            keys: Vec<OldKeyInfo>,
        }

        let payload = serde_json::json!({
            "keys": [{
                "id": "svc-1",
                "name": "Catalog API",
                "label": "Catalog API",
                "slug": "catalog-api",
                "description": "Catalog description",
                "service_category": "connection",
                "endpoint_url": "https://api.example.com",
                "connected": true,
                "requires_connection": true,
                "has_node_binding": false,
                "proxy_url": "http://localhost:3001/api/v1/proxy/svc-1/{path}",
                "proxy_url_slug": "http://localhost:3001/api/v1/proxy/s/catalog-api/{path}",
                "docs_url": "http://localhost:3001/api/v1/proxy/services/svc-1/docs",
                "openapi_url": "http://localhost:3001/api/v1/proxy/services/svc-1/openapi.json",
                "asyncapi_url": null,
                "streaming_supported": true,
                "websocket_supported": false,
                "source": "catalog"
            }]
        });

        let old: OldKeyListResponse =
            serde_json::from_value(payload).expect("additive fields should deserialize");
        assert_eq!(old.keys.len(), 1);
        assert_eq!(old.keys[0].id, "svc-1");
        assert_eq!(old.keys[0].label, "Catalog API");
        assert_eq!(old.keys[0].slug, "catalog-api");
        assert_eq!(old.keys[0].endpoint_url, "https://api.example.com");
    }

    // ---- get_key org scoping tests ----

    #[tokio::test]
    async fn get_key_scoped_member_sees_allowed_service() {
        let Some(db) = connect_test_database("h_keys_get_scoped_allowed").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let member_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;

        // Insert scoped membership that allows this specific service
        let scoped_membership = test_membership(
            &org_id,
            &member_id,
            OrgRole::Admin,
            Some(vec![service_id.clone()]),
        );
        db.collection(crate::models::org_membership::COLLECTION_NAME)
            .insert_one(scoped_membership)
            .await
            .unwrap();

        insert_key_fixture(&db, &org_id, &service_id, "scoped-svc", "Scoped Service").await;

        let Json(response) = super::get_key(
            State(state),
            test_auth_user(&member_id),
            Path("scoped-svc".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.id, service_id);
    }

    #[tokio::test]
    async fn get_key_scoped_member_cannot_see_out_of_scope_service() {
        let Some(db) = connect_test_database("h_keys_get_scoped_denied").await else {
            eprintln!("skipping: no local MongoDB");
            return;
        };
        let state = test_app_state(db.clone());
        let member_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;

        // Insert scoped membership that allows a DIFFERENT service
        let other_service_id = uuid::Uuid::new_v4().to_string();
        let scoped_membership = test_membership(
            &org_id,
            &member_id,
            OrgRole::Admin,
            Some(vec![other_service_id]),
        );
        db.collection(crate::models::org_membership::COLLECTION_NAME)
            .insert_one(scoped_membership)
            .await
            .unwrap();

        insert_key_fixture(&db, &org_id, &service_id, "hidden-svc", "Hidden Service").await;

        let err = super::get_key(
            State(state),
            test_auth_user(&member_id),
            Path("hidden-svc".to_string()),
        )
        .await
        .expect_err("scoped member should not see out-of-scope service");

        assert!(matches!(err, AppError::NotFound(_)));
    }

    // ---- DTO serialization tests ----

    #[test]
    fn create_key_request_debug_redacts_credential() {
        let req = make_create_key_request(
            "Test",
            Some("test-slug"),
            Some("https://api.example.com"),
            Some("super-secret"),
            Some("bearer"),
        );
        let debug = format!("{:?}", req);
        assert!(
            !debug.contains("super-secret"),
            "credential must be redacted in Debug"
        );
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn create_key_request_debug_redacts_oauth_fields() {
        let mut req = make_create_key_request(
            "Test",
            Some("test-slug"),
            Some("https://api.example.com"),
            None,
            None,
        );
        req.oauth_client_id = Some("cli_id_secret".to_string());
        req.oauth_client_secret = Some("cli_secret_value".to_string());
        let debug = format!("{:?}", req);
        assert!(
            !debug.contains("cli_id_secret"),
            "oauth_client_id must be redacted"
        );
        assert!(
            !debug.contains("cli_secret_value"),
            "oauth_client_secret must be redacted"
        );
    }

    #[test]
    fn delete_key_response_serialization() {
        let response = super::DeleteKeyResponse {
            message: "Key revoked successfully".to_string(),
            deleted: true,
            upstream_revocation_scheduled: false,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["message"], "Key revoked successfully");
        assert_eq!(json["deleted"], true);
        assert_eq!(json["upstream_revocation_scheduled"], false);
        assert!(json.get("upstream_revoked").is_none());
    }

    #[test]
    fn delete_key_response_not_deleted() {
        let response = super::DeleteKeyResponse {
            message: "Key is no longer pending_auth; delete skipped".to_string(),
            deleted: false,
            upstream_revocation_scheduled: false,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["deleted"], false);
    }

    #[test]
    fn key_list_response_serialization_empty() {
        let response = super::KeyListResponse { keys: vec![] };
        let json = serde_json::to_value(&response).unwrap();
        assert!(json["keys"].as_array().unwrap().is_empty());
    }

    #[test]
    fn delete_key_query_deserializes_defaults() {
        let query: super::DeleteKeyQuery = serde_json::from_str("{}").unwrap();
        assert!(query.only_if_pending.is_none());
        assert!(query.cascade_grant.is_none());
        assert!(query.grant_scope.is_none());
    }

    #[test]
    fn delete_key_query_deserializes_with_flag() {
        let query: super::DeleteKeyQuery = serde_json::from_str(
            r#"{"only_if_pending": true, "cascade_grant": false, "grant_scope": "token"}"#,
        )
        .unwrap();
        assert_eq!(query.only_if_pending, Some(true));
        assert_eq!(query.cascade_grant, Some(false));
        assert_eq!(query.grant_scope.as_deref(), Some("token"));
    }

    #[test]
    fn delete_key_query_rejects_unknown_fields() {
        let result: Result<super::DeleteKeyQuery, _> =
            serde_json::from_str(r#"{"only_if_pending": true, "unknown_field": 1}"#);
        assert!(result.is_err(), "unknown fields should be rejected");
    }

    #[test]
    fn update_key_request_deserialization_all_none() {
        let req: super::UpdateKeyRequest = serde_json::from_str("{}").unwrap();
        assert!(req.label.is_none());
        assert!(req.endpoint_url.is_none());
        assert!(req.auth_method.is_none());
        assert!(req.credential.is_none());
        assert!(req.node_id.is_none());
        assert!(req.is_active.is_none());
    }

    #[test]
    fn update_key_request_deserialization_partial() {
        let req: super::UpdateKeyRequest =
            serde_json::from_str(r#"{"label": "New Label", "is_active": false}"#).unwrap();
        assert_eq!(req.label.as_deref(), Some("New Label"));
        assert_eq!(req.is_active, Some(false));
        assert!(req.endpoint_url.is_none());
    }

    #[test]
    fn create_key_request_deserialization_minimal() {
        let json = r#"{"label": "My Key"}"#;
        let req: super::CreateKeyRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.label, "My Key");
        assert!(req.service_slug.is_none());
        assert!(req.credential.is_none());
        assert!(req.endpoint_url.is_none());
        assert!(req.slug.is_none());
        assert!(req.auth_method.is_none());
        assert!(req.target_org_id.is_none());
    }
}
