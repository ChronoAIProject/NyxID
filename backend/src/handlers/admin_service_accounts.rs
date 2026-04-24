use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin::AdminActionResponse;
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent, require_admin};
use crate::models::service_account::ServiceAccount;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, org_service, service_account_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

/// Gate access to a service account by either global admin OR admin of
/// the SA's owning org. Org-owned SAs (`owner_user_id` points at a
/// `user_type = org` user) can be managed by any admin of that org;
/// admin-created personal SAs still require global admin.
///
/// Returns the effective `created_by` value to pass through to the
/// service layer for downstream queries that filter by creator.
async fn require_admin_or_owning_org_admin(
    state: &AppState,
    auth_user: &AuthUser,
    sa: &ServiceAccount,
) -> AppResult<()> {
    // Global admin always allowed.
    if require_admin(state, auth_user).await.is_ok() {
        return Ok(());
    }

    // Otherwise the SA must be org-owned and the caller must be admin of
    // that org. `effective_owner_user_id` falls back to created_by for
    // pre-owner-field records.
    let owner = sa.effective_owner_user_id();
    let actor = auth_user.user_id.to_string();
    let access = org_service::resolve_owner_access(&state.db, &actor, owner).await?;
    if access.can_write() {
        return Ok(());
    }

    Err(AppError::Forbidden(
        "admin access required (global or owning org)".to_string(),
    ))
}

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct CreateServiceAccountRequest {
    pub name: String,
    pub description: Option<String>,
    pub allowed_scopes: String,
    pub role_ids: Option<Vec<String>>,
    pub rate_limit_override: Option<u64>,
    /// When set, create this service account under the given org. The
    /// SA's `created_by` and `owner_user_id` are both set to the org's
    /// user_id, making the SA manageable by every admin of that org.
    /// Caller must be an admin of the target org. When omitted, falls
    /// back to the legacy global-admin path (caller must be a NyxID admin).
    pub target_org_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServiceAccountListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
    /// When set, list service accounts owned by the given org instead of
    /// the global list. The caller must be an admin of that org. Without
    /// this filter the endpoint requires global admin and returns every
    /// service account in the system.
    pub org_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServiceAccountRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub allowed_scopes: Option<String>,
    pub role_ids: Option<Vec<String>>,
    pub rate_limit_override: Option<Option<u64>>,
    pub is_active: Option<bool>,
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct CreateServiceAccountResponse {
    pub id: String,
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub allowed_scopes: String,
    pub role_ids: Vec<String>,
    pub is_active: bool,
    pub created_at: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceAccountItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub client_id: String,
    pub secret_prefix: String,
    pub allowed_scopes: String,
    pub role_ids: Vec<String>,
    pub is_active: bool,
    pub rate_limit_override: Option<u64>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_authenticated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ServiceAccountListResponse {
    pub service_accounts: Vec<ServiceAccountItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Serialize)]
pub struct RotateSecretResponse {
    pub client_id: String,
    pub client_secret: String,
    pub secret_prefix: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct RevokeTokensResponse {
    pub revoked_count: u64,
    pub message: String,
}

// --- Helpers ---

fn sa_to_item(sa: ServiceAccount) -> ServiceAccountItem {
    ServiceAccountItem {
        id: sa.id,
        name: sa.name,
        description: sa.description,
        client_id: sa.client_id,
        secret_prefix: sa.secret_prefix,
        allowed_scopes: sa.allowed_scopes,
        role_ids: sa.role_ids,
        is_active: sa.is_active,
        rate_limit_override: sa.rate_limit_override,
        created_by: sa.created_by,
        created_at: sa.created_at.to_rfc3339(),
        updated_at: sa.updated_at.to_rfc3339(),
        last_authenticated_at: sa.last_authenticated_at.map(|t| t.to_rfc3339()),
    }
}

// --- Handlers ---

/// POST /api/v1/admin/service-accounts
pub async fn create_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    headers: HeaderMap,
    Json(body): Json<CreateServiceAccountRequest>,
) -> AppResult<Json<CreateServiceAccountResponse>> {
    let actor = auth_user.user_id.to_string();

    // Determine the effective owner. Two paths:
    // - target_org_id set: caller must be an admin of that org. The SA is
    //   created with owner = org user_id, so every admin of that org can
    //   manage it via the same endpoints (gated by
    //   `require_admin_or_owning_org_admin`).
    // - target_org_id not set: legacy admin-created SA, caller must be a
    //   global NyxID admin.
    let effective_owner = if let Some(target_org_id) = body.target_org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create service accounts under it"
                    .to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        require_admin(&state, &auth_user).await?;
        actor
    };

    let role_ids = body.role_ids.unwrap_or_default();

    let (sa, raw_secret) = service_account_service::create_service_account(
        &state.db,
        &body.name,
        body.description.as_deref(),
        &body.allowed_scopes,
        &role_ids,
        body.rate_limit_override,
        &effective_owner,
    )
    .await?;

    let admin_id = auth_user.user_id.to_string();

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.sa.created".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa.id,
            "client_id": &sa.client_id,
            "name": &sa.name,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
        None,
        None,
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminServiceAccountCreated,
    );

    Ok(Json(CreateServiceAccountResponse {
        id: sa.id,
        name: sa.name,
        client_id: sa.client_id,
        client_secret: raw_secret,
        allowed_scopes: sa.allowed_scopes,
        role_ids: sa.role_ids,
        is_active: sa.is_active,
        created_at: sa.created_at.to_rfc3339(),
        message:
            "Service account created. Save the client_secret now -- it cannot be retrieved later."
                .to_string(),
    }))
}

/// GET /api/v1/admin/service-accounts
pub async fn list_service_accounts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ServiceAccountListQuery>,
) -> AppResult<Json<ServiceAccountListResponse>> {
    let actor = auth_user.user_id.to_string();

    // Two listing modes:
    // - org_id set: org admin can list SAs owned by their org. No global
    //   admin requirement.
    // - org_id unset: legacy global listing, requires global admin.
    let owner_filter = if let Some(target_org_id) = query.org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its service accounts"
                    .to_string(),
            ));
        }
        Some(target_org_id.to_string())
    } else {
        require_admin(&state, &auth_user).await?;
        None
    };

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);

    let (accounts, total) = service_account_service::list_service_accounts(
        &state.db,
        page,
        per_page,
        query.search.as_deref(),
        owner_filter.as_deref(),
    )
    .await?;

    let items: Vec<ServiceAccountItem> = accounts.into_iter().map(sa_to_item).collect();

    Ok(Json(ServiceAccountListResponse {
        service_accounts: items,
        total,
        page,
        per_page,
    }))
}

/// GET /api/v1/admin/service-accounts/:sa_id
pub async fn get_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(sa_id): Path<String>,
) -> AppResult<Json<ServiceAccountItem>> {
    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    require_admin_or_owning_org_admin(&state, &auth_user, &sa).await?;

    Ok(Json(sa_to_item(sa)))
}

/// PUT /api/v1/admin/service-accounts/:sa_id
pub async fn update_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
    Json(body): Json<UpdateServiceAccountRequest>,
) -> AppResult<Json<ServiceAccountItem>> {
    let existing = service_account_service::get_service_account(&state.db, &sa_id).await?;
    require_admin_or_owning_org_admin(&state, &auth_user, &existing).await?;

    let updated = service_account_service::update_service_account(
        &state.db,
        &sa_id,
        body.name.as_deref(),
        body.description.as_deref(),
        body.allowed_scopes.as_deref(),
        body.role_ids.as_deref(),
        body.rate_limit_override,
        body.is_active,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.updated".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
        None,
        None,
    );

    Ok(Json(sa_to_item(updated)))
}

/// DELETE /api/v1/admin/service-accounts/:sa_id
pub async fn delete_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
) -> AppResult<Json<AdminActionResponse>> {
    let existing = service_account_service::get_service_account(&state.db, &sa_id).await?;
    require_admin_or_owning_org_admin(&state, &auth_user, &existing).await?;

    service_account_service::delete_service_account(&state.db, &sa_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.deleted".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
        None,
        None,
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminServiceAccountDeleted,
    );

    Ok(Json(AdminActionResponse {
        message: "Service account deactivated".to_string(),
    }))
}

/// POST /api/v1/admin/service-accounts/:sa_id/rotate-secret
pub async fn rotate_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
) -> AppResult<Json<RotateSecretResponse>> {
    let existing = service_account_service::get_service_account(&state.db, &sa_id).await?;
    require_admin_or_owning_org_admin(&state, &auth_user, &existing).await?;

    let (updated, raw_secret) = service_account_service::rotate_secret(&state.db, &sa_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.secret_rotated".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "client_id": &updated.client_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
        None,
        None,
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminServiceAccountRotated,
    );

    Ok(Json(RotateSecretResponse {
        client_id: updated.client_id,
        client_secret: raw_secret,
        secret_prefix: updated.secret_prefix,
        message: "Secret rotated. All existing tokens have been revoked. Save the new secret now."
            .to_string(),
    }))
}

/// POST /api/v1/admin/service-accounts/:sa_id/revoke-tokens
pub async fn revoke_tokens(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
) -> AppResult<Json<RevokeTokensResponse>> {
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    require_admin_or_owning_org_admin(&state, &auth_user, &_sa).await?;

    let revoked_count = service_account_service::revoke_all_tokens(&state.db, &sa_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.tokens_revoked".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "revoked_count": revoked_count,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
        None,
        None,
    );

    Ok(Json(RevokeTokensResponse {
        revoked_count,
        message: "All active tokens revoked".to_string(),
    }))
}
