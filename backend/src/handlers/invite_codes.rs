use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::require_admin;
use crate::handlers::auth::{extract_ip, extract_user_agent};
use crate::models::invite_code::{InviteCode, InviteCodeUsage};
use crate::mw::auth::AuthUser;
use crate::services::invite_code_service::InviteCodeUsageUser;
use crate::services::{audit_service, invite_code_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Request / Response types ---
//
// Note: the 1..=1000 bound on `max_uses` is enforced by the `validator` crate
// attributes on `CreateInviteCodeRequest::max_uses` below. Keeping the limit
// at the request-type level means the error message is returned through the
// normal validation-error path instead of a bespoke handler check.

#[derive(Debug, Deserialize, Validate)]
pub struct CreateInviteCodeRequest {
    #[validate(range(min = 1, max = 1000, message = "max_uses must be between 1 and 1000"))]
    pub max_uses: Option<i32>,
    #[validate(length(max = 512, message = "Note must be at most 512 characters"))]
    pub note: Option<String>,
}

/// Body for `PATCH /api/v1/admin/invite-codes/{id}`.
///
/// The `note` field is authoritative: whatever value is sent (or absent)
/// becomes the new note. Specifically:
/// - `{"note": "text"}` → sets the note to "text"
/// - `{"note": ""}` → clears the note (stored as null)
/// - `{"note": null}` → clears the note
/// - `{}` (field omitted) → clears the note
///
/// Today only the note is mutable; other fields (code, max_uses, is_active)
/// stay immutable after creation.
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateInviteCodeRequest {
    #[validate(length(max = 512, message = "Note must be at most 512 characters"))]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InviteCodeUsageResponse {
    pub user_id: String,
    pub used_at: String,
    /// Email of the user who redeemed the code, or `null` if the user has
    /// been deleted since the redemption was recorded.
    pub user_email: Option<String>,
    /// Display name of the user who redeemed the code, or `null` if the user
    /// has no display name set or has been deleted.
    pub user_display_name: Option<String>,
}

/// Nested sidecar describing the admin who created this invite code. Resolved
/// via the same batch user lookup used for redemption enrichment, so exposing
/// this adds zero extra DB round-trips. `None` when the creator has been
/// deleted since the code was minted — callers should fall back to rendering
/// the raw `created_by` UUID in that case.
#[derive(Debug, Serialize)]
pub struct InviteCodeCreator {
    /// Email of the admin. Always present whenever `creator` itself is non-null
    /// (the user projection requires it).
    pub email: String,
    /// Display name of the admin, or `null` if they have no display name set.
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InviteCodeResponse {
    pub id: String,
    pub code: String,
    pub max_uses: i32,
    pub used_count: i32,
    pub created_by: String,
    /// Resolved creator info (email + display name). `null` when the admin
    /// has been deleted since the code was minted. See [`InviteCodeCreator`].
    pub creator: Option<InviteCodeCreator>,
    pub note: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
    pub usages: Vec<InviteCodeUsageResponse>,
}

#[derive(Debug, Serialize)]
pub struct InviteCodeListResponse {
    pub invite_codes: Vec<InviteCodeResponse>,
}

#[derive(Debug, Serialize)]
pub struct DeactivateInviteCodeResponse {
    pub message: String,
}

fn usage_to_response(
    usage: InviteCodeUsage,
    users: &HashMap<String, InviteCodeUsageUser>,
) -> InviteCodeUsageResponse {
    let lookup = users.get(&usage.user_id);
    InviteCodeUsageResponse {
        user_email: lookup.map(|u| u.email.clone()),
        user_display_name: lookup.and_then(|u| u.display_name.clone()),
        user_id: usage.user_id,
        used_at: usage.used_at.to_rfc3339(),
    }
}

fn to_response(ic: InviteCode, users: &HashMap<String, InviteCodeUsageUser>) -> InviteCodeResponse {
    let creator = users.get(&ic.created_by).map(|u| InviteCodeCreator {
        email: u.email.clone(),
        display_name: u.display_name.clone(),
    });
    InviteCodeResponse {
        id: ic.id,
        code: ic.code,
        max_uses: ic.max_uses,
        used_count: ic.used_count,
        creator,
        created_by: ic.created_by,
        note: ic.note,
        is_active: ic.is_active,
        created_at: ic.created_at.to_rfc3339(),
        updated_at: ic.updated_at.to_rfc3339(),
        usages: ic
            .usages
            .into_iter()
            .map(|u| usage_to_response(u, users))
            .collect(),
    }
}

// --- Handlers ---

/// POST /api/v1/admin/invite-codes
///
/// Create a new invite code (admin only).
pub async fn create_invite_code(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    headers: HeaderMap,
    Json(body): Json<CreateInviteCodeRequest>,
) -> AppResult<Json<InviteCodeResponse>> {
    require_admin(&state, &auth_user).await?;

    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let max_uses = body.max_uses.unwrap_or(10);
    let admin_id = auth_user.user_id.to_string();

    let invite = invite_code_service::create_invite_code(
        &state.db,
        &admin_id,
        max_uses,
        body.note.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin_invite_code_create".to_string(),
        Some(serde_json::json!({
            "invite_code_id": invite.id,
            "code": invite.code,
            "max_uses": invite.max_uses,
        })),
        extract_ip(&headers, Some(peer)),
        extract_user_agent(&headers),
        None,
        None,
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::InviteCodeGenerated {
            generated_by_role: "admin".to_string(),
        },
    );

    // A freshly-created code has no usages, so the empty user map is fine.
    Ok(Json(to_response(invite, &HashMap::new())))
}

/// GET /api/v1/admin/invite-codes
///
/// List all invite codes (admin only). Each usage entry is enriched with the
/// redeeming user's email and display name via a single batch lookup against
/// the `users` collection.
pub async fn list_invite_codes(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<InviteCodeListResponse>> {
    require_admin(&state, &auth_user).await?;

    let result = invite_code_service::list_invite_codes(&state.db).await?;

    Ok(Json(InviteCodeListResponse {
        invite_codes: result
            .codes
            .into_iter()
            .map(|ic| to_response(ic, &result.users))
            .collect(),
    }))
}

/// PATCH /api/v1/admin/invite-codes/{id}
///
/// Update mutable fields on an invite code (admin only). Currently only the
/// freeform `note` is mutable; the code value, max_uses, and is_active stay
/// immutable after creation. The audit log entry intentionally records *that*
/// the note changed rather than the new value, since notes can contain
/// freeform admin-supplied text we shouldn't persist twice.
pub async fn update_invite_code(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateInviteCodeRequest>,
) -> AppResult<Json<InviteCodeResponse>> {
    require_admin(&state, &auth_user).await?;

    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let updated = invite_code_service::update_invite_code_note(&state.db, &id, body.note).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin_invite_code_update".to_string(),
        Some(serde_json::json!({
            "invite_code_id": id,
            "fields_changed": ["note"],
        })),
        extract_ip(&headers, Some(peer)),
        extract_user_agent(&headers),
        None,
        None,
    );

    // Resolve usage users for the single updated code so the response carries
    // the same enrichment shape as the list endpoint. The drawer reuses this
    // payload and expects email/display_name on each usage entry.
    let users =
        invite_code_service::fetch_usage_users(&state.db, std::slice::from_ref(&updated)).await?;
    Ok(Json(to_response(updated, &users)))
}

/// DELETE /api/v1/admin/invite-codes/{id}
///
/// Deactivate an invite code (admin only).
pub async fn deactivate_invite_code(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<DeactivateInviteCodeResponse>> {
    require_admin(&state, &auth_user).await?;

    invite_code_service::deactivate_invite_code(&state.db, &id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin_invite_code_deactivate".to_string(),
        Some(serde_json::json!({ "invite_code_id": id })),
        extract_ip(&headers, Some(peer)),
        extract_user_agent(&headers),
        None,
        None,
    );

    Ok(Json(DeactivateInviteCodeResponse {
        message: "Invite code deactivated".to_string(),
    }))
}
