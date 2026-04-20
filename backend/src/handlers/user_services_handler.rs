use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::org_membership::OrgRole;
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::user_service_service::{CredentialSource, UserServiceWithSource};
use crate::services::{node_service, org_service, unified_key_service, user_service_service};

/// Resolve which user_id owns this user service and whether the actor may
/// modify it (directly or as an org admin). Returns the effective owner_id
/// (which may be an org user_id) for downstream service calls.
///
/// Honors the membership's `allowed_service_ids` scope. An org admin whose
/// scope excludes this service gets `NotFound`.
async fn resolve_service_write_owner(
    state: &AppState,
    actor: &str,
    service_id: &str,
) -> AppResult<String> {
    let svc = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User service not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &svc.user_id).await?;
    if !access.can_read() || !access.allows_resource(&svc.id) {
        return Err(AppError::NotFound("User service not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this service".to_string(),
        ));
    }
    Ok(svc.user_id)
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateUserServiceRequest {
    pub auth_method: Option<String>,
    pub auth_key_name: Option<String>,
    /// "" to clear, Some(id) to set, None to leave unchanged
    pub node_id: Option<String>,
    pub node_priority: Option<i32>,
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
    /// with a validated list. The `nullable_field` helper preserves
    /// the omitted-vs-null distinction through serde — a plain
    /// `Option<Option<_>>` collapses both to `None`.
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub default_request_headers:
        Option<Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserServiceResponse {
    pub id: String,
    pub slug: String,
    pub endpoint_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    pub auth_method: String,
    pub auth_key_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub node_priority: i32,
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
    /// Per-user default HTTP headers injected on every proxied request
    /// (NyxID#356). Returns the user-owned entries only; catalog-level
    /// defaults inherited from the `DownstreamService` are surfaced on
    /// the catalog response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_request_headers:
        Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
    pub created_at: String,
    pub updated_at: String,
    /// Provenance: personal credentials, or inherited from an org membership.
    /// Always present in list responses; the single-item update/delete
    /// responses use Personal as a sensible default since they only operate
    /// on personally-owned services.
    pub credential_source: CredentialSourceResponse,
}

/// Wire-format provenance tag mirroring
/// [`crate::services::user_service_service::CredentialSource`].
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialSourceResponse {
    Personal,
    Org {
        org_id: String,
        org_name: String,
        role: OrgRoleResponse,
        allowed: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrgRoleResponse {
    Admin,
    Member,
    Viewer,
}

impl From<OrgRole> for OrgRoleResponse {
    fn from(role: OrgRole) -> Self {
        match role {
            OrgRole::Admin => Self::Admin,
            OrgRole::Member => Self::Member,
            OrgRole::Viewer => Self::Viewer,
        }
    }
}

impl From<CredentialSource> for CredentialSourceResponse {
    fn from(source: CredentialSource) -> Self {
        match source {
            CredentialSource::Personal => Self::Personal,
            CredentialSource::Org {
                org_user_id,
                org_name,
                role,
                allowed,
            } => Self::Org {
                org_id: org_user_id,
                org_name,
                role: role.into(),
                allowed,
            },
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserServiceListResponse {
    pub services: Vec<UserServiceResponse>,
}

#[utoipa::path(
    get,
    path = "/api/v1/user-services",
    responses(
        (status = 200, description = "List of user's proxy routing configs", body = UserServiceListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "User Services"
)]
/// GET /api/v1/user-services
///
/// Returns the union of personal and org-inherited services. Each item is
/// tagged with `credential_source` so the client can group personal vs.
/// org credentials. Viewer-role services are returned with `allowed: false`.
pub async fn list_user_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UserServiceListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let services =
        user_service_service::list_user_services_with_sources(&state.db, &user_id_str).await?;
    let items = services
        .into_iter()
        .map(user_service_with_source_response)
        .collect();
    Ok(Json(UserServiceListResponse { services: items }))
}

#[utoipa::path(
    put,
    path = "/api/v1/user-services/{service_id}",
    params(
        ("service_id" = String, Path, description = "User service ID")
    ),
    request_body = UpdateUserServiceRequest,
    responses(
        (status = 200, description = "Updated user service", body = UserServiceResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "User Services"
)]
/// PUT /api/v1/user-services/{service_id}
pub async fn update_user_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<UpdateUserServiceRequest>,
) -> AppResult<Json<UserServiceResponse>> {
    let actor = auth_user.user_id.to_string();
    let user_id_str = resolve_service_write_owner(&state, &actor, &service_id).await?;
    // The actor (human/API key) is what determines node access -- see
    // `user_service_service::update_user_service` doc for the rationale.
    let actor_for_node_check = actor.clone();

    // Load current state before update (needed for node binding sync).
    let current =
        user_service_service::get_user_service(&state.db, &user_id_str, &service_id).await?;

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
                .unwrap_or(current.identity_propagation_mode.clone()),
            identity_include_user_id: body
                .identity_include_user_id
                .unwrap_or(current.identity_include_user_id),
            identity_include_email: body
                .identity_include_email
                .unwrap_or(current.identity_include_email),
            identity_include_name: body
                .identity_include_name
                .unwrap_or(current.identity_include_name),
            identity_jwt_audience: if body.identity_jwt_audience.is_some() {
                body.identity_jwt_audience
            } else {
                current.identity_jwt_audience.clone()
            },
            forward_access_token: body
                .forward_access_token
                .unwrap_or(current.forward_access_token),
            inject_delegation_token: body
                .inject_delegation_token
                .unwrap_or(current.inject_delegation_token),
            delegation_token_scope: body
                .delegation_token_scope
                .unwrap_or(current.delegation_token_scope.clone()),
        })
    } else {
        None
    };

    user_service_service::update_user_service(
        &state.db,
        &user_id_str,
        &actor_for_node_check,
        &service_id,
        body.auth_method.as_deref(),
        body.auth_key_name.as_deref(),
        body.node_id.as_deref(),
        body.node_priority,
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
            &service_id,
        )
        .await?;
    }

    // Auto-sync NodeServiceBinding when node_id changes. The actor (a
    // human or API key) is what owns the node; for org-shared services
    // the binding is owned by the org but the node is the actor's.
    if body.node_id.is_some() {
        node_service::sync_node_binding_for_user_service(
            &state.db,
            &user_id_str,
            &actor,
            current.catalog_service_id.as_deref(),
            body.node_id.as_deref(),
            current.node_id.as_deref(),
        )
        .await?;
    }

    let svc = user_service_service::get_user_service(&state.db, &user_id_str, &service_id).await?;
    Ok(Json(user_service_response(svc)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/user-services/{service_id}",
    params(
        ("service_id" = String, Path, description = "User service ID")
    ),
    responses(
        (status = 204, description = "User service deactivated"),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "User Services"
)]
/// DELETE /api/v1/user-services/{service_id}
pub async fn delete_user_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let user_id_str = resolve_service_write_owner(&state, &actor, &service_id).await?;

    // Load current state to clean up node binding.
    let current =
        user_service_service::get_user_service(&state.db, &user_id_str, &service_id).await?;

    user_service_service::deactivate_user_service(&state.db, &user_id_str, &actor, &service_id)
        .await?;

    // Deactivate the node binding if this service was node-routed.
    node_service::sync_node_binding_for_user_service(
        &state.db,
        &user_id_str,
        &actor,
        current.catalog_service_id.as_deref(),
        None, // new node_id = none (cleared)
        current.node_id.as_deref(),
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

fn user_service_response(svc: UserService) -> UserServiceResponse {
    user_service_with_source_response(UserServiceWithSource {
        service: svc,
        source: CredentialSource::Personal,
    })
}

fn user_service_with_source_response(item: UserServiceWithSource) -> UserServiceResponse {
    let UserServiceWithSource {
        service: svc,
        source,
    } = item;
    UserServiceResponse {
        id: svc.id,
        slug: svc.slug,
        endpoint_id: svc.endpoint_id,
        api_key_id: svc.api_key_id,
        auth_method: svc.auth_method,
        auth_key_name: svc.auth_key_name,
        catalog_service_id: svc.catalog_service_id,
        node_id: svc.node_id,
        node_priority: svc.node_priority,
        is_active: svc.is_active,
        identity_propagation_mode: svc.identity_propagation_mode,
        identity_include_user_id: svc.identity_include_user_id,
        identity_include_email: svc.identity_include_email,
        identity_include_name: svc.identity_include_name,
        identity_jwt_audience: svc.identity_jwt_audience,
        forward_access_token: svc.forward_access_token,
        inject_delegation_token: svc.inject_delegation_token,
        delegation_token_scope: svc.delegation_token_scope,
        custom_user_agent: svc.custom_user_agent,
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            svc.default_request_headers,
        ),
        created_at: svc.created_at.to_rfc3339(),
        updated_at: svc.updated_at.to_rfc3339(),
        credential_source: source.into(),
    }
}
