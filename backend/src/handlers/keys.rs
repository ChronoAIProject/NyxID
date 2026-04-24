use axum::{
    Json,
    extract::{Path, Query, State},
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user_api_key::UserApiKey;
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::models::ws_frame_injection::WsFrameInjection;
use crate::mw::auth::AuthUser;
use crate::services::{
    catalog_service, credential_push_service, lark_permission, node_service, org_service,
    unified_key_service, user_api_key_service, user_endpoint_service, user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

/// Resolve which user_id owns this unified key (= UserService) and whether
/// the actor may modify it. Returns the effective owner_id (which may be an
/// org user_id) for downstream service calls.
///
/// Enforces both role (direct owner / org admin) AND the membership's
/// per-service `allowed_service_ids` scope. A scoped admin whose scope does
/// not include this key returns NotFound (same shape as a non-existent key)
/// to avoid leaking org topology.
async fn resolve_key_write_owner(state: &AppState, actor: &str, key_id: &str) -> AppResult<String> {
    let svc = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": key_id })
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
    Ok(svc.user_id)
}

/// Outcome of `resolve_key_read_owner`: the effective owner id used for
/// downstream service calls, plus the credential source for the response.
struct KeyReadAccess {
    owner_id: String,
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

    let svc = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Key not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &svc.user_id).await?;
    if !access.can_read() || !access.allows_resource(&svc.id) {
        return Err(AppError::NotFound("Key not found".to_string()));
    }

    let source = match &access {
        org_service::OwnerAccess::Direct => CredentialSource::Personal,
        org_service::OwnerAccess::AsOrgAdmin { org_user_id, .. } => {
            // Look up the org's display_name for the response payload.
            let org_name = state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! { "_id": org_user_id })
                .await?
                .and_then(|u| u.display_name)
                .unwrap_or_else(|| "Unnamed Org".to_string());
            CredentialSource::Org {
                org_user_id: org_user_id.clone(),
                org_name,
                role: crate::models::org_membership::OrgRole::Admin,
                allowed: true,
            }
        }
        org_service::OwnerAccess::AsOrgMember {
            org_user_id, role, ..
        } => {
            let org_name = state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! { "_id": org_user_id })
                .await?
                .and_then(|u| u.display_name)
                .unwrap_or_else(|| "Unnamed Org".to_string());
            // Members can proxy/use; viewers cannot. (Scope has already been
            // enforced above via allows_resource; if we got here, this
            // particular key is within the member's scope.)
            let allowed = role.can_proxy();
            CredentialSource::Org {
                org_user_id: org_user_id.clone(),
                org_name,
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
    /// SSH host (required for custom SSH services)
    pub ssh_host: Option<String>,
    /// SSH port (default: 22)
    pub ssh_port: Option<u16>,
    /// Enable SSH certificate auth (default: true)
    pub ssh_certificate_auth: Option<bool>,
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
            .field("ssh_host", &self.ssh_host)
            .field("ssh_port", &self.ssh_port)
            .field("ssh_certificate_auth", &self.ssh_certificate_auth)
            .field("ssh_principals", &self.ssh_principals)
            .field(
                "ssh_certificate_ttl_minutes",
                &self.ssh_certificate_ttl_minutes,
            )
            .field("identity_propagation_mode", &self.identity_propagation_mode)
            .field("forward_access_token", &self.forward_access_token)
            .field("inject_delegation_token", &self.inject_delegation_token)
            .field("target_org_id", &self.target_org_id)
            .finish()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KeyResponse {
    pub id: String,
    pub label: String,
    pub slug: String,
    pub endpoint_url: String,
    pub endpoint_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    pub credential_type: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub node_priority: i32,
    pub service_type: String,
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

    // Build SSH params if SSH-specific fields are present
    let ssh_params = body.ssh_host.as_deref().map(|host| {
        let principals_str = body.ssh_principals.as_deref().unwrap_or("");
        let principals: Vec<String> = principals_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        unified_key_service::SshCreateParams {
            host,
            port: body.ssh_port.unwrap_or(22),
            certificate_auth: body.ssh_certificate_auth.unwrap_or(true),
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

    let result = unified_key_service::create_key(
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
    )
    .await?;

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
        let org_name = org
            .and_then(|u| u.display_name)
            .unwrap_or_else(|| "Unnamed Org".to_string());
        response.credential_source = CredentialSourceResponse::Org {
            org_id: target_org_id.to_string(),
            org_name,
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

    // Lazily auto-provision no-auth catalog services for the user
    unified_key_service::auto_provision_no_auth_services(&state.db, &user_id_str).await?;

    let views = unified_key_service::list_keys(&state.db, &user_id_str).await?;
    let keys = views.into_iter().map(key_response_from_view).collect();
    Ok(Json(KeyListResponse { keys }))
}

#[utoipa::path(
    get,
    path = "/api/v1/keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "User service ID")
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
    let access = resolve_key_read_owner(&state, &actor, &key_id).await?;
    let mut view = unified_key_service::get_key(&state.db, &access.owner_id, &key_id).await?;
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
        &state,
        &owner_id,
        catalog_id.as_deref(),
        catalog_slug.as_deref(),
        api_key_id.as_deref(),
    )
    .await;
    response.permission_setup_url = permission_url;
    response.permission_setup_scopes = permission_scopes;
    Ok(Json(response))
}

#[utoipa::path(
    put,
    path = "/api/v1/keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "User service ID")
    ),
    request_body = UpdateKeyRequest,
    responses(
        (status = 200, description = "Key updated", body = KeyResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
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
    let user_id_str = resolve_key_write_owner(&state, &actor, &key_id).await?;

    // Load current state to find sub-resource IDs
    let view = unified_key_service::get_key(&state.db, &user_id_str, &key_id).await?;

    if view.auto_connected {
        return Err(crate::errors::AppError::BadRequest(
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
        unified_key_service::ensure_user_api_key_for_update(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            &key_id,
            body.auth_method.as_deref(),
            body.credential.as_deref(),
            body.node_id.as_deref(),
            preferred_label,
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
    let refreshed_api_key_id = unified_key_service::get_key(&state.db, &user_id_str, &key_id)
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
            unified_key_service::get_key(&state.db, &user_id_str, &key_id)
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
    let url_update = body.endpoint_url.as_deref();
    if url_update.is_some()
        || !matches!(
            spec_url_update,
            user_endpoint_service::OpenApiSpecUrlUpdate::Leave
        )
    {
        user_endpoint_service::update_endpoint(
            &state.db,
            &user_id_str,
            &view.endpoint_id,
            url_update,
            None,
            spec_url_update,
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
    let updated = unified_key_service::get_key(&state.db, &user_id_str, &key_id).await?;
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
}

#[utoipa::path(
    delete,
    path = "/api/v1/keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "User service ID"),
        ("only_if_pending" = Option<bool>, Query, description = "When true, skip the delete if the key is no longer pending_auth")
    ),
    responses(
        (status = 200, description = "Key revoked (or skipped when only_if_pending)", body = DeleteKeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
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
    let user_id_str = resolve_key_write_owner(&state, &actor, &key_id).await?;

    let view = unified_key_service::get_key(&state.db, &user_id_str, &key_id).await?;
    if view.auto_connected {
        return Err(crate::errors::AppError::BadRequest(
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
        }));
    }

    unified_key_service::revoke_key(&state.db, &user_id_str, &actor, &key_id).await?;

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::KeyDeleted {
            source: if view.catalog_service_slug.is_some() {
                "catalog".to_string()
            } else {
                "custom".to_string()
            },
        },
    );

    Ok(Json(DeleteKeyResponse {
        message: "Key revoked successfully".to_string(),
        deleted: true,
    }))
}

fn key_response_from_result(result: &unified_key_service::CreateKeyResult) -> KeyResponse {
    KeyResponse {
        id: result.service.id.clone(),
        label: result.api_key.as_ref().map_or_else(
            || result.endpoint.label.clone(),
            |api_key| api_key.label.clone(),
        ),
        slug: result.service.slug.clone(),
        endpoint_url: result.endpoint.url.clone(),
        endpoint_id: result.endpoint.id.clone(),
        api_key_id: result.api_key.as_ref().map(|api_key| api_key.id.clone()),
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
        catalog_service_id: result.service.catalog_service_id.clone(),
        catalog_service_slug: None,
        catalog_service_name: None,
        node_id: result.service.node_id.clone(),
        node_priority: result.service.node_priority,
        service_type: result.service.service_type.clone(),
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
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            result.service.default_request_headers.clone(),
        ),
        ws_frame_injections: result.service.ws_frame_injections.clone(),
        auto_connected: false,
        source_app_id: None,
        source_app_name: None,
        expires_at: result
            .api_key
            .as_ref()
            .and_then(|api_key| api_key.expires_at.map(|dt| dt.to_rfc3339())),
        last_used_at: None,
        error_message: None,
        created_at: result.service.created_at.to_rfc3339(),
        ssh_host: result.ssh_host.clone(),
        ssh_port: result.ssh_port,
        ssh_ca_public_key: result.ssh_ca_public_key.clone(),
        ssh_allowed_principals: result.ssh_allowed_principals.clone(),
        ssh_certificate_ttl_minutes: result.ssh_certificate_ttl_minutes,
        openapi_spec_url: result.endpoint.openapi_spec_url.clone(),
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
    KeyResponse {
        id: view.id,
        label: view.label,
        slug: view.slug,
        endpoint_url: view.endpoint_url,
        endpoint_id: view.endpoint_id,
        api_key_id: view.api_key_id,
        credential_type: view.credential_type,
        auth_method: view.auth_method,
        auth_key_name: view.auth_key_name,
        status: view.status,
        catalog_service_id: view.catalog_service_id,
        catalog_service_slug: view.catalog_service_slug,
        catalog_service_name: view.catalog_service_name,
        node_id: view.node_id,
        node_priority: view.node_priority,
        service_type: view.service_type,
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
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            view.default_request_headers,
        ),
        ws_frame_injections: view.ws_frame_injections,
        auto_connected: view.auto_connected,
        source_app_id: view.source_app_id,
        source_app_name: view.source_app_name,
        expires_at: view.expires_at,
        last_used_at: view.last_used_at,
        error_message: view.error_message,
        created_at: view.created_at,
        ssh_host: view.ssh_host,
        ssh_port: view.ssh_port,
        ssh_ca_public_key: view.ssh_ca_public_key,
        ssh_allowed_principals: view.ssh_allowed_principals,
        ssh_certificate_ttl_minutes: view.ssh_certificate_ttl_minutes,
        openapi_spec_url: view.openapi_spec_url,
        credential_source: view.credential_source.into(),
        permission_setup_url: None,
        permission_setup_scopes: None,
    }
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
    use crate::models::user_api_key::UserApiKey;
    use chrono::Utc;

    fn test_encryption_keys() -> EncryptionKeys {
        EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([0x22; 32], None)))
    }

    fn make_blank_api_key() -> UserApiKey {
        UserApiKey {
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
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: None,
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
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
}
