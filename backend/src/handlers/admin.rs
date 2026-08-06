use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use std::collections::{HashMap, HashSet};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::models::audit_log::AuditLog;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user::{COLLECTION_NAME as USERS, PlatformRole, User};
use crate::mw::auth::AuthUser;
use crate::services::billing::ledger as billing_ledger;
use crate::services::{
    admin_audit_service, admin_user_service, audit_chain_service, audit_service,
    chain_verify_service, consent_service, oauth_client_service, platform_settings_service,
    role_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
    /// Optional account-type filter: `"person"` or `"org"`. Unset lists all
    /// accounts (legacy behavior).
    pub user_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AuditLogQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    /// Legacy exact-match filters, predating the admin table controls.
    pub user_id: Option<String>,
    pub api_key_id: Option<String>,
    pub search: Option<String>,
    pub search_filters: Option<String>,
    pub custom_filters: Option<String>,
    pub event_type: Option<String>,
    pub status: Option<String>,
    pub actor: Option<String>,
    pub created_dates: Option<String>,
    pub created_from: Option<String>,
    pub created_to: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuditLogVerifyQuery {
    pub from_seq: Option<i64>,
    pub to_seq: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AdminUserItem {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    /// Org accounts only; `None` for person accounts.
    pub slug: Option<String>,
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
    pub seq: Option<i64>,
    pub user_id: Option<String>,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub event_type: String,
    pub event_data: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
    /// Human-readable service display name resolved from the referenced
    /// DownstreamService at query time. `None` when the event has no
    /// service context or the referenced service no longer exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    /// Canonical service slug (e.g. `"openai"`) resolved at query time.
    /// `None` when the event has no service context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_slug: Option<String>,
    /// Display name of the acting user resolved at query time. `None` when
    /// the event has no user, the user was deleted, or no name is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
    /// Email of the acting user resolved at query time. `None` when the
    /// event has no user or the user was deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditLogListResponse {
    pub entries: Vec<AuditLogItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub filter_options: AuditLogFilterOptions,
}

/// Describes the audit table's controls to the client, so the UI never hardcodes
/// a domain the server would reject.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AuditLogFilterOptions {
    pub sorts: Vec<&'static str>,
    pub search_fields: Vec<AuditLogSearchField>,
    pub fields: Vec<AuditLogFilterField>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AuditLogSearchField {
    pub key: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditLogFilterValueType {
    Enum,
    Date,
    /// Free-text only: the column is too high-cardinality to enumerate as
    /// options (UUIDs, IPs, User-Agent strings).
    Text,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditLogFilterOperator {
    Is,
    Between,
    Contains,
}

/// Owned rather than `&'static str` because the event-type options are
/// discovered from the data at request time.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AuditLogFilterOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AuditLogFilterField {
    pub key: &'static str,
    pub label: &'static str,
    pub value_type: AuditLogFilterValueType,
    pub operator: AuditLogFilterOperator,
    pub multiple: bool,
    pub options: Vec<AuditLogFilterOption>,
    /// Whether the filter also accepts free text, matched as a case-insensitive
    /// `contains` against the field's stored column and OR'd with its options.
    pub supports_custom_text: bool,
}

#[derive(Debug, Serialize)]
pub struct AuditLogVerifyResponse {
    pub status: audit_chain_service::AuditChainStatus,
    pub checked_count: u64,
    pub pre_chain_count: u64,
    pub head_seq: Option<i64>,
    pub head_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_info: Option<audit_chain_service::AuditChainBreak>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_from_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct BillingLedgerVerifyResponse {
    pub status: billing_ledger::BillingLedgerStatus,
    pub checked_count: u64,
    pub head_seq: Option<i64>,
    pub head_hash: Option<String>,
    /// Ledger seq recorded by the newest audit-chain head anchor.
    pub anchor_seq: Option<i64>,
    /// False when the newest anchor event fails audit-chain validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_info: Option<billing_ledger::BillingLedgerBreak>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_from_seq: Option<i64>,
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
        slug: u.slug,
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

    let mut filter = match query.user_type.as_deref() {
        None => doc! {},
        Some("org") => doc! { "user_type": "org" },
        // Legacy person rows predate the field; match its absence too.
        Some("person") => doc! { "user_type": { "$ne": "org" } },
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "invalid user_type filter '{other}'; expected person or org"
            )));
        }
    };
    if let Some(s) = query.search.as_deref().filter(|s| !s.is_empty()) {
        let escaped = regex::escape(s);
        let pattern = doc! { "$regex": &escaped, "$options": "i" };
        if query.user_type.as_deref() == Some("org") {
            // Orgs are found by name/slug; their emails are auto-generated.
            filter.insert(
                "$or",
                vec![
                    doc! { "email": pattern.clone() },
                    doc! { "display_name": pattern.clone() },
                    doc! { "slug": pattern },
                ],
            );
        } else {
            filter.insert("email", pattern);
        }
    }

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

/// Batch-resolves the DownstreamService rows referenced by any of the audit
/// entries in this page. Populated once per request; the result is a two-way
/// lookup keyed by both `_id` (UUID) and `slug`, since audit event_data may
/// carry either shape under `service_id` / `service_slug`.
#[derive(Default)]
struct ServiceLookup {
    by_id: HashMap<String, ResolvedService>,
    by_slug: HashMap<String, ResolvedService>,
}

#[derive(Clone)]
struct ResolvedService {
    name: String,
    slug: String,
}

/// Collect unique service references from a page of audit entries and
/// batch-load matching DownstreamService rows in at most two MongoDB round-
/// trips (one by `_id`, one by `slug`). Missing services degrade to `None`
/// downstream rather than surfacing an error -- referenced services can be
/// legitimately deleted after the audit event was recorded.
async fn resolve_service_lookup(
    db: &mongodb::Database,
    entries: &[AuditLog],
) -> AppResult<ServiceLookup> {
    let mut ids: HashSet<String> = HashSet::new();
    let mut slugs: HashSet<String> = HashSet::new();
    for entry in entries {
        collect_service_refs(entry.event_data.as_ref(), &mut ids, &mut slugs);
    }

    let mut by_id: HashMap<String, ResolvedService> = HashMap::new();
    let mut by_slug: HashMap<String, ResolvedService> = HashMap::new();

    if !ids.is_empty() {
        let id_vec: Vec<String> = ids.into_iter().collect();
        let services: Vec<DownstreamService> = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": &id_vec } })
            .await?
            .try_collect()
            .await?;
        for s in services {
            let resolved = ResolvedService {
                name: s.name.clone(),
                slug: s.slug.clone(),
            };
            by_slug.insert(s.slug.clone(), resolved.clone());
            by_id.insert(s.id, resolved);
        }
    }
    if !slugs.is_empty() {
        let slug_vec: Vec<String> = slugs.into_iter().collect();
        let services: Vec<DownstreamService> = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "slug": { "$in": &slug_vec } })
            .await?
            .try_collect()
            .await?;
        for s in services {
            let resolved = ResolvedService {
                name: s.name.clone(),
                slug: s.slug.clone(),
            };
            by_id.insert(s.id.clone(), resolved.clone());
            by_slug.insert(s.slug, resolved);
        }
    }

    Ok(ServiceLookup { by_id, by_slug })
}

fn collect_service_refs(
    event_data: Option<&serde_json::Value>,
    ids: &mut HashSet<String>,
    slugs: &mut HashSet<String>,
) {
    let Some(data) = event_data else { return };
    if let Some(v) = data.get("service_slug").and_then(|v| v.as_str())
        && !v.is_empty()
    {
        slugs.insert(v.to_string());
    }
    if let Some(v) = data.get("service_id").and_then(|v| v.as_str())
        && !v.is_empty()
    {
        // `service_id` is a UUID for /proxy/{uuid}/... routes and a slug
        // for /proxy/s/{slug}/... routes -- probe both indexes.
        if uuid::Uuid::parse_str(v).is_ok() {
            ids.insert(v.to_string());
        } else {
            slugs.insert(v.to_string());
        }
    }
}

fn resolve_entry_service(
    event_data: Option<&serde_json::Value>,
    lookup: &ServiceLookup,
) -> (Option<String>, Option<String>) {
    let Some(data) = event_data else {
        return (None, None);
    };
    let candidate_slug = data.get("service_slug").and_then(|v| v.as_str());
    let candidate_id_or_slug = data.get("service_id").and_then(|v| v.as_str());

    if let Some(slug) = candidate_slug
        && let Some(hit) = lookup.by_slug.get(slug)
    {
        return (Some(hit.name.clone()), Some(hit.slug.clone()));
    }
    if let Some(value) = candidate_id_or_slug {
        if let Some(hit) = lookup.by_id.get(value) {
            return (Some(hit.name.clone()), Some(hit.slug.clone()));
        }
        if let Some(hit) = lookup.by_slug.get(value) {
            return (Some(hit.name.clone()), Some(hit.slug.clone()));
        }
    }

    // Reference exists in event_data but no catalog row matched (deleted
    // service, or a slug/UUID that never existed): surface the raw slug so
    // the UI still has something to render.
    let raw_slug = candidate_slug
        .or_else(|| candidate_id_or_slug.filter(|v| uuid::Uuid::parse_str(v).is_err()))
        .map(|s| s.to_string());
    (None, raw_slug)
}

/// Batch-resolves display name + email for the acting users referenced by a
/// page of audit entries (one MongoDB round-trip). Deleted users simply drop
/// out of the map and render as UUID-only downstream.
async fn resolve_user_lookup(
    db: &mongodb::Database,
    entries: &[AuditLog],
) -> AppResult<HashMap<String, (Option<String>, String)>> {
    let ids: Vec<String> = entries
        .iter()
        .filter_map(|e| e.user_id.clone())
        .collect::<HashSet<String>>()
        .into_iter()
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let users: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": &ids } })
        .await?
        .try_collect()
        .await?;
    Ok(users
        .into_iter()
        .map(|u| (u.id, (u.display_name, u.email)))
        .collect())
}

/// GET /api/v1/admin/audit-log
///
/// Query the audit log (admin only). Supports server-side pagination, sorting,
/// scoped search, and filtering; the legacy `user_id` / `api_key_id` exact-match
/// params still work alongside the table controls.
pub async fn list_audit_log(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Query(query): Query<AuditLogQuery>,
) -> AppResult<Json<AuditLogListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.audit_log.list").await?;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 100);
    let sort = query.sort.as_deref().unwrap_or("-created_at");

    // Summarize the applied controls as an opaque marker list rather than the raw
    // values (which are PII-adjacent). `None` when nothing was applied.
    let filter_marker = audit_log_filter_marker(&query);

    let (entries, total) = admin_audit_service::list_entries(
        &state.db,
        admin_audit_service::AdminAuditLogListParams {
            page,
            per_page,
            search: query.search.as_deref(),
            search_filters: query.search_filters.as_deref(),
            custom_filters: query.custom_filters.as_deref(),
            event_type: query.event_type.as_deref(),
            status: query.status.as_deref(),
            actor: query.actor.as_deref(),
            user_id: query.user_id.as_deref(),
            api_key_id: query.api_key_id.as_deref(),
            created_dates: query.created_dates.as_deref(),
            created_from: query.created_from.as_deref(),
            created_to: query.created_to.as_deref(),
            sort,
        },
    )
    .await?;

    // Enrichment is best-effort display data: a lookup failure downgrades the
    // page to unenriched entries (fields are Option and simply stay absent)
    // rather than failing a listing whose primary query already succeeded.
    let (service_lookup, user_lookup) = match tokio::try_join!(
        resolve_service_lookup(&state.db, &entries),
        resolve_user_lookup(&state.db, &entries),
    ) {
        Ok(lookups) => lookups,
        Err(error) => {
            tracing::warn!(%error, "audit-log enrichment lookups failed; returning unenriched entries");
            (ServiceLookup::default(), HashMap::new())
        }
    };

    let items: Vec<AuditLogItem> = entries
        .into_iter()
        .map(|e| {
            let (service_name, service_slug) =
                resolve_entry_service(e.event_data.as_ref(), &service_lookup);
            let (user_display_name, user_email) = e
                .user_id
                .as_deref()
                .and_then(|uid| user_lookup.get(uid).cloned())
                .map(|(display_name, email)| (display_name, Some(email)))
                .unwrap_or((None, None));
            AuditLogItem {
                id: e.id,
                seq: e.seq,
                user_id: e.user_id,
                api_key_id: e.api_key_id,
                api_key_name: e.api_key_name,
                event_type: e.event_type,
                event_data: e.event_data,
                ip_address: e.ip_address,
                user_agent: e.user_agent,
                created_at: e.created_at.to_rfc3339(),
                service_name,
                service_slug,
                user_display_name,
                user_email,
            }
        })
        .collect();

    let event_types = admin_audit_service::distinct_event_types(&state.db).await?;

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
        filter_options: audit_log_filter_options(event_types),
    }))
}

/// Names the controls that were applied, never their values.
fn audit_log_filter_marker(query: &AuditLogQuery) -> Option<String> {
    let applied = [
        ("user_id", query.user_id.is_some()),
        ("api_key_id", query.api_key_id.is_some()),
        ("search", query.search.is_some()),
        ("search_filters", query.search_filters.is_some()),
        ("custom_filters", query.custom_filters.is_some()),
        ("event_type", query.event_type.is_some()),
        ("status", query.status.is_some()),
        ("actor", query.actor.is_some()),
        (
            "created_at",
            query.created_dates.is_some()
                || query.created_from.is_some()
                || query.created_to.is_some(),
        ),
    ];

    let parts: Vec<&str> = applied
        .iter()
        .filter_map(|(name, applied)| applied.then_some(*name))
        .collect();
    (!parts.is_empty()).then(|| parts.join(","))
}

fn audit_log_filter_option(value: &str, label: &str) -> AuditLogFilterOption {
    AuditLogFilterOption {
        value: value.to_string(),
        label: label.to_string(),
    }
}

/// Single source of truth: a filter offers a custom-text box exactly when the
/// service can map it to a stored column to run the `contains` against.
fn audit_log_filter_takes_custom_text(key: &str) -> bool {
    admin_audit_service::admin_custom_text_field(key).is_some()
}

fn audit_log_status_label(bucket: &str) -> &'static str {
    match bucket {
        "2xx" => "2xx Success",
        "3xx" => "3xx Redirect",
        "4xx" => "4xx Client error",
        "5xx" => "5xx Server error",
        _ => "No status",
    }
}

fn audit_log_actor_label(actor: &str) -> &'static str {
    match actor {
        "user" => "User session",
        "agent" => "Agent API key",
        _ => "Anonymous",
    }
}

fn audit_log_filter_options(event_types: Vec<String>) -> AuditLogFilterOptions {
    AuditLogFilterOptions {
        sorts: admin_audit_service::ADMIN_SORT_OPTIONS.to_vec(),
        search_fields: admin_audit_service::ADMIN_SEARCH_FIELDS
            .iter()
            .map(|(key, label)| AuditLogSearchField { key, label })
            .collect(),
        fields: vec![
            AuditLogFilterField {
                key: "event_type",
                label: "Event type",
                value_type: AuditLogFilterValueType::Enum,
                operator: AuditLogFilterOperator::Is,
                multiple: true,
                options: event_types
                    .iter()
                    .map(|event_type| audit_log_filter_option(event_type, event_type))
                    .collect(),
                supports_custom_text: audit_log_filter_takes_custom_text("event_type"),
            },
            AuditLogFilterField {
                key: "status",
                label: "Status",
                value_type: AuditLogFilterValueType::Enum,
                operator: AuditLogFilterOperator::Is,
                multiple: true,
                options: admin_audit_service::ADMIN_STATUS_FILTERS
                    .iter()
                    .map(|bucket| audit_log_filter_option(bucket, audit_log_status_label(bucket)))
                    .collect(),
                supports_custom_text: audit_log_filter_takes_custom_text("status"),
            },
            AuditLogFilterField {
                key: "actor",
                label: "Actor",
                value_type: AuditLogFilterValueType::Enum,
                operator: AuditLogFilterOperator::Is,
                multiple: true,
                options: admin_audit_service::ADMIN_ACTOR_FILTERS
                    .iter()
                    .map(|actor| audit_log_filter_option(actor, audit_log_actor_label(actor)))
                    .collect(),
                supports_custom_text: audit_log_filter_takes_custom_text("actor"),
            },
            AuditLogFilterField {
                key: "created_at",
                label: "Created",
                value_type: AuditLogFilterValueType::Date,
                operator: AuditLogFilterOperator::Between,
                multiple: true,
                options: vec![],
                supports_custom_text: audit_log_filter_takes_custom_text("created_at"),
            },
            audit_log_text_filter_field("api_key_name", "Agent"),
            audit_log_text_filter_field("user_id", "User ID"),
            audit_log_text_filter_field("api_key_id", "API Key ID"),
            audit_log_text_filter_field("ip_address", "IP address"),
            audit_log_text_filter_field("user_agent", "User agent"),
        ],
    }
}

/// A column filtered only by free text, because its values are unbounded
/// (UUIDs, IPs, User-Agent strings) and cannot be offered as checkboxes.
fn audit_log_text_filter_field(key: &'static str, label: &'static str) -> AuditLogFilterField {
    debug_assert!(
        audit_log_filter_takes_custom_text(key),
        "a text filter must map to a stored column the server can match against",
    );
    AuditLogFilterField {
        key,
        label,
        value_type: AuditLogFilterValueType::Text,
        operator: AuditLogFilterOperator::Contains,
        multiple: false,
        options: vec![],
        supports_custom_text: true,
    }
}

/// GET /api/v1/admin/audit-log/verify
///
/// Verify tamper-evident audit-log hash-chain integrity over a bounded range.
pub async fn verify_audit_log(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<AuditLogVerifyQuery>,
) -> AppResult<Json<AuditLogVerifyResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.audit_log.verify").await?;

    let report = audit_chain_service::verify_chain(
        &state.db,
        state.audit_chain_hmac_key.as_slice(),
        query.from_seq,
        query.to_seq,
        query.limit,
    )
    .await?;

    Ok(Json(AuditLogVerifyResponse {
        status: report.status,
        checked_count: report.checked_count,
        pre_chain_count: report.pre_chain_count,
        head_seq: report.head_seq,
        head_hash: report.head_hash,
        break_info: report.break_info,
        next_from_seq: report.next_from_seq,
    }))
}

/// GET /api/v1/admin/billing-ledger/verify
///
/// Verify tamper-evident billing-ledger hash-chain integrity over a
/// bounded range. Same shape and semantics as the audit-log verify.
///
/// The tail-truncation check validates the newest anchor event's own
/// hash, but the anchor's linkage into an unbroken audit chain is the
/// audit verifier's job -- run `/admin/audit-log/verify` alongside this
/// endpoint for the full guarantee.
pub async fn verify_billing_ledger(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<AuditLogVerifyQuery>,
) -> AppResult<Json<BillingLedgerVerifyResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.billing_ledger.verify").await?;

    let report = billing_ledger::verify_chain(
        &state.db,
        state.billing_ledger_hmac_key.as_slice(),
        query.from_seq,
        query.to_seq,
        query.limit,
    )
    .await?;

    // The chain walk cannot see tail truncation (a shortened chain stays
    // valid), so always cross-check the head against the newest anchor
    // recorded in the audit chain.
    let anchor =
        billing_ledger::check_head_anchor(&state.db, state.audit_chain_hmac_key.as_slice()).await?;
    let (status, break_info) = match (report.status, report.break_info, anchor.break_info) {
        (billing_ledger::BillingLedgerStatus::Ok, None, Some(anchor_break)) => (
            billing_ledger::BillingLedgerStatus::Broken,
            Some(anchor_break),
        ),
        (status, break_info, _) => (status, break_info),
    };

    Ok(Json(BillingLedgerVerifyResponse {
        status,
        checked_count: report.checked_count,
        head_seq: report.head_seq,
        head_hash: report.head_hash,
        anchor_seq: anchor.anchor_seq,
        anchor_valid: anchor.anchor_valid,
        break_info,
        next_from_seq: report.next_from_seq,
    }))
}

#[derive(Debug, Serialize)]
pub struct ChainVerifyStatusItem {
    pub chain: String,
    pub outcome: crate::models::chain_verify_status::ChainVerifyOutcome,
    pub cursor_seq: i64,
    pub head_seq: Option<i64>,
    pub checked_entries: i64,
    pub last_full_pass_at: Option<chrono::DateTime<chrono::Utc>>,
    pub break_seq: Option<i64>,
    pub break_kind: Option<String>,
    pub break_detail: Option<String>,
    pub anchor_seq: Option<i64>,
    pub anchor_valid: Option<bool>,
    pub pre_chain_count: Option<i64>,
    pub last_run_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct ChainVerificationResponse {
    pub chains: Vec<ChainVerifyStatusItem>,
}

fn chain_status_item(
    status: crate::models::chain_verify_status::ChainVerifyStatus,
) -> ChainVerifyStatusItem {
    ChainVerifyStatusItem {
        chain: status.id,
        outcome: status.outcome,
        cursor_seq: status.cursor_seq,
        head_seq: status.head_seq,
        checked_entries: status.checked_entries,
        last_full_pass_at: status.last_full_pass_at,
        break_seq: status.break_seq,
        break_kind: status.break_kind,
        break_detail: status.break_detail,
        anchor_seq: status.anchor_seq,
        anchor_valid: status.anchor_valid,
        pre_chain_count: status.pre_chain_count,
        last_run_at: status.last_run_at,
    }
}

/// GET /api/v1/admin/chain-verification
///
/// Latest automatic verification state for both hash chains, as written
/// by the background sweep. Chains the sweep has never covered yet are
/// simply absent from the list.
pub async fn get_chain_verification(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ChainVerificationResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.chain_verification.read").await?;

    let chains = [
        crate::models::chain_verify_status::CHAIN_AUDIT_LOG,
        crate::models::chain_verify_status::CHAIN_BILLING_LEDGER,
    ];
    let mut items = Vec::new();
    for chain in chains {
        if let Some(status) = chain_verify_service::load_status(&state.db, chain).await? {
            items.push(chain_status_item(status));
        }
    }
    Ok(Json(ChainVerificationResponse { chains: items }))
}

/// POST /api/v1/admin/chain-verification/run
///
/// Run one verification chunk for both chains immediately and return the
/// refreshed statuses. Same rolling semantics as the background sweep.
pub async fn run_chain_verification(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ChainVerificationResponse>> {
    require_admin(&state, &auth_user).await?;

    let report = chain_verify_service::run_once(
        &state.db,
        state.audit_chain_hmac_key.as_slice(),
        state.billing_ledger_hmac_key.as_slice(),
    )
    .await?;
    Ok(Json(ChainVerificationResponse {
        chains: vec![
            chain_status_item(report.audit),
            chain_status_item(report.billing_ledger),
        ],
    }))
}

// --- Broker Runtime Settings ---

/// GET /api/v1/admin/settings/broker
///
/// Read the effective runtime broker policy. Requires full platform-admin
/// access because the values are security rollout controls.
pub async fn get_broker_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<BrokerSettingsResponse>> {
    require_admin(&state, &auth_user).await?;

    Ok(Json(broker_settings_response(&state)))
}

/// PATCH /api/v1/admin/settings/broker
///
/// Set or clear runtime broker-policy overrides. `null` clears an override and
/// returns the setting to its env-derived default.
pub async fn update_broker_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateBrokerSettingsRequest>,
) -> AppResult<Json<BrokerSettingsResponse>> {
    require_admin(&state, &auth_user).await?;

    let changed_fields = broker_settings_changed_fields(&body);
    let settings = platform_settings_service::update_broker_settings(
        &state.db,
        platform_settings_service::BrokerSettingsPatch {
            broker_require_sender_constraint: body.broker_require_sender_constraint,
            broker_require_admin_capability: body.broker_require_admin_capability,
        },
    )
    .await?;
    let policy = platform_settings_service::BrokerPolicy::from_settings(&state.config, &settings);
    state.set_broker_policy_if_fresh(policy);

    if !changed_fields.is_empty() {
        audit_service::log_for_user(
            state.db.clone(),
            &auth_user,
            "admin_broker_settings_updated",
            Some(serde_json::json!({
                "actor_user_id": auth_user.user_id.to_string(),
                "changed_fields": changed_fields,
            })),
        );
    }

    Ok(Json(broker_settings_response(&state)))
}

fn broker_settings_changed_fields(body: &UpdateBrokerSettingsRequest) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if body.broker_require_sender_constraint.is_some() {
        changed.push("broker_require_sender_constraint");
    }
    if body.broker_require_admin_capability.is_some() {
        changed.push("broker_require_admin_capability");
    }
    changed
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

#[derive(Debug, Deserialize)]
pub struct UpdateOAuthClientRequest {
    pub broker_capability_enabled: Option<bool>,
    pub is_active: Option<bool>,
    pub redirect_uris: Option<Vec<String>>,
    pub allowed_scopes: Option<Vec<String>>,
    pub client_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct OAuthClientListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
    pub search_filters: Option<String>,
    pub custom_filters: Option<String>,
    pub client_type: Option<String>,
    pub creator_type: Option<String>,
    pub broker: Option<String>,
    pub is_active: Option<String>,
    pub scope: Option<String>,
    pub created_dates: Option<String>,
    pub created_from: Option<String>,
    pub created_to: Option<String>,
    pub sort: Option<String>,
}

impl OAuthClientListQuery {
    fn has_table_controls(&self) -> bool {
        self.page.is_some()
            || self.per_page.is_some()
            || self.search.is_some()
            || self.search_filters.is_some()
            || self.custom_filters.is_some()
            || self.client_type.is_some()
            || self.creator_type.is_some()
            || self.broker.is_some()
            || self.is_active.is_some()
            || self.scope.is_some()
            || self.created_dates.is_some()
            || self.created_from.is_some()
            || self.created_to.is_some()
            || self.sort.is_some()
    }
}

#[derive(Debug, Serialize)]
pub struct OAuthClientResponse {
    pub id: String,
    pub client_name: String,
    pub client_type: String,
    pub created_by: Option<String>,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: String,
    pub delegation_scopes: String,
    pub broker_capability_enabled: bool,
    pub broker_capability_effective: bool,
    pub broker_capability_source: BrokerCapabilitySource,
    pub revocation_webhook_url: Option<String>,
    pub is_active: bool,
    /// Raw client secret -- only returned at creation time.
    pub client_secret: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OAuthClientListResponse {
    Legacy(OAuthClientLegacyListResponse),
    Paginated(Box<OAuthClientPaginatedListResponse>),
}

#[derive(Debug, Serialize)]
pub struct OAuthClientLegacyListResponse {
    pub clients: Vec<OAuthClientResponse>,
}

#[derive(Debug, Serialize)]
pub struct OAuthClientPaginatedListResponse {
    pub clients: Vec<OAuthClientResponse>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub filter_options: OAuthClientFilterOptions,
}

#[derive(Debug, Serialize)]
pub struct OAuthClientFilterOptions {
    pub client_types: Vec<&'static str>,
    pub creator_types: Vec<&'static str>,
    pub broker_filters: Vec<&'static str>,
    pub statuses: Vec<bool>,
    pub allowed_scopes: Vec<&'static str>,
    pub sorts: Vec<&'static str>,
    pub search_fields: Vec<OAuthClientSearchField>,
    pub fields: Vec<OAuthClientFilterField>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct OAuthClientSearchField {
    pub key: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OAuthClientFilterValueType {
    Enum,
    Boolean,
    Date,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OAuthClientFilterOperator {
    Is,
    Includes,
    Between,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct OAuthClientFilterOption {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct OAuthClientFilterField {
    pub key: &'static str,
    pub label: &'static str,
    pub value_type: OAuthClientFilterValueType,
    pub operator: OAuthClientFilterOperator,
    pub multiple: bool,
    pub options: Vec<OAuthClientFilterOption>,
    /// Whether the filter also accepts free text, matched as a case-insensitive
    /// `contains` against the field's stored column and OR'd with its options.
    pub supports_custom_text: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerCapabilitySource {
    None,
    Flag,
    Scope,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerPolicySource {
    EnvDefault,
    Override,
}

#[derive(Debug, Serialize)]
pub struct BrokerPolicyFieldResponse {
    pub effective: bool,
    pub env_default: bool,
    #[serde(rename = "override")]
    pub override_value: Option<bool>,
    pub source: BrokerPolicySource,
}

#[derive(Debug, Serialize)]
pub struct BrokerSettingsResponse {
    pub broker_require_sender_constraint: BrokerPolicyFieldResponse,
    pub broker_require_admin_capability: BrokerPolicyFieldResponse,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBrokerSettingsRequest {
    /// Omit to leave unchanged, pass `true`/`false` to override, pass `null`
    /// to clear back to the env default.
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub broker_require_sender_constraint: Option<Option<bool>>,
    /// Omit to leave unchanged, pass `true`/`false` to override, pass `null`
    /// to clear back to the env default.
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub broker_require_admin_capability: Option<Option<bool>>,
}

fn oauth_client_response(
    client: crate::models::oauth_client::OauthClient,
    client_secret: Option<String>,
    broker_require_admin_capability: bool,
) -> OAuthClientResponse {
    let broker_capability_source =
        broker_capability_source(&client, broker_require_admin_capability);
    let broker_capability_effective =
        !matches!(broker_capability_source, BrokerCapabilitySource::None);

    OAuthClientResponse {
        id: client.id,
        client_name: client.client_name,
        client_type: client.client_type,
        created_by: client.created_by,
        redirect_uris: client.redirect_uris,
        allowed_scopes: client.allowed_scopes,
        delegation_scopes: client.delegation_scopes,
        broker_capability_enabled: client.broker_capability_enabled,
        broker_capability_effective,
        broker_capability_source,
        revocation_webhook_url: client.revocation_webhook_url,
        is_active: client.is_active,
        client_secret,
        created_at: client.created_at.to_rfc3339(),
    }
}

fn broker_capability_source(
    client: &crate::models::oauth_client::OauthClient,
    broker_require_admin_capability: bool,
) -> BrokerCapabilitySource {
    if client.broker_capability_enabled {
        return BrokerCapabilitySource::Flag;
    }

    let has_legacy_scope = client
        .allowed_scopes
        .split_whitespace()
        .any(|scope| scope == crate::services::oauth_broker_service::BROKER_BINDING_SCOPE);
    if !broker_require_admin_capability && has_legacy_scope {
        BrokerCapabilitySource::Scope
    } else {
        BrokerCapabilitySource::None
    }
}

fn broker_settings_response(state: &AppState) -> BrokerSettingsResponse {
    let policy = state.broker_policy();
    BrokerSettingsResponse {
        broker_require_sender_constraint: broker_policy_field_response(
            policy.broker_require_sender_constraint,
            policy.broker_require_sender_constraint_env_default,
            policy.broker_require_sender_constraint_override,
        ),
        broker_require_admin_capability: broker_policy_field_response(
            policy.broker_require_admin_capability,
            policy.broker_require_admin_capability_env_default,
            policy.broker_require_admin_capability_override,
        ),
    }
}

fn oauth_client_filter_option(value: &'static str, label: &'static str) -> OAuthClientFilterOption {
    OAuthClientFilterOption { value, label }
}

/// Single source of truth: a filter offers a custom-text box exactly when the
/// service can map it to a stored column to run the `contains` against.
fn oauth_client_filter_takes_custom_text(key: &str) -> bool {
    oauth_client_service::admin_custom_text_field(key).is_some()
}

fn oauth_scope_label(scope: &'static str) -> &'static str {
    match scope {
        "openid" => "OpenID",
        "profile" => "Profile",
        "email" => "Email",
        "roles" => "Roles",
        "groups" => "Groups",
        "offline_access" => "Offline access",
        "proxy" => "Proxy",
        "urn:nyxid:scope:broker_binding" => "Broker binding",
        _ => scope,
    }
}

fn oauth_client_filter_options() -> OAuthClientFilterOptions {
    OAuthClientFilterOptions {
        client_types: oauth_client_service::ADMIN_CLIENT_TYPE_FILTERS.to_vec(),
        creator_types: oauth_client_service::ADMIN_CREATOR_TYPE_FILTERS.to_vec(),
        broker_filters: oauth_client_service::ADMIN_BROKER_FILTERS.to_vec(),
        statuses: vec![true, false],
        allowed_scopes: oauth_client_service::KNOWN_OIDC_SCOPES.to_vec(),
        sorts: oauth_client_service::ADMIN_SORT_OPTIONS.to_vec(),
        search_fields: oauth_client_service::ADMIN_SEARCH_FIELDS
            .iter()
            .map(|(key, label)| OAuthClientSearchField { key, label })
            .collect(),
        fields: vec![
            OAuthClientFilterField {
                key: "is_active",
                label: "Status",
                value_type: OAuthClientFilterValueType::Boolean,
                operator: OAuthClientFilterOperator::Is,
                multiple: true,
                options: vec![
                    oauth_client_filter_option("true", "Active"),
                    oauth_client_filter_option("false", "Inactive"),
                ],
                supports_custom_text: oauth_client_filter_takes_custom_text("is_active"),
            },
            OAuthClientFilterField {
                key: "client_type",
                label: "Client type",
                value_type: OAuthClientFilterValueType::Enum,
                operator: OAuthClientFilterOperator::Is,
                multiple: true,
                options: vec![
                    oauth_client_filter_option("public", "Public"),
                    oauth_client_filter_option("confidential", "Confidential"),
                    oauth_client_filter_option("other", "Other"),
                ],
                supports_custom_text: oauth_client_filter_takes_custom_text("client_type"),
            },
            OAuthClientFilterField {
                key: "creator_type",
                label: "Creator",
                value_type: OAuthClientFilterValueType::Enum,
                operator: OAuthClientFilterOperator::Is,
                multiple: true,
                options: vec![
                    oauth_client_filter_option("dynamic_registration", "Dynamic registration"),
                    oauth_client_filter_option("system", "System"),
                    oauth_client_filter_option("owned", "User / org"),
                    oauth_client_filter_option("ownerless", "Ownerless"),
                ],
                supports_custom_text: oauth_client_filter_takes_custom_text("creator_type"),
            },
            OAuthClientFilterField {
                key: "broker",
                label: "Broker capability",
                value_type: OAuthClientFilterValueType::Enum,
                operator: OAuthClientFilterOperator::Is,
                multiple: true,
                options: vec![
                    oauth_client_filter_option("enabled", "Enabled"),
                    oauth_client_filter_option("disabled", "Disabled"),
                    oauth_client_filter_option("flag", "Enabled by admin grant"),
                    oauth_client_filter_option("scope", "Enabled by broker scope"),
                ],
                supports_custom_text: oauth_client_filter_takes_custom_text("broker"),
            },
            OAuthClientFilterField {
                key: "scope",
                label: "Allowed scope",
                value_type: OAuthClientFilterValueType::Enum,
                operator: OAuthClientFilterOperator::Includes,
                multiple: true,
                options: oauth_client_service::KNOWN_OIDC_SCOPES
                    .iter()
                    .map(|scope| oauth_client_filter_option(scope, oauth_scope_label(scope)))
                    .collect(),
                supports_custom_text: oauth_client_filter_takes_custom_text("scope"),
            },
            OAuthClientFilterField {
                key: "created_at",
                label: "Created",
                value_type: OAuthClientFilterValueType::Date,
                operator: OAuthClientFilterOperator::Between,
                multiple: true,
                options: vec![],
                supports_custom_text: oauth_client_filter_takes_custom_text("created_at"),
            },
        ],
    }
}

fn broker_policy_field_response(
    effective: bool,
    env_default: bool,
    override_value: Option<bool>,
) -> BrokerPolicyFieldResponse {
    BrokerPolicyFieldResponse {
        effective,
        env_default,
        override_value,
        source: if override_value.is_some() {
            BrokerPolicySource::Override
        } else {
            BrokerPolicySource::EnvDefault
        },
    }
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

    let redirect_uris = oauth_client_service::validate_redirect_uris(&body.redirect_uris)?;

    let client_type = body.client_type.as_deref().unwrap_or("confidential");
    if client_type != "confidential" && client_type != "public" {
        return Err(AppError::ValidationError(
            "client_type must be 'confidential' or 'public'".to_string(),
        ));
    }

    let user_id = auth_user.user_id.to_string();
    let delegation_scopes = body.delegation_scopes.as_deref().unwrap_or("");

    // OAuth-client delegation deliberately excludes the service-only
    // `account:read` capability.
    oauth_client_service::validate_oauth_client_delegation_scopes(delegation_scopes)?;

    let allowed_scopes = body
        .allowed_scopes
        .as_deref()
        .map(oauth_client_service::validate_allowed_scopes_list)
        .transpose()?
        .unwrap_or_else(|| oauth_client_service::DEFAULT_ALLOWED_SCOPES.to_string());
    let revocation_webhook_url =
        normalize_optional_nonempty(body.revocation_webhook_url.as_deref());
    if let Some(url) = revocation_webhook_url {
        crate::services::webhook_delivery_service::validate_webhook_url(
            url,
            "revocation_webhook_url",
        )
        .await?;
    }
    let revocation_webhook_secret_encrypted =
        match normalize_optional_nonempty(body.revocation_webhook_secret.as_deref()) {
            Some(secret) => Some(state.encryption_keys.encrypt(secret.as_bytes()).await?),
            None => None,
        };

    let (client, raw_secret) = oauth_client_service::create_client(
        &state.db,
        &body.name,
        &redirect_uris,
        client_type,
        &user_id,
        delegation_scopes,
        &allowed_scopes,
        crate::models::oauth_client::ScopeProvenance::Explicit,
        body.broker_capability_enabled.unwrap_or(false),
        revocation_webhook_url,
        revocation_webhook_secret_encrypted,
        &[],
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

    Ok(Json(oauth_client_response(
        client,
        raw_secret,
        state.broker_require_admin_capability(),
    )))
}

/// GET /api/v1/admin/oauth-clients
///
/// List all OAuth clients. Requires admin privileges.
pub async fn list_oauth_clients(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<OAuthClientListQuery>,
) -> AppResult<Json<OAuthClientListResponse>> {
    require_admin_or_operator(&state, &auth_user, "admin.oauth_clients.list").await?;

    let broker_policy = state.broker_policy();
    if !query.has_table_controls() {
        let clients = oauth_client_service::list_clients_legacy(&state.db).await?;
        let items = clients
            .into_iter()
            .map(|client| {
                oauth_client_response(client, None, broker_policy.broker_require_admin_capability)
            })
            .collect();
        return Ok(Json(OAuthClientListResponse::Legacy(
            OAuthClientLegacyListResponse { clients: items },
        )));
    }

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(25).clamp(1, 100);
    let sort = query.sort.as_deref().unwrap_or("-created_at");
    let (clients, total) = oauth_client_service::list_clients(
        &state.db,
        oauth_client_service::AdminOAuthClientListParams {
            page,
            per_page,
            search: query.search.as_deref(),
            search_filters: query.search_filters.as_deref(),
            custom_filters: query.custom_filters.as_deref(),
            client_type: query.client_type.as_deref(),
            creator_type: query.creator_type.as_deref(),
            broker: query.broker.as_deref(),
            is_active: query.is_active.as_deref(),
            scope: query.scope.as_deref(),
            created_dates: query.created_dates.as_deref(),
            created_from: query.created_from.as_deref(),
            created_to: query.created_to.as_deref(),
            sort,
            broker_require_admin_capability: broker_policy.broker_require_admin_capability,
        },
    )
    .await?;

    let items: Vec<OAuthClientResponse> = clients
        .into_iter()
        .map(|client| {
            oauth_client_response(client, None, broker_policy.broker_require_admin_capability)
        })
        .collect();

    Ok(Json(OAuthClientListResponse::Paginated(Box::new(
        OAuthClientPaginatedListResponse {
            clients: items,
            total,
            page,
            per_page,
            filter_options: oauth_client_filter_options(),
        },
    ))))
}

/// PATCH /api/v1/admin/oauth-clients/:client_id
///
/// Update admin-managed OAuth-client fields by client ID. Requires full
/// platform-admin write privileges; operators are intentionally rejected.
pub async fn update_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
    Json(body): Json<UpdateOAuthClientRequest>,
) -> AppResult<Json<OAuthClientResponse>> {
    require_admin(&state, &auth_user).await?;

    let client_name = body.client_name.as_deref().map(str::trim);
    if client_name == Some("") {
        return Err(AppError::ValidationError(
            "Client name cannot be empty".to_string(),
        ));
    }

    let redirect_uris = body
        .redirect_uris
        .as_ref()
        .map(|uris| oauth_client_service::validate_redirect_uris(uris))
        .transpose()?;
    let allowed_scopes = body
        .allowed_scopes
        .as_deref()
        .map(oauth_client_service::validate_allowed_scopes_list)
        .transpose()?;

    let changed_fields = oauth_client_update_changed_fields(&body);
    let updated = oauth_client_service::admin_update_client(
        &state.db,
        &client_id,
        oauth_client_service::AdminUpdateClient {
            client_name,
            redirect_uris: redirect_uris.as_deref(),
            allowed_scopes: allowed_scopes.as_deref(),
            broker_capability_enabled: body.broker_capability_enabled,
            is_active: body.is_active,
        },
    )
    .await?;

    if !changed_fields.is_empty() {
        audit_service::log_for_user(
            state.db.clone(),
            &auth_user,
            "admin_oauth_client_updated",
            Some(serde_json::json!({
                "actor_user_id": auth_user.user_id.to_string(),
                "client_id": client_id,
                "changed_fields": changed_fields,
            })),
        );
    }

    Ok(Json(oauth_client_response(
        updated,
        None,
        state.broker_require_admin_capability(),
    )))
}

fn oauth_client_update_changed_fields(body: &UpdateOAuthClientRequest) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if body.broker_capability_enabled.is_some() {
        changed.push("broker_capability_enabled");
    }
    if body.is_active.is_some() {
        changed.push("is_active");
    }
    if body.redirect_uris.is_some() {
        changed.push("redirect_uris");
    }
    if body.allowed_scopes.is_some() {
        changed.push("allowed_scopes");
    }
    if body.client_name.is_some() {
        changed.push("client_name");
    }
    changed
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

    fn lookup_with(id: &str, slug: &str, name: &str) -> ServiceLookup {
        let resolved = ResolvedService {
            name: name.to_string(),
            slug: slug.to_string(),
        };
        let mut by_id = HashMap::new();
        by_id.insert(id.to_string(), resolved.clone());
        let mut by_slug = HashMap::new();
        by_slug.insert(slug.to_string(), resolved);
        ServiceLookup { by_id, by_slug }
    }

    #[test]
    fn collect_service_refs_splits_uuid_and_slug() {
        let mut ids = HashSet::new();
        let mut slugs = HashSet::new();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        collect_service_refs(
            Some(&serde_json::json!({ "service_id": uuid })),
            &mut ids,
            &mut slugs,
        );
        collect_service_refs(
            Some(&serde_json::json!({ "service_id": "openai" })),
            &mut ids,
            &mut slugs,
        );
        collect_service_refs(
            Some(&serde_json::json!({ "service_slug": "openclaw" })),
            &mut ids,
            &mut slugs,
        );
        collect_service_refs(
            Some(&serde_json::json!({ "service_id": "" })),
            &mut ids,
            &mut slugs,
        );
        collect_service_refs(None, &mut ids, &mut slugs);
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(uuid));
        assert_eq!(slugs.len(), 2);
        assert!(slugs.contains("openai") && slugs.contains("openclaw"));
    }

    #[test]
    fn resolve_entry_service_resolves_by_slug_and_id() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let lookup = lookup_with(uuid, "openai", "OpenAI API");

        // event_data carrying the canonical slug
        let (name, slug) = resolve_entry_service(
            Some(&serde_json::json!({ "service_slug": "openai" })),
            &lookup,
        );
        assert_eq!(name.as_deref(), Some("OpenAI API"));
        assert_eq!(slug.as_deref(), Some("openai"));

        // service_id as UUID (from /proxy/{uuid}/... routes)
        let (name, slug) =
            resolve_entry_service(Some(&serde_json::json!({ "service_id": uuid })), &lookup);
        assert_eq!(name.as_deref(), Some("OpenAI API"));
        assert_eq!(slug.as_deref(), Some("openai"));

        // service_id as slug (from /proxy/s/{slug}/... routes)
        let (name, slug) = resolve_entry_service(
            Some(&serde_json::json!({ "service_id": "openai" })),
            &lookup,
        );
        assert_eq!(name.as_deref(), Some("OpenAI API"));
        assert_eq!(slug.as_deref(), Some("openai"));
    }

    #[tokio::test]
    async fn resolve_lookups_enrich_from_database() {
        use crate::models::downstream_service::test_helpers::dummy_service;
        use crate::test_utils::connect_test_database;

        let Some(db) = connect_test_database("admin_audit_enrich").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let active_id = uuid::Uuid::new_v4().to_string();
        let active = DownstreamService {
            id: active_id.clone(),
            name: "OpenAI API".to_string(),
            slug: "openai".to_string(),
            is_active: true,
            ..dummy_service()
        };
        // Deactivated services must still resolve: audit rows reference them
        // long after an admin retires the catalog entry.
        let inactive = DownstreamService {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Retired Service".to_string(),
            slug: "retired".to_string(),
            is_active: false,
            ..dummy_service()
        };
        let services = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        services.insert_one(&active).await.expect("insert active");
        services
            .insert_one(&inactive)
            .await
            .expect("insert inactive");

        let user_id = uuid::Uuid::new_v4().to_string();
        let user = make_user(&user_id);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert user");

        let make_entry = |event_data: serde_json::Value, uid: Option<&str>| AuditLog {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uid.map(str::to_string),
            event_type: "proxy_request".to_string(),
            event_data: Some(event_data),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
            seq: None,
            prev_hash: None,
            entry_hash: None,
            created_at: Utc::now(),
        };
        let entries = vec![
            make_entry(
                serde_json::json!({ "service_id": active_id }),
                Some(&user_id),
            ),
            make_entry(serde_json::json!({ "service_id": "openai" }), None),
            make_entry(serde_json::json!({ "service_slug": "retired" }), None),
        ];

        let service_lookup = resolve_service_lookup(&db, &entries)
            .await
            .expect("service lookup");
        let user_lookup = resolve_user_lookup(&db, &entries)
            .await
            .expect("user lookup");

        // UUID and slug references resolve to the same catalog row.
        let (name, slug) = resolve_entry_service(entries[0].event_data.as_ref(), &service_lookup);
        assert_eq!(name.as_deref(), Some("OpenAI API"));
        assert_eq!(slug.as_deref(), Some("openai"));
        let (name, slug) = resolve_entry_service(entries[1].event_data.as_ref(), &service_lookup);
        assert_eq!(name.as_deref(), Some("OpenAI API"));
        assert_eq!(slug.as_deref(), Some("openai"));

        // Inactive service still resolves by slug.
        let (name, slug) = resolve_entry_service(entries[2].event_data.as_ref(), &service_lookup);
        assert_eq!(name.as_deref(), Some("Retired Service"));
        assert_eq!(slug.as_deref(), Some("retired"));

        let (display_name, email) = user_lookup.get(&user_id).cloned().expect("user resolved");
        assert_eq!(display_name.as_deref(), Some("Test User"));
        assert_eq!(email, format!("{user_id}@example.com"));
    }

    #[test]
    fn resolve_entry_service_unmatched_falls_back_to_raw_slug() {
        let lookup = ServiceLookup {
            by_id: HashMap::new(),
            by_slug: HashMap::new(),
        };
        // Slug that no longer matches a catalog row still surfaces raw
        let (name, slug) = resolve_entry_service(
            Some(&serde_json::json!({ "service_slug": "deleted-service" })),
            &lookup,
        );
        assert_eq!(name, None);
        assert_eq!(slug.as_deref(), Some("deleted-service"));

        // Unmatched UUID yields nothing renderable (raw UUID is already in event_data)
        let (name, slug) = resolve_entry_service(
            Some(&serde_json::json!({ "service_id": "550e8400-e29b-41d4-a716-446655440000" })),
            &lookup,
        );
        assert_eq!(name, None);
        assert_eq!(slug, None);

        // No service context at all
        let (name, slug) = resolve_entry_service(Some(&serde_json::json!({ "foo": 1 })), &lookup);
        assert_eq!(name, None);
        assert_eq!(slug, None);
        let (name, slug) = resolve_entry_service(None, &lookup);
        assert_eq!(name, None);
        assert_eq!(slug, None);
    }

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
            slug: None,
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

    #[test]
    fn oauth_client_list_query_accepts_scalar_multi_status_and_date_values() {
        let scalar_uri: http::Uri = "/api/v1/admin/oauth-clients?is_active=true"
            .parse()
            .unwrap();
        let Query(scalar) = Query::<OAuthClientListQuery>::try_from_uri(&scalar_uri).unwrap();
        assert_eq!(scalar.is_active.as_deref(), Some("true"));

        let multi_uri: http::Uri = "/api/v1/admin/oauth-clients?is_active=true%2Cfalse&client_type=public%2Cconfidential&created_from=2026-07-03&created_to=2026-07-10&created_dates=2026-06-01%2C2026-06-15"
            .parse()
            .unwrap();
        let Query(multi) = Query::<OAuthClientListQuery>::try_from_uri(&multi_uri).unwrap();
        assert_eq!(multi.is_active.as_deref(), Some("true,false"));
        assert_eq!(multi.client_type.as_deref(), Some("public,confidential"));
        assert_eq!(multi.created_from.as_deref(), Some("2026-07-03"));
        assert_eq!(multi.created_to.as_deref(), Some("2026-07-10"));
        assert_eq!(
            multi.created_dates.as_deref(),
            Some("2026-06-01,2026-06-15")
        );

        let search_uri: http::Uri = "/api/v1/admin/oauth-clients?search_filters=%5B%7B%22field%22%3A%22client%22%2C%22values%22%3A%5B%22console%22%2C%22portal%22%5D%7D%5D"
            .parse()
            .unwrap();
        let Query(search) = Query::<OAuthClientListQuery>::try_from_uri(&search_uri).unwrap();
        assert_eq!(
            search.search_filters.as_deref(),
            Some(r#"[{"field":"client","values":["console","portal"]}]"#)
        );
    }

    #[test]
    fn oauth_client_list_query_only_opts_in_for_recognized_table_controls() {
        let unknown_uri: http::Uri = "/api/v1/admin/oauth-clients?future_parameter=value"
            .parse()
            .unwrap();
        let Query(unknown) = Query::<OAuthClientListQuery>::try_from_uri(&unknown_uri).unwrap();
        assert!(!unknown.has_table_controls());

        for query in [
            "page=1",
            "per_page=25",
            "search=client",
            "search_filters=%5B%7B%22field%22%3A%22client%22%2C%22values%22%3A%5B%22console%22%5D%7D%5D",
            "client_type=public",
            "creator_type=system",
            "broker=enabled",
            "is_active=true",
            "scope=openid",
            "created_dates=2026-07-03",
            "created_from=2026-07-01",
            "created_to=2026-07-31",
            "sort=-created_at",
        ] {
            let uri: http::Uri = format!("/api/v1/admin/oauth-clients?{query}")
                .parse()
                .unwrap();
            let Query(parsed) = Query::<OAuthClientListQuery>::try_from_uri(&uri).unwrap();
            assert!(
                parsed.has_table_controls(),
                "recognized parameter must opt into table behavior: {query}"
            );
        }
    }

    #[test]
    fn oauth_client_filter_metadata_describes_every_supported_field() {
        let metadata = oauth_client_filter_options();
        assert_eq!(
            metadata
                .search_fields
                .iter()
                .map(|field| (field.key, field.label))
                .collect::<Vec<_>>(),
            [
                ("client", "Client"),
                ("client_type", "Client type"),
                ("created_by", "Created by"),
                ("allowed_scopes", "Allowed scopes"),
            ]
        );
        assert_eq!(
            metadata
                .fields
                .iter()
                .filter(|field| field.multiple)
                .count(),
            6
        );
        assert_eq!(
            metadata
                .fields
                .iter()
                .map(|field| field.key)
                .collect::<Vec<_>>(),
            [
                "is_active",
                "client_type",
                "creator_type",
                "broker",
                "scope",
                "created_at",
            ]
        );

        let status = metadata
            .fields
            .iter()
            .find(|field| field.key == "is_active")
            .expect("status metadata");
        assert_eq!(status.value_type, OAuthClientFilterValueType::Boolean);
        assert_eq!(status.operator, OAuthClientFilterOperator::Is);
        assert!(status.multiple);
        assert_eq!(
            status
                .options
                .iter()
                .map(|option| (option.value, option.label))
                .collect::<Vec<_>>(),
            [("true", "Active"), ("false", "Inactive")]
        );

        let scopes = metadata
            .fields
            .iter()
            .find(|field| field.key == "scope")
            .expect("scope metadata");
        assert_eq!(scopes.value_type, OAuthClientFilterValueType::Enum);
        assert_eq!(scopes.operator, OAuthClientFilterOperator::Includes);
        assert!(scopes.multiple);
        assert_eq!(
            scopes.options.len(),
            oauth_client_service::KNOWN_OIDC_SCOPES.len()
        );
        assert!(scopes.options.iter().any(|option| {
            option.value == crate::services::oauth_broker_service::BROKER_BINDING_SCOPE
                && option.label == "Broker binding"
        }));

        let created_at = metadata
            .fields
            .iter()
            .find(|field| field.key == "created_at")
            .expect("created-at metadata");
        assert_eq!(created_at.value_type, OAuthClientFilterValueType::Date);
        assert_eq!(created_at.operator, OAuthClientFilterOperator::Between);
        assert_eq!(created_at.label, "Created");
        assert!(created_at.multiple);
        assert!(created_at.options.is_empty());

        for sort in [
            "client_name",
            "client_type",
            "created_by",
            "broker",
            "is_active",
            "allowed_scopes",
            "created_at",
        ] {
            assert!(
                metadata.sorts.contains(&sort),
                "missing ascending {sort} sort"
            );
            assert!(
                metadata.sorts.contains(&format!("-{sort}").as_str()),
                "missing descending {sort} sort"
            );
        }
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
    use crate::models::authorization_code::{AuthorizationCode, COLLECTION_NAME as AUTH_CODES};
    use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
    use crate::models::platform_settings::COLLECTION_NAME as PLATFORM_SETTINGS;
    use crate::models::user::UserType;
    use crate::services::{audit_service, role_service};
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
                user_type: None,
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

    fn test_oauth_client(id: &str) -> OauthClient {
        let now = chrono::Utc::now();
        OauthClient {
            id: id.to_string(),
            client_name: "Aevatar DCR".to_string(),
            client_secret_hash: "NONE".to_string(),
            redirect_uris: vec!["https://aevatar.example/callback".to_string()],
            allowed_scopes: oauth_client_service::DEFAULT_MCP_ALLOWED_SCOPES.to_string(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code".to_string(),
            client_type: "public".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: Some("dynamic_registration".to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn oauth_client_response_reports_effective_scope_triggered_broker_capability() {
        let mut client = test_oauth_client("scope-triggered-client");
        client.allowed_scopes = format!(
            "openid {}",
            crate::services::oauth_broker_service::BROKER_BINDING_SCOPE
        );
        client.broker_capability_enabled = false;

        let legacy_policy_response = oauth_client_response(client.clone(), None, false);
        assert!(legacy_policy_response.broker_capability_effective);
        assert_eq!(
            legacy_policy_response.broker_capability_source,
            BrokerCapabilitySource::Scope
        );

        let admin_policy_response = oauth_client_response(client, None, true);
        assert!(!admin_policy_response.broker_capability_effective);
        assert_eq!(
            admin_policy_response.broker_capability_source,
            BrokerCapabilitySource::None
        );
    }

    #[tokio::test]
    async fn operator_lists_paginated_oauth_clients_with_metadata_and_no_secret() {
        let Some(db) = connect_test_database("admin_oauth_client_list_operator").await else {
            eprintln!("skipping admin oauth-client list test: no local MongoDB available");
            return;
        };
        let operator_id = insert_user(&db, false, true).await;
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(test_oauth_client("operator-visible-client"))
            .await
            .expect("insert OAuth client");

        let response = list_oauth_clients(
            State(test_app_state(db)),
            test_auth_user(&operator_id),
            Query(OAuthClientListQuery {
                page: Some(1),
                ..OAuthClientListQuery::default()
            }),
        )
        .await
        .expect("operator can list OAuth clients")
        .0;

        let OAuthClientListResponse::Paginated(response) = response else {
            panic!("explicit page must select paginated response");
        };
        assert_eq!(response.total, 1);
        assert_eq!(response.page, 1);
        assert_eq!(response.per_page, 25);
        assert_eq!(response.clients.len(), 1);
        assert!(response.clients[0].client_secret.is_none());
        assert_eq!(
            response.filter_options.client_types,
            oauth_client_service::ADMIN_CLIENT_TYPE_FILTERS
        );
        assert_eq!(
            response.filter_options.allowed_scopes,
            oauth_client_service::KNOWN_OIDC_SCOPES
        );
        assert_eq!(response.filter_options.fields.len(), 6);
        assert_eq!(
            response
                .filter_options
                .fields
                .iter()
                .map(|field| field.key)
                .collect::<Vec<_>>(),
            [
                "is_active",
                "client_type",
                "creator_type",
                "broker",
                "scope",
                "created_at",
            ]
        );
    }

    #[tokio::test]
    async fn operator_no_query_gets_complete_legacy_shape_without_secrets() {
        let Some(db) = connect_test_database("admin_oauth_client_list_legacy_operator").await
        else {
            eprintln!("skipping admin OAuth-client legacy list test: no local MongoDB available");
            return;
        };
        let operator_id = insert_user(&db, false, true).await;
        let now = chrono::Utc::now();
        let mut older = test_oauth_client("legacy-first");
        older.created_at = now - chrono::Duration::days(1);
        older.updated_at = older.created_at;
        let mut newer = test_oauth_client("legacy-second");
        newer.created_at = now;
        newer.updated_at = newer.created_at;
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_many([older, newer])
            .await
            .expect("insert legacy OAuth clients");

        let response = list_oauth_clients(
            State(test_app_state(db)),
            test_auth_user(&operator_id),
            Query(OAuthClientListQuery::default()),
        )
        .await
        .expect("operator can use legacy OAuth-client list")
        .0;
        let json = serde_json::to_value(&response).expect("serialize legacy response");
        assert_eq!(
            json.as_object()
                .expect("legacy response object")
                .keys()
                .collect::<Vec<_>>(),
            ["clients"]
        );

        let OAuthClientListResponse::Legacy(response) = response else {
            panic!("no-query request must select legacy response");
        };
        assert_eq!(response.clients.len(), 2);
        assert_eq!(response.clients[0].id, "legacy-second");
        assert_eq!(response.clients[1].id, "legacy-first");
        assert!(
            response
                .clients
                .iter()
                .all(|client| client.client_secret.is_none())
        );
    }

    #[tokio::test]
    async fn admin_patch_oauth_client_updates_ownerless_dynamic_registration_client() {
        let Some(db) = connect_test_database("admin_oauth_client_patch_dcr").await else {
            eprintln!("skipping admin oauth-client patch test: no local MongoDB available");
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let client_id = "dcr-aevatar-client";
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(test_oauth_client(client_id))
            .await
            .expect("insert dcr client");
        let now = chrono::Utc::now();
        db.collection::<AuthorizationCode>(AUTH_CODES)
            .insert_one(AuthorizationCode {
                id: "pending-auth-code".to_string(),
                code_hash: "pending-auth-code-hash".to_string(),
                client_id: client_id.to_string(),
                user_id: admin_id.clone(),
                redirect_uri: "https://aevatar.example/callback".to_string(),
                scope: "openid".to_string(),
                code_challenge: None,
                code_challenge_method: None,
                nonce: None,
                external_subject: None,
                binding_grant_id: None,
                resource_uris: Vec::new(),
                allowed_service_ids: Vec::new(),
                allow_all_services: true,
                expires_at: now + chrono::Duration::minutes(5),
                used: false,
                created_at: now,
            })
            .await
            .expect("insert pending auth code");
        let state = test_app_state(db.clone());
        let audit_rx =
            audit_service::notify_on_audit_write_for_user("admin_oauth_client_updated", &admin_id);

        let response = update_oauth_client(
            State(state),
            test_auth_user(&admin_id),
            Path(client_id.to_string()),
            Json(UpdateOAuthClientRequest {
                broker_capability_enabled: Some(true),
                is_active: Some(false),
                redirect_uris: Some(vec![
                    " https://aevatar.example/new-callback ".to_string(),
                    "https://aevatar.example/new-callback".to_string(),
                ]),
                allowed_scopes: Some(vec![
                    "openid".to_string(),
                    "urn:nyxid:scope:broker_binding".to_string(),
                ]),
                client_name: Some("Aevatar Broker".to_string()),
            }),
        )
        .await
        .expect("admin can patch DCR client");

        assert_eq!(response.0.id, client_id);
        assert_eq!(
            response.0.created_by.as_deref(),
            Some("dynamic_registration")
        );
        assert!(response.0.broker_capability_enabled);
        assert!(!response.0.is_active);
        assert!(response.0.client_secret.is_none());
        assert_eq!(
            response.0.redirect_uris,
            vec!["https://aevatar.example/new-callback".to_string()]
        );
        assert_eq!(
            response.0.allowed_scopes,
            "openid urn:nyxid:scope:broker_binding"
        );
        let pending_codes = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .count_documents(doc! { "client_id": client_id, "used": false })
            .await
            .expect("count pending authorization codes");
        assert_eq!(pending_codes, 0);

        tokio::time::timeout(std::time::Duration::from_secs(2), audit_rx)
            .await
            .expect("audit write should finish")
            .expect("audit watcher should receive id");
    }

    #[tokio::test]
    async fn oauth_client_patch_rejects_operator_and_plain_user() {
        let Some(db) = connect_test_database("admin_oauth_client_patch_reject").await else {
            eprintln!("skipping admin oauth-client rejection test: no local MongoDB available");
            return;
        };
        let operator_id = insert_user(&db, false, true).await;
        let plain_id = insert_user(&db, false, false).await;
        let client_id = "dcr-reject-client";
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(test_oauth_client(client_id))
            .await
            .expect("insert dcr client");
        let state = test_app_state(db);

        for actor_id in [operator_id, plain_id] {
            let err = update_oauth_client(
                State(state.clone()),
                test_auth_user(&actor_id),
                Path(client_id.to_string()),
                Json(UpdateOAuthClientRequest {
                    broker_capability_enabled: Some(true),
                    is_active: None,
                    redirect_uris: None,
                    allowed_scopes: None,
                    client_name: None,
                }),
            )
            .await
            .expect_err("non-admin must not patch OAuth clients");
            assert!(
                matches!(err, AppError::Forbidden(_)),
                "expected Forbidden for actor {actor_id}, got {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn oauth_client_patch_validates_scopes_and_redirect_uris() {
        let Some(db) = connect_test_database("admin_oauth_client_patch_validation").await else {
            eprintln!("skipping admin oauth-client validation test: no local MongoDB available");
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let client_id = "dcr-validation-client";
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(test_oauth_client(client_id))
            .await
            .expect("insert dcr client");
        let state = test_app_state(db);

        let scope_err = update_oauth_client(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(client_id.to_string()),
            Json(UpdateOAuthClientRequest {
                broker_capability_enabled: None,
                is_active: None,
                redirect_uris: None,
                allowed_scopes: Some(vec!["admin".to_string()]),
                client_name: None,
            }),
        )
        .await
        .expect_err("invalid scope must be rejected");
        assert!(matches!(scope_err, AppError::ValidationError(_)));

        let uri_err = update_oauth_client(
            State(state),
            test_auth_user(&admin_id),
            Path(client_id.to_string()),
            Json(UpdateOAuthClientRequest {
                broker_capability_enabled: None,
                is_active: None,
                redirect_uris: Some(vec!["javascript:alert(1)".to_string()]),
                allowed_scopes: None,
                client_name: None,
            }),
        )
        .await
        .expect_err("invalid redirect_uri must be rejected");
        assert!(matches!(uri_err, AppError::ValidationError(_)));
    }

    #[test]
    fn broker_settings_request_distinguishes_omitted_null_and_bool() {
        let omitted: UpdateBrokerSettingsRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(omitted.broker_require_sender_constraint, None);
        assert_eq!(omitted.broker_require_admin_capability, None);

        let cleared: UpdateBrokerSettingsRequest = serde_json::from_str(
            r#"{
                "broker_require_sender_constraint": null,
                "broker_require_admin_capability": null
            }"#,
        )
        .unwrap();
        assert_eq!(cleared.broker_require_sender_constraint, Some(None));
        assert_eq!(cleared.broker_require_admin_capability, Some(None));

        let overridden: UpdateBrokerSettingsRequest = serde_json::from_str(
            r#"{
                "broker_require_sender_constraint": true,
                "broker_require_admin_capability": false
            }"#,
        )
        .unwrap();
        assert_eq!(
            overridden.broker_require_sender_constraint,
            Some(Some(true))
        );
        assert_eq!(
            overridden.broker_require_admin_capability,
            Some(Some(false))
        );
    }

    #[tokio::test]
    async fn broker_settings_get_set_and_clear_refreshes_runtime_policy() {
        let Some(db) = connect_test_database("admin_broker_settings").await else {
            eprintln!("skipping broker settings test: no local MongoDB available");
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let mut config = crate::test_utils::test_app_config();
        config.broker_require_sender_constraint = false;
        config.broker_require_admin_capability = true;
        let state = crate::test_utils::test_app_state_with_config(db.clone(), config);

        let initial = get_broker_settings(State(state.clone()), test_auth_user(&admin_id))
            .await
            .expect("admin can read broker settings");
        assert!(!initial.0.broker_require_sender_constraint.effective);
        assert!(initial.0.broker_require_admin_capability.effective);
        assert!(matches!(
            initial.0.broker_require_sender_constraint.source,
            BrokerPolicySource::EnvDefault
        ));

        let audit_rx = audit_service::notify_on_audit_write_for_user(
            "admin_broker_settings_updated",
            &admin_id,
        );
        let updated = update_broker_settings(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(UpdateBrokerSettingsRequest {
                broker_require_sender_constraint: Some(Some(true)),
                broker_require_admin_capability: Some(Some(false)),
            }),
        )
        .await
        .expect("admin can update broker settings");
        assert!(updated.0.broker_require_sender_constraint.effective);
        assert!(!updated.0.broker_require_admin_capability.effective);
        assert!(state.broker_require_sender_constraint());
        assert!(!state.broker_require_admin_capability());
        let current_policy = state.broker_policy();
        let stale_policy = platform_settings_service::BrokerPolicy {
            revision: current_policy.revision - 1,
            broker_require_sender_constraint: !current_policy.broker_require_sender_constraint,
            broker_require_sender_constraint_env_default: current_policy
                .broker_require_sender_constraint_env_default,
            broker_require_sender_constraint_override: Some(
                !current_policy.broker_require_sender_constraint,
            ),
            broker_require_admin_capability: !current_policy.broker_require_admin_capability,
            broker_require_admin_capability_env_default: current_policy
                .broker_require_admin_capability_env_default,
            broker_require_admin_capability_override: Some(
                !current_policy.broker_require_admin_capability,
            ),
        };
        assert!(!state.set_broker_policy_if_fresh(stale_policy));
        assert_eq!(state.broker_policy(), current_policy);
        tokio::time::timeout(std::time::Duration::from_secs(2), audit_rx)
            .await
            .expect("audit write should finish")
            .expect("audit watcher should receive id");

        let cleared = update_broker_settings(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(UpdateBrokerSettingsRequest {
                broker_require_sender_constraint: Some(None),
                broker_require_admin_capability: Some(None),
            }),
        )
        .await
        .expect("admin can clear broker settings");
        assert!(!cleared.0.broker_require_sender_constraint.effective);
        assert!(cleared.0.broker_require_admin_capability.effective);
        assert!(!state.broker_require_sender_constraint());
        assert!(state.broker_require_admin_capability());

        let stored = db
            .collection::<mongodb::bson::Document>(PLATFORM_SETTINGS)
            .find_one(doc! { "_id": "platform" })
            .await
            .expect("query settings")
            .expect("settings doc exists");
        assert!(!stored.contains_key("broker_require_sender_constraint"));
        assert!(!stored.contains_key("broker_require_admin_capability"));
        assert_eq!(stored.get_i64("broker_policy_revision").unwrap(), 2);
    }

    #[tokio::test]
    async fn broker_settings_rejects_operator_and_plain_user() {
        let Some(db) = connect_test_database("admin_broker_settings_reject").await else {
            eprintln!("skipping broker settings rejection test: no local MongoDB available");
            return;
        };
        let operator_id = insert_user(&db, false, true).await;
        let plain_id = insert_user(&db, false, false).await;
        let state = test_app_state(db);

        for actor_id in [operator_id, plain_id] {
            let get_err = get_broker_settings(State(state.clone()), test_auth_user(&actor_id))
                .await
                .expect_err("non-admin must not read broker settings");
            assert!(matches!(get_err, AppError::Forbidden(_)));

            let patch_err = update_broker_settings(
                State(state.clone()),
                test_auth_user(&actor_id),
                Json(UpdateBrokerSettingsRequest {
                    broker_require_sender_constraint: Some(Some(true)),
                    broker_require_admin_capability: None,
                }),
            )
            .await
            .expect_err("non-admin must not patch broker settings");
            assert!(matches!(patch_err, AppError::Forbidden(_)));
        }
    }

    #[tokio::test]
    async fn broker_settings_runtime_override_flips_dcr_gate_without_restart() {
        let Some(db) = connect_test_database("admin_broker_settings_dcr_flip").await else {
            eprintln!("skipping broker settings DCR flip test: no local MongoDB available");
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let mut config = crate::test_utils::test_app_config();
        config.broker_require_admin_capability = false;
        let state = crate::test_utils::test_app_state_with_config(db, config);
        let broker_scope = format!(
            "openid {}",
            crate::services::oauth_broker_service::BROKER_BINDING_SCOPE
        );

        let (_status, _body) = crate::handlers::oauth::register_client(
            State(state.clone()),
            Json(crate::handlers::oauth::RegisterClientRequest {
                client_name: Some("Allowed Before Override".to_string()),
                redirect_uris: Some(vec!["http://localhost:8080/callback".to_string()]),
                grant_types: None,
                response_types: None,
                token_endpoint_auth_method: Some("none".to_string()),
                scope: Some(broker_scope.clone()),
            }),
        )
        .await
        .expect("env-default false allows broker scope in DCR");

        let _updated = update_broker_settings(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(UpdateBrokerSettingsRequest {
                broker_require_sender_constraint: None,
                broker_require_admin_capability: Some(Some(true)),
            }),
        )
        .await
        .expect("admin can enable runtime admin-capability requirement");
        assert!(state.broker_require_admin_capability());

        let err = crate::handlers::oauth::register_client(
            State(state),
            Json(crate::handlers::oauth::RegisterClientRequest {
                client_name: Some("Rejected After Override".to_string()),
                redirect_uris: Some(vec!["http://localhost:8081/callback".to_string()]),
                grant_types: None,
                response_types: None,
                token_endpoint_auth_method: Some("none".to_string()),
                scope: Some(broker_scope),
            }),
        )
        .await
        .expect_err("runtime override should reject broker DCR without restart");
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn broker_settings_runtime_override_flips_developer_app_gate_without_restart() {
        let Some(db) = connect_test_database("admin_broker_settings_dev_app_flip").await else {
            eprintln!(
                "skipping broker settings developer app flip test: no local MongoDB available"
            );
            return;
        };
        let admin_id = insert_user(&db, true, false).await;
        let plain_id = insert_user(&db, false, false).await;
        let mut config = crate::test_utils::test_app_config();
        config.broker_require_admin_capability = false;
        let state = crate::test_utils::test_app_state_with_config(db, config);

        let _created = crate::handlers::developer_apps::create_my_oauth_client(
            State(state.clone()),
            test_auth_user(&plain_id),
            TelemetryContext::default(),
            Json(
                crate::handlers::developer_apps::CreateDeveloperOAuthClientRequest {
                    name: "Allowed broker app".to_string(),
                    redirect_uris: vec!["https://app.example/allowed".to_string()],
                    client_type: Some("public".to_string()),
                    delegation_scopes: None,
                    broker_capability_enabled: Some(true),
                    revocation_webhook_url: None,
                    revocation_webhook_secret: None,
                    allowed_scopes: None,
                    target_org_id: None,
                    default_service_catalog_slugs: None,
                },
            ),
        )
        .await
        .expect("env-default false allows self-service broker flag");

        let _updated = update_broker_settings(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(UpdateBrokerSettingsRequest {
                broker_require_sender_constraint: None,
                broker_require_admin_capability: Some(Some(true)),
            }),
        )
        .await
        .expect("admin can enable runtime admin-capability requirement");

        let err = crate::handlers::developer_apps::create_my_oauth_client(
            State(state),
            test_auth_user(&plain_id),
            TelemetryContext::default(),
            Json(
                crate::handlers::developer_apps::CreateDeveloperOAuthClientRequest {
                    name: "Rejected broker app".to_string(),
                    redirect_uris: vec!["https://app.example/rejected".to_string()],
                    client_type: Some("public".to_string()),
                    delegation_scopes: None,
                    broker_capability_enabled: Some(true),
                    revocation_webhook_url: None,
                    revocation_webhook_secret: None,
                    allowed_scopes: None,
                    target_org_id: None,
                    default_service_catalog_slugs: None,
                },
            ),
        )
        .await
        .expect_err("runtime override should reject non-admin broker app provisioning");
        assert!(matches!(err, AppError::Forbidden(_)));
    }
}
