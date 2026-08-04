use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::models::user::{COLLECTION_NAME as USERS, PlatformRole, User};
use crate::mw::auth::AuthUser;
use crate::services::{
    admin_user_service, audit_service, consent_service, oauth_client_service, role_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

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
    pub api_key_id: Option<String>,
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
    pub is_operator: bool,
    /// Resolved platform role: `"admin"`, `"operator"`, or `"user"`.
    pub role: String,
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
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
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
    pub role: String,
    pub is_admin: bool,
    pub is_operator: bool,
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

/// Body for `PATCH /admin/users/{id}/role`. Either `role` or `is_admin`
/// must be set; `role` wins when both are present. `role` accepts
/// `"admin"`, `"operator"`, or `"user"`. `is_admin` is the legacy two-tier
/// shape and is preserved so existing CLI/UI clients keep working.
#[derive(Debug, Deserialize)]
pub struct SetRoleRequest {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
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
    pub role: String,
    pub is_admin: bool,
    pub is_operator: bool,
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

fn normalize_optional_nonempty(input: Option<&str>) -> Option<&str> {
    input.map(str::trim).filter(|value| !value.is_empty())
}

/// Convert a User model into an AdminUserItem response struct.
fn user_to_admin_item(u: User, platform_role: PlatformRole) -> AdminUserItem {
    let role = platform_role.as_str().to_string();
    let (is_admin, is_operator) = platform_role.legacy_flags();
    AdminUserItem {
        id: u.id,
        email: u.email,
        display_name: u.display_name,
        avatar_url: u.avatar_url,
        email_verified: u.email_verified,
        is_active: u.is_active,
        is_admin,
        is_operator,
        role,
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
    _headers: HeaderMap,
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
    if body.role != "admin" && body.role != "operator" && body.role != "user" {
        return Err(AppError::ValidationError(
            "Role must be 'admin', 'operator', or 'user'".to_string(),
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

    let platform_role = role_service::resolve_platform_role(&state.db, &user).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.created",
        Some(serde_json::json!({
            "target_user_id": &user.id,
            "target_email": &user.email,
            "role": platform_role.as_str(),
        })),
    );

    let role = platform_role.as_str().to_string();
    let (is_admin, is_operator) = platform_role.legacy_flags();
    Ok(Json(CreateUserResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        role,
        is_admin,
        is_operator,
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
    require_admin_or_operator(&state, &auth_user, "admin.users.list").await?;

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

    let platform_role_ids = role_service::get_platform_role_ids(&state.db).await?;
    let items: Vec<AdminUserItem> = users
        .into_iter()
        .map(|user| {
            let platform_role =
                role_service::resolve_platform_role_from_ids(&user, &platform_role_ids);
            user_to_admin_item(user, platform_role)
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
    require_admin_or_operator(&state, &auth_user, "admin.users.get").await?;

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let platform_role = role_service::resolve_platform_role(&state.db, &user_model).await?;
    Ok(Json(user_to_admin_item(user_model, platform_role)))
}

/// PUT /api/v1/admin/users/:user_id
///
/// Edit a user's profile fields (admin only).
pub async fn update_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.updated",
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "target_email": &updated.email,
            "changes": {
                "display_name": body.display_name,
                "email": body.email,
                "avatar_url": body.avatar_url,
            }
        })),
    );

    let platform_role = role_service::resolve_platform_role(&state.db, &updated).await?;
    Ok(Json(user_to_admin_item(updated, platform_role)))
}

/// PATCH /api/v1/admin/users/:user_id/role
///
/// Toggle admin role for a user (admin only, cannot change own role).
pub async fn set_user_role(
    State(state): State<AppState>,
    auth_user: AuthUser,
    _headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<SetRoleRequest>,
) -> AppResult<Json<RoleUpdateResponse>> {
    require_admin(&state, &auth_user).await?;

    let admin_id = auth_user.user_id.to_string();

    // Resolve the requested role. `role` wins when both are present so new
    // clients can opt into the three-tier model without the legacy
    // `is_admin` flag silently overriding it.
    let role = match (body.role.as_deref(), body.is_admin) {
        (Some(r), _) => r.to_string(),
        (None, Some(true)) => "admin".to_string(),
        (None, Some(false)) => "user".to_string(),
        (None, None) => {
            return Err(AppError::ValidationError(
                "Provide either 'role' ('admin'|'operator'|'user') or 'is_admin' (bool)"
                    .to_string(),
            ));
        }
    };

    let updated =
        admin_user_service::set_platform_role(&state.db, &admin_id, &user_id, &role).await?;
    let platform_role = role_service::resolve_platform_role(&state.db, &updated).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.role_changed",
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "role": platform_role.as_str(),
        })),
    );

    let role = platform_role.as_str().to_string();
    let (is_admin, is_operator) = platform_role.legacy_flags();

    Ok(Json(RoleUpdateResponse {
        id: user_id,
        role,
        is_admin,
        is_operator,
        message: "User platform role updated".to_string(),
    }))
}

/// PATCH /api/v1/admin/users/:user_id/status
///
/// Toggle active status for a user (admin only, cannot change own status).
/// When disabling, all sessions are revoked.
pub async fn set_user_status(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    _headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<SetStatusRequest>,
) -> AppResult<Json<StatusUpdateResponse>> {
    require_admin(&state, &auth_user).await?;

    let admin_id = auth_user.user_id.to_string();

    admin_user_service::set_user_active(&state.db, &admin_id, &user_id, body.is_active).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.status_changed",
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "is_active": body.is_active,
        })),
    );

    // `is_active=false` is the suspend path; `is_active=true` is unsuspend.
    // There is no dedicated suspend/unsuspend route — this single endpoint
    // serves both, so the emitted variant mirrors the applied bool.
    let event = if body.is_active {
        TelemetryEvent::AdminUserUnsuspended
    } else {
        TelemetryEvent::AdminUserSuspended
    };
    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        event,
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
    _headers: HeaderMap,
    Path(user_id): Path<String>,
) -> AppResult<Json<AdminActionResponse>> {
    require_admin(&state, &auth_user).await?;

    let _token = admin_user_service::force_password_reset(&state.db, &user_id).await?;

    #[cfg(debug_assertions)]
    if let Some(ref t) = _token {
        tracing::debug!(token = %t, user_id = %user_id, "Admin-initiated password reset token (dev only)");
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.password_reset",
        Some(serde_json::json!({ "target_user_id": &user_id })),
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
    _headers: HeaderMap,
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.deleted",
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "target_email": &target_email,
        })),
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
    _headers: HeaderMap,
    Path(user_id): Path<String>,
) -> AppResult<Json<VerifyEmailResponse>> {
    require_admin(&state, &auth_user).await?;

    admin_user_service::verify_email(&state.db, &user_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.email_verified",
        Some(serde_json::json!({ "target_user_id": &user_id })),
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
    require_admin_or_operator(&state, &auth_user, "admin.users.sessions.list").await?;

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
    _headers: HeaderMap,
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin.user.sessions_revoked",
        Some(serde_json::json!({
            "target_user_id": &user_id,
            "revoked_count": revoked_count,
        })),
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
    tele: TelemetryContext,
    Query(query): Query<AuditLogQuery>,
) -> AppResult<Json<AuditLogListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.audit_log.list").await?;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let mut filter = doc! {};
    if let Some(ref uid) = query.user_id {
        filter.insert("user_id", uid);
    }
    if let Some(ref api_key_id) = query.api_key_id {
        filter.insert("api_key_id", api_key_id);
    }

    // Summarize filters as an opaque marker list rather than the raw IDs
    // (which are PII-adjacent). `None` when no filter was applied.
    let filter_marker: Option<String> = {
        let mut parts: Vec<&str> = Vec::new();
        if query.user_id.is_some() {
            parts.push("user_id");
        }
        if query.api_key_id.is_some() {
            parts.push("api_key_id");
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(","))
        }
    };

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
            api_key_id: e.api_key_id,
            api_key_name: e.api_key_name,
            event_type: e.event_type,
            event_data: e.event_data,
            ip_address: e.ip_address,
            user_agent: e.user_agent,
            created_at: e.created_at.to_rfc3339(),
        })
        .collect();

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminAuditLogViewed {
            filter: filter_marker,
        },
    );

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
    pub broker_capability_enabled: Option<bool>,
    pub revocation_webhook_url: Option<String>,
    pub revocation_webhook_secret: Option<String>,
    /// OIDC scopes this client is allowed to request.
    /// Defaults to `["openid", "profile", "email"]` when omitted; `[]` canonicalizes to `["openid"]`.
    pub allowed_scopes: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct OAuthClientResponse {
    pub id: String,
    pub client_name: String,
    pub client_type: String,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: String,
    pub delegation_scopes: String,
    pub broker_capability_enabled: bool,
    pub revocation_webhook_url: Option<String>,
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
    tele: TelemetryContext,
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

    let allowed_scopes = body
        .allowed_scopes
        .as_deref()
        .map(oauth_client_service::validate_allowed_scopes_list)
        .transpose()?
        .unwrap_or_else(|| oauth_client_service::DEFAULT_ALLOWED_SCOPES.to_string());
    let revocation_webhook_url =
        normalize_optional_nonempty(body.revocation_webhook_url.as_deref());
    let revocation_webhook_secret_encrypted =
        match normalize_optional_nonempty(body.revocation_webhook_secret.as_deref()) {
            Some(secret) => Some(state.encryption_keys.encrypt(secret.as_bytes()).await?),
            None => None,
        };

    let (client, raw_secret) = oauth_client_service::create_client(
        &state.db,
        &body.name,
        &body.redirect_uris,
        client_type,
        &user_id,
        delegation_scopes,
        &allowed_scopes,
        body.broker_capability_enabled.unwrap_or(false),
        revocation_webhook_url,
        revocation_webhook_secret_encrypted,
    )
    .await?;

    tracing::info!(
        client_id = %client.id,
        client_name = %client.client_name,
        created_by = %user_id,
        "OAuth client created"
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminOauthClientRegistered,
    );

    Ok(Json(OAuthClientResponse {
        id: client.id.clone(),
        client_name: client.client_name,
        client_type: client.client_type,
        redirect_uris: client.redirect_uris,
        allowed_scopes: client.allowed_scopes,
        delegation_scopes: client.delegation_scopes,
        broker_capability_enabled: client.broker_capability_enabled,
        revocation_webhook_url: client.revocation_webhook_url,
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
    require_admin_or_operator(&state, &auth_user, "admin.oauth_clients.list").await?;

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
                broker_capability_enabled: c.broker_capability_enabled,
                revocation_webhook_url: c.revocation_webhook_url,
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
    require_admin_or_operator(&state, &auth_user, "admin.oauth_clients.consents.list").await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{PlatformRole, User, UserType};
    use chrono::Utc;

    fn make_user(id: &str) -> User {
        let now = Utc::now();
        User {
            id: id.to_string(),
            email: format!("{id}@example.com"),
            password_hash: Some("$argon2id$hash".to_string()),
            display_name: Some("Test User".to_string()),
            slug: None,
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            is_operator: false,
            role_ids: vec![],
            group_ids: vec![],
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            user_type: UserType::Person,
            primary_org_id: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            profile_config: Default::default(),
        }
    }

    // --- normalize_optional_nonempty tests ---

    #[test]
    fn normalize_optional_nonempty_none_returns_none() {
        assert_eq!(normalize_optional_nonempty(None), None);
    }

    #[test]
    fn normalize_optional_nonempty_empty_string_returns_none() {
        assert_eq!(normalize_optional_nonempty(Some("")), None);
    }

    #[test]
    fn normalize_optional_nonempty_whitespace_only_returns_none() {
        assert_eq!(normalize_optional_nonempty(Some("   ")), None);
        assert_eq!(normalize_optional_nonempty(Some("\t\n")), None);
    }

    #[test]
    fn normalize_optional_nonempty_trims_whitespace() {
        assert_eq!(
            normalize_optional_nonempty(Some("  hello  ")),
            Some("hello")
        );
    }

    #[test]
    fn normalize_optional_nonempty_preserves_normal_string() {
        assert_eq!(normalize_optional_nonempty(Some("hello")), Some("hello"));
    }

    // --- user_to_admin_item tests ---

    #[test]
    fn user_to_admin_item_admin_role() {
        let user = make_user("user-1");
        let item = user_to_admin_item(user.clone(), PlatformRole::Admin);

        assert_eq!(item.id, "user-1");
        assert_eq!(item.email, "user-1@example.com");
        assert_eq!(item.display_name, Some("Test User".to_string()));
        assert_eq!(
            item.avatar_url,
            Some("https://example.com/avatar.png".to_string())
        );
        assert!(item.email_verified);
        assert!(item.is_active);
        assert!(item.is_admin);
        assert!(!item.is_operator);
        assert_eq!(item.role, "admin");
        assert!(!item.mfa_enabled);
        assert!(item.last_login_at.is_none());
    }

    #[test]
    fn user_to_admin_item_operator_role() {
        let user = make_user("user-2");
        let item = user_to_admin_item(user, PlatformRole::Operator);

        assert!(!item.is_admin);
        assert!(item.is_operator);
        assert_eq!(item.role, "operator");
    }

    #[test]
    fn user_to_admin_item_user_role() {
        let user = make_user("user-3");
        let item = user_to_admin_item(user, PlatformRole::User);

        assert!(!item.is_admin);
        assert!(!item.is_operator);
        assert_eq!(item.role, "user");
    }

    #[test]
    fn user_to_admin_item_with_last_login() {
        let mut user = make_user("user-4");
        user.last_login_at = Some(Utc::now());
        let item = user_to_admin_item(user, PlatformRole::User);

        assert!(item.last_login_at.is_some());
    }

    #[test]
    fn user_to_admin_item_no_display_name_or_avatar() {
        let mut user = make_user("user-5");
        user.display_name = None;
        user.avatar_url = None;
        let item = user_to_admin_item(user, PlatformRole::User);

        assert!(item.display_name.is_none());
        assert!(item.avatar_url.is_none());
    }

    #[test]
    fn user_to_admin_item_mfa_enabled() {
        let mut user = make_user("user-6");
        user.mfa_enabled = true;
        let item = user_to_admin_item(user, PlatformRole::User);

        assert!(item.mfa_enabled);
    }

    #[test]
    fn user_to_admin_item_inactive_user() {
        let mut user = make_user("user-7");
        user.is_active = false;
        let item = user_to_admin_item(user, PlatformRole::Admin);

        assert!(!item.is_active);
        assert!(item.is_admin);
    }

    #[test]
    fn user_to_admin_item_created_at_is_rfc3339() {
        let user = make_user("user-8");
        let item = user_to_admin_item(user, PlatformRole::User);
        // Verify it parses as a valid RFC 3339 timestamp
        chrono::DateTime::parse_from_rfc3339(&item.created_at)
            .expect("created_at should be valid RFC 3339");
    }

    // --- Serde round-trip tests for request/response structs ---

    #[test]
    fn set_role_request_deserializes_with_role_only() {
        let json = r#"{"role": "operator"}"#;
        let req: SetRoleRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.role, Some("operator".to_string()));
        assert_eq!(req.is_admin, None);
    }

    #[test]
    fn set_role_request_deserializes_with_is_admin_only() {
        let json = r#"{"is_admin": true}"#;
        let req: SetRoleRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.role, None);
        assert_eq!(req.is_admin, Some(true));
    }

    #[test]
    fn set_role_request_deserializes_empty_body() {
        let json = r#"{}"#;
        let req: SetRoleRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.role, None);
        assert_eq!(req.is_admin, None);
    }

    #[test]
    fn admin_user_item_serializes_all_fields() {
        let item = AdminUserItem {
            id: "id-1".to_string(),
            email: "test@example.com".to_string(),
            display_name: Some("Display".to_string()),
            avatar_url: None,
            email_verified: true,
            is_active: true,
            is_admin: false,
            is_operator: true,
            role: "operator".to_string(),
            mfa_enabled: false,
            created_at: "2024-01-01T00:00:00+00:00".to_string(),
            last_login_at: None,
        };
        let json = serde_json::to_value(&item).expect("serialize");
        assert_eq!(json["id"], "id-1");
        assert_eq!(json["role"], "operator");
        assert!(json["is_operator"].as_bool().unwrap());
        assert!(!json["is_admin"].as_bool().unwrap());
        assert!(json["last_login_at"].is_null());
    }

    #[test]
    fn admin_action_response_serializes() {
        let resp = AdminActionResponse {
            message: "done".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["message"], "done");
    }
}

#[cfg(test)]
mod operator_route_tests {
    //! End-to-end tests proving the operator role's read/write split holds at
    //! the actual handler entrypoint, not just inside the helper. These are
    //! the tests the reviewer asked for: an operator must get 403 from a
    //! representative write handler (`set_user_role`) and 200 from a
    //! representative read handler (`list_users`).
    use super::*;
    use crate::models::user::UserType;
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use uuid::Uuid;

    async fn insert_user(db: &mongodb::Database, is_admin: bool, is_operator: bool) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed platform roles");
        let platform_role_ids = role_service::get_platform_role_ids(db)
            .await
            .expect("platform role ids");
        let id = Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        if is_admin {
            user.role_ids.push(platform_role_ids.admin);
        } else if is_operator {
            user.role_ids.push(platform_role_ids.operator);
        }
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert test user");
        id
    }

    #[tokio::test]
    async fn operator_can_list_users() {
        let Some(db) = connect_test_database("admin_route_operator_read").await else {
            eprintln!("skipping operator_can_list_users: no local MongoDB available");
            return;
        };
        let operator_id = insert_user(&db, false, true).await;
        let state = test_app_state(db);

        let result = list_users(
            State(state),
            test_auth_user(&operator_id),
            Query(UserListQuery {
                page: None,
                per_page: None,
                search: None,
            }),
        )
        .await
        .expect("operator should be allowed to GET /admin/users");
        assert!(
            result.0.users.iter().any(|u| u.id == operator_id),
            "operator should see at least their own row in the list"
        );
    }

    #[tokio::test]
    async fn operator_cannot_change_user_role() {
        let Some(db) = connect_test_database("admin_route_operator_write").await else {
            eprintln!("skipping operator_cannot_change_user_role: no local MongoDB available");
            return;
        };
        let operator_id = insert_user(&db, false, true).await;
        let target_id = insert_user(&db, false, false).await;
        let state = test_app_state(db);

        let err = set_user_role(
            State(state),
            test_auth_user(&operator_id),
            HeaderMap::new(),
            Path(target_id.clone()),
            Json(SetRoleRequest {
                role: Some("admin".to_string()),
                is_admin: None,
            }),
        )
        .await
        .expect_err("operator must NOT be allowed to PATCH /admin/users/{id}/role");
        assert!(
            matches!(err, AppError::Forbidden(_)),
            "operator role change should yield 403 Forbidden, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn operator_cannot_create_user() {
        let Some(db) = connect_test_database("admin_route_operator_create").await else {
            eprintln!("skipping operator_cannot_create_user: no local MongoDB available");
            return;
        };
        let operator_id = insert_user(&db, false, true).await;
        let state = test_app_state(db);

        let err = create_user(
            State(state),
            test_auth_user(&operator_id),
            HeaderMap::new(),
            Json(CreateUserRequest {
                email: "newbie@example.com".to_string(),
                password: "password123".to_string(),
                display_name: None,
                role: "user".to_string(),
            }),
        )
        .await
        .expect_err("operator must NOT be allowed to POST /admin/users");
        assert!(
            matches!(err, AppError::Forbidden(_)),
            "operator create-user should yield 403 Forbidden, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn set_role_operator_assigns_operator_system_role() {
        let Some(db) = connect_test_database("admin_route_set_operator").await else {
            eprintln!("skipping set_role_operator: no local MongoDB available");
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let target_id = insert_user(&db, false, false).await;
        let state = test_app_state(db.clone());

        let response = set_user_role(
            State(state),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(target_id.clone()),
            Json(SetRoleRequest {
                role: Some("operator".to_string()),
                is_admin: None,
            }),
        )
        .await
        .expect("admin can assign operator role");

        assert_eq!(response.0.role, "operator");
        assert!(!response.0.is_admin);
        assert!(response.0.is_operator);

        let platform_role_ids = role_service::get_platform_role_ids(&db)
            .await
            .expect("platform role ids");
        let target = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &target_id })
            .await
            .expect("query target")
            .expect("target exists");
        assert!(target.role_ids.contains(&platform_role_ids.operator));
        assert!(!target.role_ids.contains(&platform_role_ids.admin));
    }

    #[tokio::test]
    async fn set_role_legacy_is_admin_true_assigns_admin_system_role() {
        let Some(db) = connect_test_database("admin_route_set_legacy_admin").await else {
            eprintln!("skipping set_role_legacy_admin: no local MongoDB available");
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let target_id = insert_user(&db, false, false).await;
        let state = test_app_state(db.clone());

        let response = set_user_role(
            State(state),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(target_id.clone()),
            Json(SetRoleRequest {
                role: None,
                is_admin: Some(true),
            }),
        )
        .await
        .expect("legacy is_admin=true still assigns admin");

        assert_eq!(response.0.role, "admin");
        assert!(response.0.is_admin);
        assert!(!response.0.is_operator);

        let platform_role_ids = role_service::get_platform_role_ids(&db)
            .await
            .expect("platform role ids");
        let target = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &target_id })
            .await
            .expect("query target")
            .expect("target exists");
        assert!(target.role_ids.contains(&platform_role_ids.admin));
        assert!(!target.role_ids.contains(&platform_role_ids.operator));
    }

    #[tokio::test]
    async fn set_role_user_revokes_admin_and_operator_roles() {
        let Some(db) = connect_test_database("admin_route_set_user").await else {
            eprintln!("skipping set_role_user: no local MongoDB available");
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let target_id = insert_user(&db, false, false).await;
        let platform_role_ids = role_service::get_platform_role_ids(&db)
            .await
            .expect("platform role ids");
        db.collection::<User>(USERS)
            .update_one(
                doc! { "_id": &target_id },
                doc! { "$addToSet": { "role_ids": { "$each": [
                    &platform_role_ids.admin,
                    &platform_role_ids.operator,
                ]}}},
            )
            .await
            .expect("grant both platform roles");
        let state = test_app_state(db.clone());

        let response = set_user_role(
            State(state),
            test_auth_user(&admin_id),
            HeaderMap::new(),
            Path(target_id.clone()),
            Json(SetRoleRequest {
                role: Some("user".to_string()),
                is_admin: None,
            }),
        )
        .await
        .expect("admin can demote to user");

        assert_eq!(response.0.role, "user");
        assert!(!response.0.is_admin);
        assert!(!response.0.is_operator);

        let target = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &target_id })
            .await
            .expect("query target")
            .expect("target exists");
        assert!(!target.role_ids.contains(&platform_role_ids.admin));
        assert!(!target.role_ids.contains(&platform_role_ids.operator));
    }
}
