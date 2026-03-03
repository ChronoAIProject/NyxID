use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{admin_user_service, audit_service, consent_service, oauth_client_service};

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminUserItem {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
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
    pub event_data: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct AuditLogListResponse {
    pub entries: Vec<AuditLogItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

// --- New request/response types for admin user management ---

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct CreateUserResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub is_admin: bool,
    pub is_active: bool,
    pub email_verified: bool,
    pub created_at: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetRoleRequest {
    pub is_admin: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetStatusRequest {
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct AdminActionResponse {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct RoleUpdateResponse {
    pub id: String,
    pub is_admin: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct StatusUpdateResponse {
    pub id: String,
    pub is_active: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyEmailResponse {
    pub id: String,
    pub email_verified: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSessionItem {
    pub id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub last_active_at: String,
    pub revoked: bool,
}

#[derive(Debug, Serialize)]
pub struct AdminSessionListResponse {
    pub sessions: Vec<AdminSessionItem>,
    pub total: u64,
}

#[derive(Debug, Serialize)]
pub struct RevokeSessionsResponse {
    pub revoked_count: u64,
    pub message: String,
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
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    Ok(())
}

/// Extract the client IP from headers (X-Forwarded-For) or return None.
fn extract_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Extract the User-Agent header.
fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// Convert a User model into an AdminUserItem response struct.
fn user_to_admin_item(u: User) -> AdminUserItem {
    AdminUserItem {
        id: u.id,
        email: u.email,
        display_name: u.display_name,
        avatar_url: u.avatar_url,
        email_verified: u.email_verified,
        is_active: u.is_active,
        is_admin: u.is_admin,
        mfa_enabled: u.mfa_enabled,
        created_at: u.created_at.to_rfc3339(),
        last_login_at: u.last_login_at.map(|t| t.to_rfc3339()),
    }
}

// --- Handlers ---

/// POST /api/v1/admin/users
///
/// Create a new user (admin only). The created account is pre-verified and active.
pub async fn create_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(body): Json<CreateUserRequest>,
) -> AppResult<Json<CreateUserResponse>> {
    require_admin(&state, &auth_user).await?;

    // Validate email format
    let email = body.email.trim().to_string();
    if email.is_empty() {
        return Err(AppError::ValidationError("Email is required".to_string()));
    }

    // Validate password minimum length
    if body.password.len() < 8 {
        return Err(AppError::ValidationError(
            "Password must be at least 8 characters".to_string(),
        ));
    }

    // Validate role
    if body.role != "admin" && body.role != "user" {
        return Err(AppError::ValidationError(
            "Role must be 'admin' or 'user'".to_string(),
        ));
    }

    let user = admin_user_service::create_user(
        &state.db,
        &email,
        &body.password,
        body.display_name.as_deref(),
        &body.role,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.user.created".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user.id,
            "target_email": &user.email,
            "is_admin": user.is_admin,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(CreateUserResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        is_admin: user.is_admin,
        is_active: user.is_active,
        email_verified: user.email_verified,
        created_at: user.created_at.to_rfc3339(),
        message: "User created successfully".to_string(),
    }))
}

/// GET /api/v1/admin/users
///
/// List all users (admin only). Supports pagination.
pub async fn list_users(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<UserListQuery>,
) -> AppResult<Json<AdminUserListResponse>> {
    require_admin(&state, &auth_user).await?;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let filter = match query.search.as_deref() {
        Some(s) if !s.is_empty() => {
            let escaped = regex::escape(s);
            doc! { "email": { "$regex": &escaped, "$options": "i" } }
        }
        _ => doc! {},
    };

    let total = state
        .db
        .collection::<User>(USERS)
        .count_documents(filter.clone())
        .await?;

    let users: Vec<User> = state
        .db
        .collection::<User>(USERS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    let items: Vec<AdminUserItem> = users.into_iter().map(user_to_admin_item).collect();

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

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    Ok(Json(user_to_admin_item(user_model)))
}

/// PUT /api/v1/admin/users/:user_id
///
/// Edit a user's profile fields (admin only).
pub async fn update_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<UpdateUserRequest>,
) -> AppResult<Json<AdminUserItem>> {
    require_admin(&state, &auth_user).await?;

    let updated = admin_user_service::update_user(
        &state.db,
        &user_id,
        body.display_name.as_deref(),
        body.email.as_deref(),
        body.avatar_url.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.user.updated".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "target_email": &updated.email,
            "changes": {
                "display_name": body.display_name,
                "email": body.email,
                "avatar_url": body.avatar_url,
            }
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(user_to_admin_item(updated)))
}

/// PATCH /api/v1/admin/users/:user_id/role
///
/// Toggle admin role for a user (admin only, cannot change own role).
pub async fn set_user_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<SetRoleRequest>,
) -> AppResult<Json<RoleUpdateResponse>> {
    require_admin(&state, &auth_user).await?;

    let admin_id = auth_user.user_id.to_string();

    admin_user_service::set_admin_role(&state.db, &admin_id, &user_id, body.is_admin).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.user.role_changed".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "is_admin": body.is_admin,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(RoleUpdateResponse {
        id: user_id,
        is_admin: body.is_admin,
        message: "User admin role updated".to_string(),
    }))
}

/// PATCH /api/v1/admin/users/:user_id/status
///
/// Toggle active status for a user (admin only, cannot change own status).
/// When disabling, all sessions are revoked.
pub async fn set_user_status(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<SetStatusRequest>,
) -> AppResult<Json<StatusUpdateResponse>> {
    require_admin(&state, &auth_user).await?;

    let admin_id = auth_user.user_id.to_string();

    admin_user_service::set_user_active(&state.db, &admin_id, &user_id, body.is_active).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.user.status_changed".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "is_active": body.is_active,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(StatusUpdateResponse {
        id: user_id,
        is_active: body.is_active,
        message: "User status updated".to_string(),
    }))
}

/// POST /api/v1/admin/users/:user_id/reset-password
///
/// Force a password reset for a user (admin only). Revokes all sessions.
pub async fn force_password_reset(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> AppResult<Json<AdminActionResponse>> {
    require_admin(&state, &auth_user).await?;

    let _token = admin_user_service::force_password_reset(&state.db, &user_id).await?;

    #[cfg(debug_assertions)]
    if let Some(ref t) = _token {
        tracing::debug!(token = %t, user_id = %user_id, "Admin-initiated password reset token (dev only)");
    }

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.user.password_reset".to_string(),
        Some(serde_json::json!({ "target_user_id": &user_id })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminActionResponse {
        message: "Password reset initiated".to_string(),
    }))
}

/// DELETE /api/v1/admin/users/:user_id
///
/// Delete a user with full cascade cleanup (admin only, cannot delete self).
pub async fn delete_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> AppResult<Json<AdminActionResponse>> {
    require_admin(&state, &auth_user).await?;

    let admin_id = auth_user.user_id.to_string();

    // Fetch user email before deletion for audit log
    let target = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;
    let target_email = target.email.clone();

    admin_user_service::delete_user_cascade(&state.db, &admin_id, &user_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.user.deleted".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "target_email": &target_email,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminActionResponse {
        message: "User deleted".to_string(),
    }))
}

/// PATCH /api/v1/admin/users/:user_id/verify-email
///
/// Manually verify a user's email (admin only).
pub async fn verify_user_email(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> AppResult<Json<VerifyEmailResponse>> {
    require_admin(&state, &auth_user).await?;

    admin_user_service::verify_email(&state.db, &user_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.user.email_verified".to_string(),
        Some(serde_json::json!({ "target_user_id": &user_id })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(VerifyEmailResponse {
        id: user_id,
        email_verified: true,
        message: "Email verified".to_string(),
    }))
}

/// GET /api/v1/admin/users/:user_id/sessions
///
/// List all sessions for a user (admin only).
pub async fn list_user_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(user_id): Path<String>,
) -> AppResult<Json<AdminSessionListResponse>> {
    require_admin(&state, &auth_user).await?;

    let sessions = admin_user_service::list_user_sessions(&state.db, &user_id).await?;

    let total = sessions.len() as u64;
    let items: Vec<AdminSessionItem> = sessions
        .into_iter()
        .map(|s| AdminSessionItem {
            id: s.id,
            ip_address: s.ip_address,
            user_agent: s.user_agent,
            created_at: s.created_at.to_rfc3339(),
            expires_at: s.expires_at.to_rfc3339(),
            last_active_at: s.last_active_at.to_rfc3339(),
            revoked: s.revoked,
        })
        .collect();

    Ok(Json(AdminSessionListResponse {
        sessions: items,
        total,
    }))
}

/// DELETE /api/v1/admin/users/:user_id/sessions
///
/// Revoke all sessions for a user (admin only).
pub async fn revoke_user_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> AppResult<Json<RevokeSessionsResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify user exists
    let _target = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let revoked_count = admin_user_service::revoke_all_user_sessions(&state.db, &user_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.user.sessions_revoked".to_string(),
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "revoked_count": revoked_count,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(RevokeSessionsResponse {
        revoked_count,
        message: "All sessions revoked".to_string(),
    }))
}

/// GET /api/v1/admin/audit-log
///
/// Query the audit log (admin only). Supports pagination and user_id filter.
pub async fn list_audit_log(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<AuditLogQuery>,
) -> AppResult<Json<AuditLogListResponse>> {
    require_admin(&state, &auth_user).await?;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let mut filter = doc! {};
    if let Some(ref uid) = query.user_id {
        filter.insert("user_id", uid);
    }

    let total = state
        .db
        .collection::<AuditLog>(AUDIT_LOG)
        .count_documents(filter.clone())
        .await?;

    let entries: Vec<AuditLog> = state
        .db
        .collection::<AuditLog>(AUDIT_LOG)
        .find(filter)
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
            event_data: e.event_data,
            ip_address: e.ip_address,
            user_agent: e.user_agent,
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
    /// Space-separated delegation scopes (empty = token exchange disabled).
    pub delegation_scopes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OAuthClientResponse {
    pub id: String,
    pub client_name: String,
    pub client_type: String,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: String,
    pub delegation_scopes: String,
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
    let delegation_scopes = body.delegation_scopes.as_deref().unwrap_or("");

    // M3: Validate delegation_scopes against known scopes
    if !delegation_scopes.is_empty() {
        let valid_scopes = ["llm:proxy", "proxy:*", "llm:status"];
        for s in delegation_scopes.split_whitespace() {
            if !valid_scopes.contains(&s) {
                return Err(AppError::ValidationError(format!(
                    "Invalid delegation scope '{}'. Must be one of: {}",
                    s,
                    valid_scopes.join(", ")
                )));
            }
        }
    }

    let (client, raw_secret) = oauth_client_service::create_client(
        &state.db,
        &body.name,
        &body.redirect_uris,
        client_type,
        &user_id,
        delegation_scopes,
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
        delegation_scopes: client.delegation_scopes,
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
                delegation_scopes: c.delegation_scopes,
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

    Ok(Json(
        serde_json::json!({ "message": "OAuth client deactivated" }),
    ))
}

// --- Client Consents ---

#[derive(Debug, Serialize)]
pub struct ClientConsentItem {
    pub id: String,
    pub user_id: String,
    pub user_email: Option<String>,
    pub scopes: String,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ClientConsentListResponse {
    pub consents: Vec<ClientConsentItem>,
}

/// GET /api/v1/admin/oauth-clients/:client_id/consents
///
/// List all user consents granted to a specific OAuth client.
pub async fn list_client_consents(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<ClientConsentListResponse>> {
    require_admin(&state, &auth_user).await?;

    let consents = consent_service::list_client_consents(&state.db, &client_id).await?;

    let mut items = Vec::with_capacity(consents.len());
    for c in consents {
        let user_email = state
            .db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &c.user_id })
            .await?
            .map(|u| u.email);

        items.push(ClientConsentItem {
            id: c.id,
            user_id: c.user_id,
            user_email,
            scopes: c.scopes,
            granted_at: c.granted_at.to_rfc3339(),
            expires_at: c.expires_at.map(|t| t.to_rfc3339()),
        });
    }

    Ok(Json(ClientConsentListResponse { consents: items }))
}
