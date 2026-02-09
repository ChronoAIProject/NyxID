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
