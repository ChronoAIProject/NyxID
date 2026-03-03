use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::mw::auth::AuthUser;
use crate::services::{approval_service, audit_service};

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct ApprovalRequestItem {
    pub id: String,
    pub service_name: String,
    pub service_slug: String,
    pub requester_type: String,
    pub requester_label: Option<String>,
    pub operation_summary: String,
    pub status: String,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub decision_channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApprovalRequestsResponse {
    pub requests: Vec<ApprovalRequestItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Serialize)]
pub struct ApprovalGrantItem {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    pub requester_type: String,
    pub requester_id: String,
    pub requester_label: Option<String>,
    pub granted_at: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct ApprovalGrantsResponse {
    pub grants: Vec<ApprovalGrantItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Serialize)]
pub struct ApprovalStatusResponse {
    pub status: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct DecideResponse {
    pub id: String,
    pub status: String,
    pub decided_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// --- Query/Request types ---

#[derive(Debug, Deserialize)]
pub struct ApprovalRequestsQuery {
    pub status: Option<String>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GrantsQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct DecideRequest {
    pub approved: bool,
}

// --- Handlers ---

/// GET /api/v1/approvals/requests
pub async fn list_requests(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ApprovalRequestsQuery>,
) -> AppResult<Json<ApprovalRequestsResponse>> {
    let user_id = auth_user.user_id.to_string();

    if let Some(ref status) = query.status {
        if !["pending", "approved", "rejected", "expired"].contains(&status.as_str()) {
            return Err(crate::errors::AppError::ValidationError(
                "status must be one of: pending, approved, rejected, expired".to_string(),
            ));
        }
    }

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).min(100);

    let (requests, total) = approval_service::list_requests(
        &state.db,
        &user_id,
        query.status.as_deref(),
        page,
        per_page,
    )
    .await?;

    let items: Vec<ApprovalRequestItem> = requests
        .into_iter()
        .map(|r| ApprovalRequestItem {
            id: r.id,
            service_name: r.service_name,
            service_slug: r.service_slug,
            requester_type: r.requester_type,
            requester_label: r.requester_label,
            operation_summary: r.operation_summary,
            status: r.status,
            created_at: r.created_at.to_rfc3339(),
            decided_at: r.decided_at.map(|d| d.to_rfc3339()),
            decision_channel: r.decision_channel,
        })
        .collect();

    Ok(Json(ApprovalRequestsResponse {
        requests: items,
        total,
        page,
        per_page,
    }))
}

/// GET /api/v1/approvals/requests/{request_id}/status
///
/// Polling endpoint for callers that received approval_required.
/// Accessible by delegated tokens and service accounts.
///
/// SECURITY: caller must authenticate and match the original requester binding
/// (resource owner + requester_type + requester_id).
pub async fn get_request_status(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<String>,
) -> AppResult<Json<ApprovalStatusResponse>> {
    let request = approval_service::get_request(&state.db, &request_id).await?;
    let owner_user_id = auth_user.effective_approval_owner_user_id();
    let requester_type = auth_user.approval_requester_type().ok_or_else(|| {
        AppError::Forbidden("Session-authenticated callers cannot poll approval status".to_string())
    })?;
    let requester_id = auth_user.approval_requester_id();

    if request.user_id != owner_user_id
        || request.requester_type != requester_type
        || request.requester_id != requester_id
    {
        return Err(AppError::Forbidden(
            "You are not authorized to view this approval request".to_string(),
        ));
    }

    Ok(Json(ApprovalStatusResponse {
        status: request.status,
        expires_at: request.expires_at.to_rfc3339(),
    }))
}

/// POST /api/v1/approvals/requests/{request_id}/decide
///
/// Approve or reject an approval request via the web UI.
pub async fn decide_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<String>,
    Json(body): Json<DecideRequest>,
) -> AppResult<Json<DecideResponse>> {
    let user_id = auth_user.user_id.to_string();

    // Verify the request belongs to this user
    let request = approval_service::get_request(&state.db, &request_id).await?;
    if request.user_id != user_id {
        return Err(crate::errors::AppError::Forbidden(
            "You can only decide on your own approval requests".to_string(),
        ));
    }

    let updated = approval_service::process_decision(
        &state.db,
        &state.config,
        &state.http_client,
        &request_id,
        body.approved,
        "web",
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "approval_decision".to_string(),
        Some(serde_json::json!({
            "request_id": request_id,
            "service_id": updated.service_id,
            "approved": body.approved,
            "channel": "web",
        })),
        None,
        None,
    );

    Ok(Json(DecideResponse {
        id: updated.id,
        status: updated.status,
        decided_at: updated.decided_at.map(|d| d.to_rfc3339()),
    }))
}

/// GET /api/v1/approvals/grants
pub async fn list_grants(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<GrantsQuery>,
) -> AppResult<Json<ApprovalGrantsResponse>> {
    let user_id = auth_user.user_id.to_string();
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).min(100);

    let (grants, total) =
        approval_service::list_grants(&state.db, &user_id, page, per_page).await?;

    let items: Vec<ApprovalGrantItem> = grants
        .into_iter()
        .map(|g| ApprovalGrantItem {
            id: g.id,
            service_id: g.service_id,
            service_name: g.service_name,
            requester_type: g.requester_type,
            requester_id: g.requester_id,
            requester_label: g.requester_label,
            granted_at: g.granted_at.to_rfc3339(),
            expires_at: g.expires_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ApprovalGrantsResponse {
        grants: items,
        total,
        page,
        per_page,
    }))
}

/// DELETE /api/v1/approvals/grants/{grant_id}
pub async fn revoke_grant(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(grant_id): Path<String>,
) -> AppResult<Json<MessageResponse>> {
    let user_id = auth_user.user_id.to_string();

    approval_service::revoke_grant(&state.db, &user_id, &grant_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "approval_grant_revoked".to_string(),
        Some(serde_json::json!({ "grant_id": grant_id })),
        None,
        None,
    );

    Ok(Json(MessageResponse {
        message: "Grant revoked".to_string(),
    }))
}

// --- Per-service approval config types ---

#[derive(Debug, Serialize)]
pub struct ServiceApprovalConfigItem {
    pub service_id: String,
    pub service_name: String,
    pub approval_required: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceApprovalConfigsResponse {
    pub configs: Vec<ServiceApprovalConfigItem>,
}

#[derive(Debug, Deserialize)]
pub struct SetServiceApprovalConfigRequest {
    pub approval_required: bool,
}

// --- Per-service approval config handlers ---

/// GET /api/v1/approvals/service-configs
///
/// List all per-service approval overrides for the current user.
pub async fn list_service_configs(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ServiceApprovalConfigsResponse>> {
    let user_id = auth_user.user_id.to_string();

    let configs = approval_service::list_service_approval_configs(&state.db, &user_id).await?;

    let items: Vec<ServiceApprovalConfigItem> = configs
        .into_iter()
        .map(|c| ServiceApprovalConfigItem {
            service_id: c.service_id,
            service_name: c.service_name,
            approval_required: c.approval_required,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ServiceApprovalConfigsResponse { configs: items }))
}

/// PUT /api/v1/approvals/service-configs/{service_id}
///
/// Set a per-service approval override. Creates or updates.
pub async fn set_service_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<SetServiceApprovalConfigRequest>,
) -> AppResult<Json<ServiceApprovalConfigItem>> {
    let user_id = auth_user.user_id.to_string();

    // Verify the service exists
    let service = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(mongodb::bson::doc! { "_id": &service_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    let config = approval_service::set_service_approval_config(
        &state.db,
        &user_id,
        &service_id,
        &service.name,
        body.approval_required,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "service_approval_config_set".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "service_name": service.name,
            "approval_required": body.approval_required,
        })),
        None,
        None,
    );

    Ok(Json(ServiceApprovalConfigItem {
        service_id: config.service_id,
        service_name: config.service_name,
        approval_required: config.approval_required,
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
    }))
}

/// DELETE /api/v1/approvals/service-configs/{service_id}
///
/// Remove a per-service approval override (revert to global default).
pub async fn delete_service_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<MessageResponse>> {
    let user_id = auth_user.user_id.to_string();

    approval_service::delete_service_approval_config(&state.db, &user_id, &service_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "service_approval_config_deleted".to_string(),
        Some(serde_json::json!({ "service_id": service_id })),
        None,
        None,
    );

    Ok(Json(MessageResponse {
        message: "Per-service approval config removed".to_string(),
    }))
}
