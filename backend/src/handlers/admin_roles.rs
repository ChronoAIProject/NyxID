use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::errors::AppResult;
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent, require_admin};
use crate::models::role::Role;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, role_service};
use crate::AppState;

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_default: Option<bool>,
    pub client_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub permissions: Option<Vec<String>>,
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RoleListQuery {
    pub client_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RoleResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_default: bool,
    pub is_system: bool,
    pub client_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct RoleListResponse {
    pub roles: Vec<RoleResponse>,
}

#[derive(Debug, Serialize)]
pub struct UserRolesResponse {
    pub direct_roles: Vec<RoleResponse>,
    pub inherited_roles: Vec<RoleResponse>,
    pub effective_permissions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RoleAssignmentResponse {
    pub message: String,
}

// --- Helpers ---

fn role_to_response(r: Role) -> RoleResponse {
    RoleResponse {
        id: r.id,
        name: r.name,
        slug: r.slug,
        description: r.description,
        permissions: r.permissions,
        is_default: r.is_default,
        is_system: r.is_system,
        client_id: r.client_id,
        created_at: r.created_at.to_rfc3339(),
        updated_at: r.updated_at.to_rfc3339(),
    }
}

// --- Handlers ---

/// GET /api/v1/admin/roles
pub async fn list_roles(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<RoleListQuery>,
) -> AppResult<Json<RoleListResponse>> {
    require_admin(&state, &auth_user).await?;

    let roles = role_service::list_roles(&state.db, query.client_id.as_deref()).await?;
    let items: Vec<RoleResponse> = roles.into_iter().map(role_to_response).collect();

    Ok(Json(RoleListResponse { roles: items }))
}

/// POST /api/v1/admin/roles
pub async fn create_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(body): Json<CreateRoleRequest>,
) -> AppResult<Json<RoleResponse>> {
    require_admin(&state, &auth_user).await?;

    if body.name.is_empty() || body.slug.is_empty() {
        return Err(crate::errors::AppError::ValidationError(
            "Name and slug are required".to_string(),
        ));
    }

    let role = role_service::create_role(
        &state.db,
        &body.name,
        &body.slug,
        body.description.as_deref(),
        &body.permissions,
        body.is_default.unwrap_or(false),
        body.client_id.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.role.created".to_string(),
        Some(serde_json::json!({
            "role_id": &role.id,
            "role_slug": &role.slug,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(role_to_response(role)))
}

/// GET /api/v1/admin/roles/:role_id
pub async fn get_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(role_id): Path<String>,
) -> AppResult<Json<RoleResponse>> {
    require_admin(&state, &auth_user).await?;

    let role = role_service::get_role(&state.db, &role_id).await?;
    Ok(Json(role_to_response(role)))
}

/// PUT /api/v1/admin/roles/:role_id
pub async fn update_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(role_id): Path<String>,
    Json(body): Json<UpdateRoleRequest>,
) -> AppResult<Json<RoleResponse>> {
    require_admin(&state, &auth_user).await?;

    let role = role_service::update_role(
        &state.db,
        &role_id,
        body.name.as_deref(),
        body.slug.as_deref(),
        body.description.as_deref(),
        body.permissions.as_deref(),
        body.is_default,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.role.updated".to_string(),
        Some(serde_json::json!({
            "role_id": &role_id,
            "role_slug": &role.slug,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(role_to_response(role)))
}

/// DELETE /api/v1/admin/roles/:role_id
pub async fn delete_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(role_id): Path<String>,
) -> AppResult<Json<RoleAssignmentResponse>> {
    require_admin(&state, &auth_user).await?;

    role_service::delete_role(&state.db, &role_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.role.deleted".to_string(),
        Some(serde_json::json!({ "role_id": &role_id })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(RoleAssignmentResponse {
        message: "Role deleted".to_string(),
    }))
}

/// GET /api/v1/admin/users/:user_id/roles
pub async fn get_user_roles(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(user_id): Path<String>,
) -> AppResult<Json<UserRolesResponse>> {
    require_admin(&state, &auth_user).await?;

    let rbac = crate::services::rbac_helpers::resolve_user_rbac(&state.db, &user_id).await?;
    let direct_roles = role_service::get_user_roles(&state.db, &user_id).await?;

    // Inherited roles = effective roles minus direct roles.
    // Use a targeted $in query on inherited slugs instead of fetching all roles.
    let direct_slugs: std::collections::HashSet<&str> =
        direct_roles.iter().map(|r| r.slug.as_str()).collect();
    let inherited_slugs: Vec<&str> = rbac
        .role_slugs
        .iter()
        .filter(|s| !direct_slugs.contains(s.as_str()))
        .map(|s| s.as_str())
        .collect();

    let inherited_roles = if inherited_slugs.is_empty() {
        vec![]
    } else {
        use futures::TryStreamExt;
        use mongodb::bson::doc;
        state
            .db
            .collection::<Role>(crate::models::role::COLLECTION_NAME)
            .find(doc! { "slug": { "$in": &inherited_slugs } })
            .await?
            .try_collect()
            .await?
    };

    Ok(Json(UserRolesResponse {
        direct_roles: direct_roles.into_iter().map(role_to_response).collect(),
        inherited_roles: inherited_roles
            .into_iter()
            .map(role_to_response)
            .collect(),
        effective_permissions: rbac.permissions,
    }))
}

/// POST /api/v1/admin/users/:user_id/roles/:role_id
pub async fn assign_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((user_id, role_id)): Path<(String, String)>,
) -> AppResult<Json<RoleAssignmentResponse>> {
    require_admin(&state, &auth_user).await?;

    role_service::assign_role_to_user(&state.db, &user_id, &role_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.role.assigned".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "role_id": &role_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(RoleAssignmentResponse {
        message: "Role assigned".to_string(),
    }))
}

/// DELETE /api/v1/admin/users/:user_id/roles/:role_id
pub async fn revoke_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((user_id, role_id)): Path<(String, String)>,
) -> AppResult<Json<RoleAssignmentResponse>> {
    require_admin(&state, &auth_user).await?;

    role_service::revoke_role_from_user(&state.db, &user_id, &role_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.role.revoked".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "role_id": &role_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(RoleAssignmentResponse {
        message: "Role revoked".to_string(),
    }))
}
