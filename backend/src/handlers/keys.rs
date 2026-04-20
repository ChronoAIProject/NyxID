use axum::{
    Json,
    extract::{Path, State},
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::{
    credential_push_service, node_service, org_service, unified_key_service, user_api_key_service,
    user_endpoint_service, user_service_service,
};

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
    Ok(Json(key_response_from_view(view)))
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

    // Update label on UserApiKey if provided (skip for auto-connected no-auth services)
    if let Some(ref label) = body.label {
        if let Some(ref ak_id) = view.api_key_id {
            user_api_key_service::update_api_key(
                &state.db,
                &state.encryption_keys,
                &user_id_str,
                ak_id,
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

    // Update endpoint URL if provided
    if let Some(ref url) = body.endpoint_url {
        user_endpoint_service::update_endpoint(
            &state.db,
            &user_id_str,
            &view.endpoint_id,
            Some(url.as_str()),
            None,
            user_endpoint_service::OpenApiSpecUrlUpdate::Leave,
        )
        .await?;
    }

    // Update OpenAPI spec URL if provided. Empty string clears.
    if let Some(ref spec_url) = body.openapi_spec_url {
        let update = if spec_url.trim().is_empty() {
            user_endpoint_service::OpenApiSpecUrlUpdate::Clear
        } else {
            user_endpoint_service::OpenApiSpecUrlUpdate::Set(spec_url.as_str())
        };
        user_endpoint_service::update_endpoint(
            &state.db,
            &user_id_str,
            &view.endpoint_id,
            None,
            None,
            update,
        )
        .await?;
    }

    let has_identity_update = body.identity_propagation_mode.is_some()
        || body.identity_include_user_id.is_some()
        || body.identity_include_email.is_some()
        || body.identity_include_name.is_some()
        || body.identity_jwt_audience.is_some()
        || body.forward_access_token.is_some()
        || body.inject_delegation_token.is_some()
        || body.delegation_token_scope.is_some();

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

        user_service_service::update_user_service(
            &state.db,
            &user_id_str,
            &actor,
            &key_id,
            body.auth_method.as_deref(),
            body.auth_key_name.as_deref(),
            body.node_id.as_deref(),
            None,
            body.is_active,
            identity.as_ref(),
            body.custom_user_agent.as_deref(),
            body.default_request_headers.as_ref(),
        )
        .await?;

        if body.node_id.is_some() || body.auth_method.is_some() {
            unified_key_service::reconcile_provider_key_for_service_routing(
                &state.db,
                &user_id_str,
                &key_id,
            )
            .await?;
        }

        // Auto-sync NodeServiceBinding when node_id changes. The actor
        // owns the node, so it must be the one validated -- the binding
        // owner (`user_id_str`) may be an org.
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
    }

    // Return refreshed view
    let updated = unified_key_service::get_key(&state.db, &user_id_str, &key_id).await?;
    Ok(Json(key_response_from_view(updated)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/keys/{key_id}",
    params(
        ("key_id" = String, Path, description = "User service ID")
    ),
    responses(
        (status = 200, description = "Key revoked", body = DeleteKeyResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "AI Services"
)]
/// DELETE /api/v1/keys/{key_id}
pub async fn delete_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<DeleteKeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let user_id_str = resolve_key_write_owner(&state, &actor, &key_id).await?;

    let view = unified_key_service::get_key(&state.db, &user_id_str, &key_id).await?;
    if view.auto_connected {
        return Err(crate::errors::AppError::BadRequest(
            "Auto-connected services cannot be deleted".to_string(),
        ));
    }

    unified_key_service::revoke_key(&state.db, &user_id_str, &actor, &key_id).await?;
    Ok(Json(DeleteKeyResponse {
        message: "Key revoked successfully".to_string(),
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
    }
}
