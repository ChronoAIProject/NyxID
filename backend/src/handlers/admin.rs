use axum::{
    extract::{Path, Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::services::oauth_client_service;
use crate::AppState;

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct AdminUserItem {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email_verified: bool,
    pub is_active: bool,
    pub is_admin: bool,
    pub mfa_enabled: bool,
    pub created_at: String,
    pub last_login_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminUserListResponse {
    pub users: Vec<AdminUserItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Serialize)]
pub struct AuditLogItem {
    pub id: String,
    pub user_id: Option<String>,
    pub event_type: String,
    pub ip_address: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct AuditLogListResponse {
    pub entries: Vec<AuditLogItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

// --- Helpers ---

/// Verify that the authenticated user is an admin.
async fn require_admin(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin {
        return Err(AppError::Forbidden(
            "Admin access required".to_string(),
        ));
    }

    Ok(())
}

// --- Handlers ---

/// GET /api/v1/admin/users
///
/// List all users (admin only). Supports pagination.
pub async fn list_users(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(pagination): Query<PaginationQuery>,
) -> AppResult<Json<AdminUserListResponse>> {
    require_admin(&state, &auth_user).await?;

    let page = pagination.page.unwrap_or(1).max(1);
    let per_page = pagination.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let total = state
        .db
        .collection::<User>(USERS)
        .count_documents(doc! {})
        .await?;

    let users: Vec<User> = state
        .db
        .collection::<User>(USERS)
        .find(doc! {})
        .sort(doc! { "created_at": -1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    let items: Vec<AdminUserItem> = users
        .into_iter()
        .map(|u| AdminUserItem {
            id: u.id,
            email: u.email,
            display_name: u.display_name,
            email_verified: u.email_verified,
            is_active: u.is_active,
            is_admin: u.is_admin,
            mfa_enabled: u.mfa_enabled,
            created_at: u.created_at.to_rfc3339(),
            last_login_at: u.last_login_at.map(|t| t.to_rfc3339()),
        })
        .collect();

    Ok(Json(AdminUserListResponse {
        users: items,
        total,
        page,
        per_page,
    }))
}

/// GET /api/v1/admin/users/:user_id
///
/// Get a specific user's details (admin only).
pub async fn get_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(user_id): Path<String>,
) -> AppResult<Json<AdminUserItem>> {
    require_admin(&state, &auth_user).await?;

    let user_id_str = user_id;

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    Ok(Json(AdminUserItem {
        id: user_model.id,
        email: user_model.email,
        display_name: user_model.display_name,
        email_verified: user_model.email_verified,
        is_active: user_model.is_active,
        is_admin: user_model.is_admin,
        mfa_enabled: user_model.mfa_enabled,
        created_at: user_model.created_at.to_rfc3339(),
        last_login_at: user_model.last_login_at.map(|t| t.to_rfc3339()),
    }))
}

/// GET /api/v1/admin/audit-log
///
/// Query the audit log (admin only). Supports pagination.
pub async fn list_audit_log(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(pagination): Query<PaginationQuery>,
) -> AppResult<Json<AuditLogListResponse>> {
    require_admin(&state, &auth_user).await?;

    let page = pagination.page.unwrap_or(1).max(1);
    let per_page = pagination.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let total = state
        .db
        .collection::<AuditLog>(AUDIT_LOG)
        .count_documents(doc! {})
        .await?;

    let entries: Vec<AuditLog> = state
        .db
        .collection::<AuditLog>(AUDIT_LOG)
        .find(doc! {})
        .sort(doc! { "created_at": -1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    let items: Vec<AuditLogItem> = entries
        .into_iter()
        .map(|e| AuditLogItem {
            id: e.id,
            user_id: e.user_id,
            event_type: e.event_type,
            ip_address: e.ip_address,
            created_at: e.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(AuditLogListResponse {
        entries: items,
        total,
        page,
        per_page,
    }))
}

// --- OAuth Client Admin ---

#[derive(Debug, Deserialize)]
pub struct CreateOAuthClientRequest {
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub client_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OAuthClientResponse {
    pub id: String,
    pub client_name: String,
    pub client_type: String,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: String,
    pub is_active: bool,
    /// Raw client secret -- only returned at creation time.
    pub client_secret: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct OAuthClientListResponse {
    pub clients: Vec<OAuthClientResponse>,
}

/// POST /api/v1/admin/oauth-clients
///
/// Create a new OAuth client. Requires admin privileges.
pub async fn create_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateOAuthClientRequest>,
) -> AppResult<Json<OAuthClientResponse>> {
    require_admin(&state, &auth_user).await?;

    if body.name.is_empty() {
        return Err(AppError::ValidationError(
            "Client name is required".to_string(),
        ));
    }

    if body.redirect_uris.is_empty() {
        return Err(AppError::ValidationError(
            "At least one redirect_uri is required".to_string(),
        ));
    }

    let client_type = body.client_type.as_deref().unwrap_or("confidential");
    if client_type != "confidential" && client_type != "public" {
        return Err(AppError::ValidationError(
            "client_type must be 'confidential' or 'public'".to_string(),
        ));
    }

    let user_id = auth_user.user_id.to_string();
    let (client, raw_secret) = oauth_client_service::create_client(
        &state.db,
        &body.name,
        &body.redirect_uris,
        client_type,
        &user_id,
    )
    .await?;

    tracing::info!(
        client_id = %client.id,
        client_name = %client.client_name,
        created_by = %user_id,
        "OAuth client created"
    );

    Ok(Json(OAuthClientResponse {
        id: client.id.clone(),
        client_name: client.client_name,
        client_type: client.client_type,
        redirect_uris: client.redirect_uris,
        allowed_scopes: client.allowed_scopes,
        is_active: client.is_active,
        client_secret: raw_secret,
        created_at: client.created_at.to_rfc3339(),
    }))
}

/// GET /api/v1/admin/oauth-clients
///
/// List all OAuth clients. Requires admin privileges.
pub async fn list_oauth_clients(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<OAuthClientListResponse>> {
    require_admin(&state, &auth_user).await?;

    let clients = oauth_client_service::list_clients(&state.db).await?;

    let items: Vec<OAuthClientResponse> = clients
        .into_iter()
        .map(|c| {
            OAuthClientResponse {
                id: c.id,
                client_name: c.client_name,
                client_type: c.client_type,
                redirect_uris: c.redirect_uris,
                allowed_scopes: c.allowed_scopes,
                is_active: c.is_active,
                client_secret: None, // never expose secret in list
                created_at: c.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(OAuthClientListResponse { clients: items }))
}

/// DELETE /api/v1/admin/oauth-clients/:client_id
///
/// Deactivate an OAuth client. Requires admin privileges.
pub async fn delete_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    require_admin(&state, &auth_user).await?;

    oauth_client_service::delete_client(&state.db, &client_id).await?;

    tracing::info!(
        client_id = %client_id,
        deactivated_by = %auth_user.user_id,
        "OAuth client deactivated"
    );

    Ok(Json(serde_json::json!({ "message": "OAuth client deactivated" })))
}
