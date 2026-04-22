use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::service_approval_config::ApprovalMode;
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
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
    pub action_description: Option<String>,
    /// Tool approval fields (null for proxy-initiated approvals)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_destructive: Option<bool>,
    pub approval_mode: ApprovalMode,
    pub status: String,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub decision_channel: Option<String>,
    /// True when this request was created under an org's per-service
    /// approval policy. Clients use this to render an "Org" badge and
    /// the `on behalf of {org_name}` context line.
    #[serde(default)]
    pub from_org_policy: bool,
    /// Owning org id, present only when `from_org_policy` is true.
    /// For org-policy requests this equals `ApprovalRequest.user_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    /// Owning org display name, resolved at list time via `org_service`.
    /// May be absent even when `from_org_policy` is true if the org row
    /// is missing a display name or the lookup failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
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
    /// True when the grant is owned by an org (reusable by any member of
    /// that org). Clients render an "Org" chip when set.
    #[serde(default)]
    pub org_scoped: bool,
    /// Owning org id, present only when `org_scoped` is true.
    /// For org-scoped grants this equals `ApprovalGrant.user_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    /// Owning org display name, resolved at list time via `org_service`.
    /// May be absent even when `org_scoped` is true if the org row
    /// is missing a display name or the lookup failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
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

// --- Tool approval types ---

#[derive(Debug, Deserialize)]
pub struct CreateToolApprovalRequest {
    pub tool_name: String,
    pub tool_call_id: Option<String>,
    pub arguments: Option<String>,
    pub is_destructive: Option<bool>,
    #[allow(dead_code)]
    pub approval_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateApprovalResponse {
    pub id: String,
    pub status: String,
    pub expires_at: String,
}

fn to_approval_request_item(
    request: crate::models::approval_request::ApprovalRequest,
    org_name: Option<String>,
) -> ApprovalRequestItem {
    let from_org_policy = request.from_org_policy;
    let org_id = if from_org_policy {
        Some(request.user_id.clone())
    } else {
        None
    };
    let resolved_org_name = if from_org_policy { org_name } else { None };
    ApprovalRequestItem {
        id: request.id,
        service_name: request.service_name,
        service_slug: request.service_slug,
        requester_type: request.requester_type,
        requester_label: request.requester_label,
        operation_summary: request.operation_summary,
        action_description: request.action_description,
        tool_name: request.tool_name,
        tool_call_id: request.tool_call_id,
        tool_arguments: request.tool_arguments,
        is_destructive: request.is_destructive,
        approval_mode: request.approval_mode,
        status: request.status,
        created_at: request.created_at.to_rfc3339(),
        decided_at: request.decided_at.map(|d| d.to_rfc3339()),
        decision_channel: request.decision_channel,
        from_org_policy,
        org_id,
        org_name: resolved_org_name,
    }
}

/// Map an `ApprovalGrant` to its API representation, stamping the org
/// context fields when the grant is org-scoped. `org_name` is passed in
/// after a batch lookup by the caller so we never issue per-row queries.
fn to_approval_grant_item(
    grant: crate::models::approval_grant::ApprovalGrant,
    org_name: Option<String>,
) -> ApprovalGrantItem {
    let org_scoped = grant.org_scoped;
    let org_id = if org_scoped {
        Some(grant.user_id.clone())
    } else {
        None
    };
    let resolved_org_name = if org_scoped { org_name } else { None };
    ApprovalGrantItem {
        id: grant.id,
        service_id: grant.service_id,
        service_name: grant.service_name,
        requester_type: grant.requester_type,
        requester_id: grant.requester_id,
        requester_label: grant.requester_label,
        granted_at: grant.granted_at.to_rfc3339(),
        expires_at: grant.expires_at.to_rfc3339(),
        org_scoped,
        org_id,
        org_name: resolved_org_name,
    }
}

/// Translate a scoped admin's `allowed_service_ids` (which live in
/// `UserService.id` space) into the concrete set of storage-space
/// `service_id`s that can match an `ApprovalRequest` or
/// `ApprovalGrant` row under the supplied org. The stored
/// `service_id` may be either a `UserService.id` (custom services)
/// or the `catalog_service_id` from the underlying `UserService`
/// (catalog-backed services), so we union both for every allowed
/// UserService that still exists.
///
/// This is the inverse direction of `scope_user_service_ids_for_config`:
/// that helper maps a stored row to the UserService.ids the scope is
/// written against; this one maps the scope forward into the space
/// the stored rows actually live in, so we can push the whole filter
/// into Mongo and keep pagination correct.
///
/// Returns an empty Vec when no supplied id resolves to a UserService
/// owned by the org (deleted services, or a mis-scoped membership).
/// Callers use an empty result to short-circuit the whole org branch.
async fn resolve_scope_storage_service_ids(
    db: &mongodb::Database,
    org_user_id: &str,
    allowed_user_service_ids: &[String],
) -> AppResult<Vec<String>> {
    use futures::TryStreamExt;
    if allowed_user_service_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids_bson = mongodb::bson::Array::new();
    for id in allowed_user_service_ids {
        ids_bson.push(mongodb::bson::Bson::String(id.clone()));
    }
    let rows: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(mongodb::bson::doc! {
            "_id": { "$in": ids_bson },
            "user_id": org_user_id,
        })
        .await?
        .try_collect()
        .await?;

    let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in rows {
        out.insert(row.id.clone());
        if let Some(cat) = row.catalog_service_id {
            out.insert(cat);
        }
    }
    Ok(out.into_iter().collect())
}

/// Build the `OrgFilterBranch` list the service layer consumes, given
/// the caller's current active admin memberships. Scoped admins get
/// their `allowed_service_ids` pre-resolved into storage-space ids so
/// the Mongo filter applies scope at query time (rather than post-
/// fetch, which would break pagination). Unscoped admins get
/// `service_id_scope: None`.
///
/// Returns an empty Vec when the caller has no admin memberships.
async fn resolve_admin_org_branches(
    db: &mongodb::Database,
    actor_user_id: &str,
) -> AppResult<Vec<approval_service::OrgFilterBranch>> {
    let memberships = crate::services::org_service::list_memberships_for_member(
        db,
        actor_user_id,
        false, // active only
    )
    .await?;

    let mut branches: Vec<approval_service::OrgFilterBranch> =
        Vec::with_capacity(memberships.len());
    for m in memberships {
        if !matches!(m.role, crate::models::org_membership::OrgRole::Admin) {
            continue;
        }
        let service_id_scope: Option<Vec<String>> = match m.allowed_service_ids {
            None => None,
            Some(ids) => Some(resolve_scope_storage_service_ids(db, &m.org_user_id, &ids).await?),
        };
        branches.push(approval_service::OrgFilterBranch {
            org_id: m.org_user_id,
            service_id_scope,
        });
    }
    Ok(branches)
}

/// Resolve a map of `org_id -> display_name` for the supplied set of
/// org ids. Looks each id up via `org_service::get_org_user` (reuse;
/// no new helper) and skips any that fail to resolve — missing org
/// names on the response are permitted and the client renders a
/// generic "Org" fallback. De-dupes inputs so each distinct org is
/// fetched at most once per list call.
async fn resolve_org_names(
    db: &mongodb::Database,
    org_ids: impl IntoIterator<Item = String>,
) -> std::collections::HashMap<String, String> {
    let mut unique: Vec<String> = org_ids.into_iter().collect();
    unique.sort();
    unique.dedup();
    let mut out = std::collections::HashMap::with_capacity(unique.len());
    for id in unique {
        match crate::services::org_service::get_org_user(db, &id).await {
            Ok(user) => {
                if let Some(name) = user.display_name.clone() {
                    out.insert(id, name);
                }
            }
            Err(e) => {
                tracing::warn!(org_id = %id, error = %e, "Failed to resolve org display name for approvals listing");
            }
        }
    }
    out
}

/// Legacy strict ownership check kept for the existing unit tests. The
/// runtime path now uses `ensure_caller_can_decide` to support org-policy
/// approvals.
#[cfg(test)]
fn ensure_request_owned_by_user(request_user_id: &str, auth_user_id: &str) -> AppResult<()> {
    if request_user_id != auth_user_id {
        return Err(AppError::Forbidden(
            "You are not authorized to view this approval request".to_string(),
        ));
    }

    Ok(())
}

/// Authorize a caller against an approval request that may belong to an
/// org. The caller is allowed if either of the following holds:
///
/// 1. They are the literal `request.user_id` owner (personal request).
/// 2. They are *currently* an admin of the org that owns the request
///    AND that admin's `allowed_service_ids` scope (if any) covers the
///    `UserService` backing the request.
///
/// `request.notify_user_ids` is intentionally NOT consulted here. It
/// is only a routing hint captured at request creation time, so for
/// org-policy requests it would otherwise let an admin who has since
/// been removed or demoted decide outstanding requests. The live
/// `resolve_owner_access` check is the single source of truth.
///
/// `request.service_id` is either a catalog `DownstreamService.id`
/// (catalog-backed services) or a `UserService.id` directly (custom
/// services — see ChronoAIProject/NyxID#165). `OrgMembership.allowed_service_ids`
/// lives in the `UserService.id` space, so we translate through the
/// shared `scope_user_service_ids_for_config` helper which covers both
/// cases: a direct `UserService.id` match *and* any `UserService` rows
/// that reference the id as their `catalog_service_id`. Without the
/// direct-match branch, a scoped admin would be notified for custom-
/// service approvals but denied on decision (empty id list → `allows_any_resource` false for scoped roles).
async fn ensure_caller_can_decide(
    db: &mongodb::Database,
    request: &crate::models::approval_request::ApprovalRequest,
    auth_user_id: &str,
) -> AppResult<()> {
    if request.user_id == auth_user_id {
        return Ok(());
    }

    // Check whether the request owner is an org and the caller is one of
    // its admins. `resolve_owner_access` returns Forbidden for non-org
    // owners or non-member callers, which collapses both "ex-admin" and
    // "stranger" into the same denial path.
    let access =
        crate::services::org_service::resolve_owner_access(db, auth_user_id, &request.user_id)
            .await?;
    if !access.can_write() {
        return Err(AppError::Forbidden(
            "You are not authorized to act on this approval request".to_string(),
        ));
    }

    // Translate the stored service id (catalog id *or* UserService id)
    // into the `UserService.id` resource space the org membership scope
    // uses, then gate on `allows_any_resource`. Empty result means the
    // backing UserService has been deleted — safer to require an
    // unscoped admin to decide it.
    let user_service_ids =
        scope_user_service_ids_for_config(db, &request.user_id, &request.service_id).await?;
    if !access.allows_any_resource(&user_service_ids) {
        return Err(AppError::Forbidden(
            "Your org admin role is scoped to other services and cannot decide on this approval request".to_string(),
        ));
    }

    Ok(())
}

// --- Query/Request types ---

#[derive(Debug, Deserialize)]
pub struct ApprovalRequestsQuery {
    pub status: Option<String>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    /// When set, list approval requests scoped to the given org instead of
    /// the caller's personal scope. The caller must be an admin of that
    /// org. Without this filter the endpoint returns the actor's personal
    /// approval history (existing behavior).
    pub org_id: Option<String>,
    /// Opt-in union: when `true` (and `org_id` is not set), also include
    /// org-policy requests owned by every org the caller is currently an
    /// active admin of. Default `false` keeps the existing personal-only
    /// behavior so web clients that distinguish personal vs org pages are
    /// unaffected. Mobile sets this so the unified Activity inbox can
    /// show personal + admin-org items together (see ChronoAIProject/NyxID#376).
    pub include_admin_orgs: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct GrantsQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    /// When set, list grants owned by the given org instead of the
    /// caller's personal scope. The caller must be an admin of that
    /// org. Org-policy approvals create grants under the org's user_id,
    /// so this is the only way for org admins to see them.
    pub org_id: Option<String>,
    /// Opt-in union for active grants. Same semantics as the field on
    /// `ApprovalRequestsQuery`. Default `false`.
    pub include_admin_orgs: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeGrantQuery {
    /// When set, revoke a grant owned by the given org. The caller
    /// must be an admin of that org.
    pub org_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DecideRequest {
    pub approved: bool,
    pub duration_sec: Option<i64>,
}

// --- Handlers ---

/// GET /api/v1/approvals/requests
pub async fn list_requests(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ApprovalRequestsQuery>,
) -> AppResult<Json<ApprovalRequestsResponse>> {
    let actor = auth_user.user_id.to_string();

    // `status` accepts one value ("pending") or a comma-separated list
    // ("approved,rejected,expired") so the history view can ask for
    // "everything except pending" in one query. Each token is validated
    // against the canonical set; whitespace around tokens is tolerated.
    let allowed_statuses = ["pending", "approved", "rejected", "expired"];
    let parsed_statuses: Vec<String> = match query.status.as_deref() {
        None => Vec::new(),
        Some(raw) => raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    };
    for s in &parsed_statuses {
        if !allowed_statuses.contains(&s.as_str()) {
            return Err(crate::errors::AppError::ValidationError(
                "status must be one of: pending, approved, rejected, expired (comma-separated list allowed)"
                    .to_string(),
            ));
        }
    }

    // org_id query param scopes the listing to org-policy approvals.
    // The caller must be an admin of the target org.
    let listing_user_id = if let Some(target_org_id) = query.org_id.as_deref() {
        let access =
            crate::services::org_service::resolve_owner_access(&state.db, &actor, target_org_id)
                .await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its approval history"
                    .to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor.clone()
    };

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).min(100);

    // Opt-in union: resolve admin-org branches (scope pre-translated
    // into storage-space service ids) so scope is enforced in the
    // Mongo filter itself. This keeps pagination correct for scoped
    // admins — post-fetch filtering could leave a scoped admin's
    // first page empty while later pages held in-scope rows, which
    // the mobile Activity inbox would treat as end-of-list. Union
    // only runs on the default personal listing; when `?org_id=` is
    // supplied the caller already pinned the scope to one org and the
    // existing single-owner path stays in charge.
    let admin_branches = if query.org_id.is_none() && query.include_admin_orgs.unwrap_or(false) {
        resolve_admin_org_branches(&state.db, &actor).await?
    } else {
        Vec::new()
    };

    let status_refs: Vec<&str> = parsed_statuses.iter().map(|s| s.as_str()).collect();
    let (requests, total) = approval_service::list_requests(
        &state.db,
        &listing_user_id,
        &admin_branches,
        &status_refs,
        page,
        per_page,
    )
    .await?;

    // Batch-resolve org display names for rows backed by an org policy.
    // When `?org_id=` is set every returned row shares the same owning
    // org, so this collapses to a single fetch; on the personal inbox
    // this fetches one name per distinct org the caller is an admin
    // for, bounded by the page size.
    let org_ids: Vec<String> = requests
        .iter()
        .filter(|r| r.from_org_policy)
        .map(|r| r.user_id.clone())
        .collect();
    let org_names = resolve_org_names(&state.db, org_ids).await;

    let items: Vec<ApprovalRequestItem> = requests
        .into_iter()
        .map(|r| {
            let name = if r.from_org_policy {
                org_names.get(&r.user_id).cloned()
            } else {
                None
            };
            to_approval_request_item(r, name)
        })
        .collect();

    Ok(Json(ApprovalRequestsResponse {
        requests: items,
        total,
        page,
        per_page,
    }))
}

/// GET /api/v1/approvals/requests/{request_id}
///
/// Returns approval request detail for the current user. Org admins can
/// view requests created under their org's policy in addition to their
/// own personal requests.
pub async fn get_request_by_id(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<String>,
) -> AppResult<Json<ApprovalRequestItem>> {
    let user_id = auth_user.user_id.to_string();
    let request = approval_service::get_request(&state.db, &request_id).await?;

    ensure_caller_can_decide(&state.db, &request, &user_id).await?;

    // Single-row path: look up the owning org name directly when the
    // request was created under an org policy so the detail response
    // carries the same org context as the list endpoint.
    let org_name = if request.from_org_policy {
        resolve_org_names(&state.db, std::iter::once(request.user_id.clone()))
            .await
            .remove(&request.user_id)
    } else {
        None
    };
    Ok(Json(to_approval_request_item(request, org_name)))
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

    // Requester binding must always match -- this is the defense against
    // an unrelated API key polling someone else's request.
    if request.requester_type != requester_type || request.requester_id != requester_id {
        return Err(AppError::Forbidden(
            "You are not authorized to view this approval request".to_string(),
        ));
    }

    // Owner check: the strict legacy path required `request.user_id == owner_user_id`
    // (the actor's effective approval owner). Org-policy requests live
    // under the org's user_id, so the strict check would block legitimate
    // polling by the agent that triggered the request. Allow either:
    // - the legacy match, OR
    // - the actor has any access to the owning org (member or admin),
    //   which means the request was created on their behalf via cascade.
    if request.user_id != owner_user_id {
        let access = crate::services::org_service::resolve_owner_access(
            &state.db,
            &owner_user_id,
            &request.user_id,
        )
        .await?;
        if !access.can_read() {
            return Err(AppError::Forbidden(
                "You are not authorized to view this approval request".to_string(),
            ));
        }
    }

    Ok(Json(ApprovalStatusResponse {
        status: request.status,
        expires_at: request.expires_at.to_rfc3339(),
    }))
}

/// POST /api/v1/approvals/requests
///
/// Create a tool approval request (for external callers such as Aevatar).
/// Triggers the same notification pipeline as proxy-initiated approvals.
pub async fn create_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateToolApprovalRequest>,
) -> AppResult<(axum::http::StatusCode, Json<CreateApprovalResponse>)> {
    // Validate tool_name
    let tool_name = body.tool_name.trim();
    if tool_name.is_empty() || tool_name.len() > 256 {
        return Err(AppError::ValidationError(
            "tool_name must be 1-256 characters".to_string(),
        ));
    }

    // Validate arguments length
    if let Some(ref args) = body.arguments
        && args.len() > 65536
    {
        return Err(AppError::ValidationError(
            "arguments must be at most 65536 characters".to_string(),
        ));
    }

    let user_id = auth_user.user_id.to_string();

    // Determine requester identity from auth context.
    // Session callers use "user" as requester_type (they are requesting
    // approval from themselves, valid for agents running in their session).
    let (requester_type, requester_id) = match auth_user.approval_requester_type() {
        Some(rt) => (rt.to_string(), auth_user.approval_requester_id()),
        None => ("user".to_string(), user_id.clone()),
    };

    let requester_label = auth_user.api_key_name.as_deref();

    let request = approval_service::create_tool_approval_request(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &user_id,
        tool_name,
        body.tool_call_id.as_deref(),
        body.arguments.as_deref(),
        body.is_destructive.unwrap_or(false),
        &requester_type,
        &requester_id,
        requester_label,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "tool_approval_created".to_string(),
        Some(serde_json::json!({
            "request_id": &request.id,
            "tool_name": tool_name,
            "is_destructive": body.is_destructive.unwrap_or(false),
        })),
        None,
        None,
        auth_user.api_key_id.as_deref().map(String::from),
        auth_user.api_key_name.clone(),
    );

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateApprovalResponse {
            id: request.id,
            status: request.status,
            expires_at: request.expires_at.to_rfc3339(),
        }),
    ))
}

/// POST /api/v1/approvals/requests/{request_id}/decide
///
/// Approve or reject an approval request via the web UI.
pub async fn decide_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DecideRequest>,
) -> AppResult<Json<DecideResponse>> {
    let user_id = auth_user.user_id.to_string();
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    // Verify the caller can act on this request -- direct owner, named
    // recipient, or current admin of the owning org.
    let request = approval_service::get_request(&state.db, &request_id).await?;
    ensure_caller_can_decide(&state.db, &request, &user_id).await?;

    if let Some(duration_sec) = body.duration_sec
        && duration_sec <= 0
    {
        return Err(crate::errors::AppError::ValidationError(
            "duration_sec must be positive".to_string(),
        ));
    }

    let updated = approval_service::process_decision(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.clone(),
        state.apns_auth.clone(),
        &request_id,
        body.approved,
        body.duration_sec,
        idempotency_key,
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
    let actor = auth_user.user_id.to_string();
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).min(100);

    // org_id query param scopes the listing to grants owned by an org
    // (created when an org-policy approval is granted in grant mode).
    // Caller must be an admin of the target org.
    let listing_user_id = if let Some(target_org_id) = query.org_id.as_deref() {
        let access =
            crate::services::org_service::resolve_owner_access(&state.db, &actor, target_org_id)
                .await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its approval grants"
                    .to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor.clone()
    };

    // Mirror `list_requests`: opt-in union only kicks in on the
    // personal listing path. Scope + per-owner grant-mode filtering
    // happens in the Mongo filter via `OrgFilterBranch`.
    let admin_branches = if query.org_id.is_none() && query.include_admin_orgs.unwrap_or(false) {
        resolve_admin_org_branches(&state.db, &actor).await?
    } else {
        Vec::new()
    };

    let (grants, total) =
        approval_service::list_grants(&state.db, &listing_user_id, &admin_branches, page, per_page)
            .await?;

    // Same batching strategy as `list_requests`: resolve each distinct
    // owning org id once per call. Personal grants (`org_scoped=false`)
    // contribute nothing to the lookup.
    let org_ids: Vec<String> = grants
        .iter()
        .filter(|g| g.org_scoped)
        .map(|g| g.user_id.clone())
        .collect();
    let org_names = resolve_org_names(&state.db, org_ids).await;

    let items: Vec<ApprovalGrantItem> = grants
        .into_iter()
        .map(|g| {
            let name = if g.org_scoped {
                org_names.get(&g.user_id).cloned()
            } else {
                None
            };
            to_approval_grant_item(g, name)
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
    Query(query): Query<RevokeGrantQuery>,
) -> AppResult<Json<MessageResponse>> {
    let actor = auth_user.user_id.to_string();

    // Same pattern as list_grants: when org_id is supplied, the grant
    // is expected to be owned by the org and the caller must be an
    // admin of that org. Without this branch, org-policy grants would
    // be unrevokable through the API.
    let owner_user_id = if let Some(target_org_id) = query.org_id.as_deref() {
        let access =
            crate::services::org_service::resolve_owner_access(&state.db, &actor, target_org_id)
                .await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to revoke its approval grants"
                    .to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor.clone()
    };

    approval_service::revoke_grant(&state.db, &owner_user_id, &grant_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "approval_grant_revoked".to_string(),
        Some(serde_json::json!({
            "grant_id": grant_id,
            "owner_user_id": owner_user_id,
        })),
        None,
        None,
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
    /// The storage key used by proxy resolution. For catalog-backed user
    /// services this is `catalog_service_id`; for custom user services this
    /// is the `UserService.id` itself.
    pub service_id: String,
    pub service_name: String,
    pub approval_required: bool,
    pub approval_mode: ApprovalMode,
    pub created_at: String,
    pub updated_at: String,
    /// The `UserService.id` that this policy applies to, when the config can
    /// be traced back to one of the owner's active user services. Clients
    /// should prefer this over `service_id` when cross-referencing against
    /// `/user-services`, so configured AI services line up with the proxy
    /// user sees in their dashboard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_service_id: Option<String>,
    /// Proxy slug of the matching `UserService`, for display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_service_slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ServiceApprovalConfigsResponse {
    pub configs: Vec<ServiceApprovalConfigItem>,
    /// `(org_id, service_id)` pairs where an org the caller is a member
    /// of has set its own per-service policy. `resolve_org_aware_approval`
    /// treats those org policies as dominant over the actor's personal
    /// config, so the UI should hide the matching entry from the
    /// personal Add-Override picker — but only for that specific org.
    /// When the same catalog service is inherited from a *different*
    /// org without its own policy, the personal override is still
    /// effective and should remain selectable.
    ///
    /// Populated only when listing the caller's personal configs (no
    /// `?org_id` query). Left empty for the org-scoped list since the
    /// org admin already sees the org's own configs directly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dominant_org_policies: Vec<DominantOrgPolicy>,
}

#[derive(Debug, Serialize)]
pub struct DominantOrgPolicy {
    pub org_id: String,
    pub service_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SetServiceApprovalConfigRequest {
    pub approval_required: Option<bool>,
    pub approval_mode: Option<ApprovalMode>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ServiceApprovalConfigQuery {
    /// When set, the operation targets the given org's policy instead of
    /// the caller's personal scope. The caller must be an admin of that
    /// org. Used by the per-service approval CRUD endpoints so an org
    /// admin can set/list/delete policies on org-shared services.
    pub org_id: Option<String>,
}

/// Resolve the effective `user_id` *and* the caller's `OwnerAccess` for
/// a service approval config operation.
///
/// Without `?org_id`, the actor manages their own personal configs and
/// `OwnerAccess::Direct` is returned (always passes scope checks).
///
/// With `?org_id=X`, the actor must be an admin of X. Returned access
/// carries the membership's `allowed_service_ids` so per-service scope
/// gating can run on the catalog id passed in the path. Without that
/// scope check a scoped admin (`allowed_service_ids = [svc-A]`) could
/// otherwise toggle the policy on svc-B, bypassing the scope model the
/// rest of the org-aware handlers enforce.
async fn resolve_service_config_owner(
    state: &AppState,
    actor: &str,
    org_id: Option<&str>,
) -> AppResult<(String, crate::services::org_service::OwnerAccess)> {
    if let Some(org) = org_id {
        let access =
            crate::services::org_service::resolve_owner_access(&state.db, actor, org).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to set per-service approval policy"
                    .to_string(),
            ));
        }
        Ok((org.to_string(), access))
    } else {
        Ok((
            actor.to_string(),
            crate::services::org_service::OwnerAccess::Direct,
        ))
    }
}

/// Collect the set of `UserService.id`s (the resource space that
/// `OrgMembership.allowed_service_ids` lives in) that a given approval
/// config `service_id` can reach:
///
/// 1. A direct `UserService.id` match (for custom services, or for policies
///    written against the UserService id itself).
/// 2. Every active `UserService` whose `catalog_service_id` matches
///    (for catalog-backed policies, which cover all of the owner's user
///    services that reuse that catalog entry).
///
/// The direct match deliberately does **not** filter on `is_active` —
/// otherwise a scoped admin loses the ability to decide or clean up an
/// outstanding approval the moment a custom service is deactivated,
/// stranding the request. `user_service_ids_for_catalog` has the same
/// always-visible semantics for catalog-backed rows; we mirror it here
/// so deactivation never widens denial. Active-only ownership is
/// enforced separately by `resolve_approval_target`, which governs
/// *creating* or *updating* configs (a distinct operation from
/// authorizing a caller to clean one up).
async fn scope_user_service_ids_for_config(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: &str,
) -> AppResult<Vec<String>> {
    let mut ids = Vec::new();
    if (db
        .collection::<UserService>(USER_SERVICES)
        .find_one(mongodb::bson::doc! {
            "_id": service_id,
            "user_id": owner_user_id,
        })
        .await?)
        .is_some()
    {
        ids.push(service_id.to_string());
    }
    let mut catalog_ids = crate::services::user_service_service::user_service_ids_for_catalog(
        db,
        owner_user_id,
        service_id,
    )
    .await?;
    for id in catalog_ids.drain(..) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Filter matching an *active*, owner-scoped `UserService` row. Used by
/// write paths (`resolve_approval_target`) where creating a new policy
/// against a deactivated service doesn't make sense. The scope/decision
/// path deliberately drops the `is_active` predicate — see
/// `scope_user_service_ids_for_config`.
fn doc_ownership(owner_user_id: &str, service_id: &str) -> mongodb::bson::Document {
    mongodb::bson::doc! {
        "_id": service_id,
        "user_id": owner_user_id,
        "is_active": true,
    }
}

/// Apply the org membership scope to a single approval-config target.
/// Translates the stored `service_id` (which may be a catalog
/// `DownstreamService.id` or a `UserService.id` for custom services) to
/// the underlying `UserService.id`s that live in the
/// `OrgMembership.allowed_service_ids` resource space and then runs
/// `allows_any_resource`.
///
/// Orphan handling matches the list filter exactly so that any config
/// an admin can *see* is also a config they can *delete*:
///
/// - **Unscoped admins** (membership `allowed_service_ids = None`) pass
///   through orphans because `allows_any_resource(&[])` returns `true`
///   for unscoped roles. This is what lets an admin remove a stale
///   org policy whose backing `UserService` was already deleted.
/// - **Scoped admins** (`allowed_service_ids = Some(...)`) deny orphans
///   because `allows_any_resource(&[])` returns `false` -- they have no
///   concrete claim to a service that doesn't exist.
///
/// Without the symmetric handling, an admin could land here from
/// `list_service_configs` (which uses `allows_any_resource` directly,
/// so unscoped sees orphans), see a stale config, and then hit
/// `404 NotFound` from a stricter delete path. The list and delete
/// paths must agree.
async fn ensure_service_config_in_scope(
    db: &mongodb::Database,
    access: &crate::services::org_service::OwnerAccess,
    owner_user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    // Direct owners (personal scope) skip the lookup entirely.
    if matches!(access, crate::services::org_service::OwnerAccess::Direct) {
        return Ok(());
    }
    let user_service_ids = scope_user_service_ids_for_config(db, owner_user_id, service_id).await?;
    if !access.allows_any_resource(&user_service_ids) {
        return Err(AppError::OrgRoleInsufficient(
            "your org admin role is scoped to other services and cannot manage this approval policy".to_string(),
        ));
    }
    Ok(())
}

/// Outcome of resolving the path `service_id` into the identifiers used
/// downstream: the storage key under which the `ServiceApprovalConfig`
/// lives, a denormalized display name, and (when available) the backing
/// `UserService` for UI annotation.
struct ApprovalTarget {
    /// Key used by `ServiceApprovalConfig.service_id` and by proxy
    /// approval resolution (matches
    /// `proxy_service::build_minimal_downstream_service`).
    effective_service_id: String,
    display_name: String,
    user_service_id: Option<String>,
    user_service_slug: Option<String>,
}

/// Translate a `service_id` path parameter into the storage key and
/// display info for the approval policy.
///
/// The caller may pass:
/// - A `UserService.id` owned by `owner_user_id` (primary path — matches
///   the identifier returned by `GET /api/v1/user-services`). We then
///   load the user service and collapse to `catalog_service_id` when
///   present so the policy lines up with proxy resolution.
/// - A catalog `DownstreamService.id` (legacy path, still accepted so
///   existing callers and pre-existing policies keep working).
///
/// Returns `NotFound` if neither lookup succeeds.
async fn resolve_approval_target(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: &str,
) -> AppResult<ApprovalTarget> {
    if let Some(user_service) = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc_ownership(owner_user_id, service_id))
        .await?
    {
        // Pick the display name by key space:
        //  - Catalog-backed: the policy is keyed by `catalog_service_id`
        //    and covers every sibling UserService reusing that catalog.
        //    Persist the catalog `DownstreamService.name` so the stored
        //    `service_name` is shared, stable, and never leaks a
        //    sibling's endpoint label to another scoped admin.
        //  - Custom: the policy is keyed by the UserService id itself,
        //    so the endpoint label (or slug fallback) is both accurate
        //    and uniquely attached to that one service.
        let (effective_service_id, display_name) =
            if let Some(ref catalog_id) = user_service.catalog_service_id {
                let catalog_name = db
                    .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                    .find_one(mongodb::bson::doc! { "_id": catalog_id })
                    .await?
                    .map(|s| s.name)
                    .unwrap_or_else(|| user_service.slug.clone());
                (catalog_id.clone(), catalog_name)
            } else {
                let endpoint_label = db
                    .collection::<UserEndpoint>(USER_ENDPOINTS)
                    .find_one(mongodb::bson::doc! { "_id": &user_service.endpoint_id })
                    .await?
                    .map(|ep| ep.label)
                    .unwrap_or_else(|| user_service.slug.clone());
                (user_service.id.clone(), endpoint_label)
            };
        return Ok(ApprovalTarget {
            effective_service_id,
            display_name,
            user_service_id: Some(user_service.id.clone()),
            user_service_slug: Some(user_service.slug),
        });
    }

    if let Some(service) = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(mongodb::bson::doc! { "_id": service_id, "is_active": true })
        .await?
    {
        return Ok(ApprovalTarget {
            effective_service_id: service.id,
            display_name: service.name,
            user_service_id: None,
            user_service_slug: Some(service.slug),
        });
    }

    Err(AppError::NotFound("Service not found".to_string()))
}

/// Look up the owner's matching `UserService` for an existing config's
/// `service_id`, filtered to the caller's `OwnerAccess` scope. The
/// returned annotation is surfaced in list responses and used by the UI
/// as the mutation id, so it must never name a service the caller is
/// not entitled to operate on.
///
/// Resolution order:
///  1. A direct `UserService.id` match (custom-service configs, or
///     policies written against the UserService id directly).
///  2. The newest active `UserService` sharing `catalog_service_id`
///     (catalog-backed policies cover every sibling; we just need a
///     representative).
///
/// `OwnerAccess::AsOrgAdmin` with a populated `allowed_service_ids`
/// restricts both paths — scoped admins never see siblings outside their
/// scope. `Direct` and unscoped `AsOrgAdmin` (`allowed_service_ids:
/// None`) accept any match. Returning `None` is safe: the UI falls back
/// to `service_id` for mutations, and the admin's outer scope check
/// already guarantees they control the policy.
async fn find_matching_user_service_for_config(
    db: &mongodb::Database,
    access: &crate::services::org_service::OwnerAccess,
    owner_user_id: &str,
    stored_service_id: &str,
) -> AppResult<Option<UserService>> {
    if let Some(us) = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc_ownership(owner_user_id, stored_service_id))
        .await?
        && access.allows_resource(&us.id)
    {
        return Ok(Some(us));
    }

    // Catalog-backed fallback: stream the siblings in newest-first order
    // and return the first one the access scope accepts. A scoped admin
    // whose allowed_service_ids doesn't cover any sibling gets `None`,
    // keeping their metadata sealed.
    use futures::TryStreamExt;
    let mut cursor = db
        .collection::<UserService>(USER_SERVICES)
        .find(mongodb::bson::doc! {
            "user_id": owner_user_id,
            "catalog_service_id": stored_service_id,
            "is_active": true,
        })
        .sort(mongodb::bson::doc! { "created_at": -1 })
        .await?;
    while let Some(us) = cursor.try_next().await? {
        if access.allows_resource(&us.id) {
            return Ok(Some(us));
        }
    }
    Ok(None)
}

// --- Per-service approval config handlers ---

/// GET /api/v1/approvals/service-configs
///
/// List per-service approval overrides for the current user, or for the
/// org passed via `?org_id=X` (caller must be admin of that org).
pub async fn list_service_configs(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ServiceApprovalConfigQuery>,
) -> AppResult<Json<ServiceApprovalConfigsResponse>> {
    let actor = auth_user.user_id.to_string();
    let (user_id, access) =
        resolve_service_config_owner(&state, &actor, query.org_id.as_deref()).await?;

    let configs = approval_service::list_service_approval_configs(&state.db, &user_id).await?;

    // Filter to the membership scope so a scoped admin only sees policies
    // for services they actually manage. Direct owners short-circuit and
    // see everything.
    let mut items: Vec<ServiceApprovalConfigItem> = Vec::with_capacity(configs.len());
    for c in configs {
        if !matches!(access, crate::services::org_service::OwnerAccess::Direct) {
            let user_service_ids =
                scope_user_service_ids_for_config(&state.db, &user_id, &c.service_id).await?;
            if !access.allows_any_resource(&user_service_ids) {
                continue;
            }
        }
        let matching =
            find_matching_user_service_for_config(&state.db, &access, &user_id, &c.service_id)
                .await?;
        let (user_service_id, user_service_slug) = match matching {
            Some(us) => (Some(us.id), Some(us.slug)),
            None => (None, None),
        };
        items.push(ServiceApprovalConfigItem {
            service_id: c.service_id,
            service_name: c.service_name,
            approval_required: c.approval_required,
            approval_mode: c.approval_mode,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
            user_service_id,
            user_service_slug,
        });
    }

    // For the caller's personal list, also collect the `service_id` set
    // of org policies that would dominate `resolve_org_aware_approval`
    // for any org they're a member of. The UI uses this to hide those
    // services from the personal Add-Override picker so users don't
    // create no-op overrides against org-routed proxy calls. The org-
    // scoped list path sees the org's own policies directly, so this
    // stays empty there.
    //
    // Each included id must honor the member's own scope:
    //   - Viewer role: the member can't proxy org services at all, so
    //     no org policy can dominate *their* calls — skip entirely.
    //   - Scoped member/admin (`allowed_service_ids: Some(...)`): only
    //     include policies whose UserService-space translation
    //     intersects the member's allowed set. Otherwise we'd expose
    //     raw `service_id`s (especially custom-service ids, which are
    //     just `UserService.id`s) for resources the member was never
    //     granted access to — a violation of the org scope invariant.
    //   - Unscoped (`allowed_service_ids: None`): every org policy
    //     applies.
    let dominant_org_policies = if query.org_id.is_some() {
        Vec::new()
    } else {
        let mut out: Vec<DominantOrgPolicy> = Vec::new();
        let memberships =
            crate::services::org_service::list_memberships_for_member(&state.db, &actor, false)
                .await?;
        for m in memberships {
            if !m.role.can_proxy() {
                continue;
            }
            let org_configs =
                approval_service::list_service_approval_configs(&state.db, &m.org_user_id).await?;
            for c in org_configs {
                let in_scope = match &m.allowed_service_ids {
                    None => true,
                    Some(allowed) => {
                        let user_service_ids = scope_user_service_ids_for_config(
                            &state.db,
                            &m.org_user_id,
                            &c.service_id,
                        )
                        .await?;
                        user_service_ids
                            .iter()
                            .any(|id| allowed.iter().any(|a| a == id))
                    }
                };
                if in_scope {
                    out.push(DominantOrgPolicy {
                        org_id: m.org_user_id.clone(),
                        service_id: c.service_id,
                    });
                }
            }
        }
        out
    };

    Ok(Json(ServiceApprovalConfigsResponse {
        configs: items,
        dominant_org_policies,
    }))
}

/// PUT /api/v1/approvals/service-configs/{service_id}
///
/// Set a per-service approval override. Creates or updates. Pass
/// `?org_id=X` to set the policy on org X's behalf (caller must be admin).
///
/// The path `service_id` accepts either a `UserService.id` (the natural
/// key that users interact with via `/user-services` and the unified keys
/// UI — including custom services that have no catalog backing) or a
/// legacy catalog `DownstreamService.id` (kept working so existing API
/// consumers and pre-existing policies don't break). In both cases the
/// stored config is keyed by the *effective* service id that proxy
/// approval resolution uses (the catalog id when the user service is
/// catalog-backed, otherwise the user service id itself), so a single
/// policy naturally covers all user services reusing the same catalog
/// entry while custom services get their own isolated policy.
pub async fn set_service_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Query(query): Query<ServiceApprovalConfigQuery>,
    Json(body): Json<SetServiceApprovalConfigRequest>,
) -> AppResult<Json<ServiceApprovalConfigItem>> {
    let actor = auth_user.user_id.to_string();
    let (user_id, access) =
        resolve_service_config_owner(&state, &actor, query.org_id.as_deref()).await?;

    if body.approval_required.is_none() && body.approval_mode.is_none() {
        return Err(AppError::ValidationError(
            "At least one of approval_required or approval_mode must be provided".to_string(),
        ));
    }

    let target = resolve_approval_target(&state.db, &user_id, &service_id).await?;

    // Reject a scoped admin targeting a specific UserService outside
    // their `allowed_service_ids`, even when a sibling for the same
    // catalog is in scope. The catalog-level check below (effective
    // service id) would otherwise pass via the in-scope sibling and the
    // response/audit would leak the out-of-scope service id/slug.
    if let Some(ref us_id) = target.user_service_id
        && !access.allows_resource(us_id)
    {
        return Err(AppError::OrgRoleInsufficient(
            "your org admin role is scoped to other services and cannot manage this approval policy".to_string(),
        ));
    }

    // Per-service scope check: scoped admins can only manage policies for
    // services in their `allowed_service_ids` set.
    ensure_service_config_in_scope(&state.db, &access, &user_id, &target.effective_service_id)
        .await?;

    let config = approval_service::set_service_approval_config(
        &state.db,
        &user_id,
        &target.effective_service_id,
        &target.display_name,
        body.approval_required,
        body.approval_mode.as_ref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "service_approval_config_set".to_string(),
        Some(serde_json::json!({
            "service_id": target.effective_service_id,
            "service_name": target.display_name,
            "user_service_id": target.user_service_id,
            "policy_owner_user_id": user_id,
            "approval_required": config.approval_required,
            "approval_mode": config.approval_mode.as_str(),
        })),
        None,
        None,
        None,
        None,
    );

    Ok(Json(ServiceApprovalConfigItem {
        service_id: config.service_id,
        service_name: config.service_name,
        approval_required: config.approval_required,
        approval_mode: config.approval_mode,
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
        user_service_id: target.user_service_id,
        user_service_slug: target.user_service_slug,
    }))
}

/// DELETE /api/v1/approvals/service-configs/{service_id}
///
/// Remove a per-service approval override (revert to global default).
/// Pass `?org_id=X` to remove the policy on org X's behalf (admin only).
pub async fn delete_service_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Query(query): Query<ServiceApprovalConfigQuery>,
) -> AppResult<Json<MessageResponse>> {
    let actor = auth_user.user_id.to_string();
    let (user_id, access) =
        resolve_service_config_owner(&state, &actor, query.org_id.as_deref()).await?;

    // Resolve the path id to the stored-key space up-front so callers can
    // pass either a UserService.id (what clients see on /user-services) or
    // the raw effective service id (what's stored in the config). Falls
    // back to the raw id when the user service has been deleted — that's
    // an orphan cleanup case, and the legacy catalog lookup inside
    // `resolve_approval_target` keeps those reachable.
    let (effective_service_id, user_service_id) =
        match resolve_approval_target(&state.db, &user_id, &service_id).await {
            Ok(target) => (target.effective_service_id, target.user_service_id),
            Err(AppError::NotFound(_)) => (service_id.clone(), None),
            Err(e) => return Err(e),
        };

    // Per-service scope check, same as set_service_config.
    ensure_service_config_in_scope(&state.db, &access, &user_id, &effective_service_id).await?;

    approval_service::delete_service_approval_config(&state.db, &user_id, &effective_service_id)
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "service_approval_config_deleted".to_string(),
        Some(serde_json::json!({
            "service_id": effective_service_id,
            "user_service_id": user_service_id,
            "policy_owner_user_id": user_id,
        })),
        None,
        None,
        None,
        None,
    );

    Ok(Json(MessageResponse {
        message: "Per-service approval config removed".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::approval_request::ApprovalRequest;
    use chrono::Utc;

    fn sample_request(user_id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            service_name: "OpenAI".to_string(),
            service_slug: "openai".to_string(),
            requester_type: "service_account".to_string(),
            requester_id: "sa_123".to_string(),
            requester_label: Some("CI bot".to_string()),
            operation_summary: "proxy:POST /v1/chat/completions".to_string(),
            action_description: Some("POST /v1/chat/completions (model: gpt-4)".to_string()),
            tool_name: None,
            tool_call_id: None,
            tool_arguments: None,
            is_destructive: None,
            approval_mode: ApprovalMode::PerRequest,
            status: "pending".to_string(),
            idempotency_key: "idem_123".to_string(),
            notification_channel: Some("fcm".to_string()),
            telegram_message_id: None,
            telegram_chat_id: None,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
            decided_at: None,
            decision_channel: None,
            decision_idempotency_key: None,
            notify_user_ids: vec![],
            from_org_policy: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn ensure_request_owned_by_user_allows_owner() {
        let result = ensure_request_owned_by_user("user_1", "user_1");
        assert!(result.is_ok());
    }

    #[test]
    fn ensure_request_owned_by_user_rejects_non_owner() {
        let result = ensure_request_owned_by_user("user_1", "user_2");
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[test]
    fn to_approval_request_item_maps_core_fields() {
        let request = sample_request("user_1");
        let expected_id = request.id.clone();
        let expected_service = request.service_name.clone();
        let expected_status = request.status.clone();

        let item = to_approval_request_item(request, None);

        assert_eq!(item.id, expected_id);
        assert_eq!(item.service_name, expected_service);
        assert_eq!(item.status, expected_status);
        assert!(item.created_at.contains('T'));
        // Proxy approvals have no tool fields
        assert!(item.tool_name.is_none());
        assert!(item.is_destructive.is_none());
    }

    #[test]
    fn to_approval_request_item_includes_tool_fields() {
        let mut request = sample_request("user_1");
        request.tool_name = Some("invoke_service".to_string());
        request.tool_call_id = Some("call_abc".to_string());
        request.tool_arguments = Some(r#"{"service_id":"svc_1"}"#.to_string());
        request.is_destructive = Some(true);

        let item = to_approval_request_item(request, None);

        assert_eq!(item.tool_name.as_deref(), Some("invoke_service"));
        assert_eq!(item.tool_call_id.as_deref(), Some("call_abc"));
        assert!(item.tool_arguments.is_some());
        assert_eq!(item.is_destructive, Some(true));
    }

    #[test]
    fn to_approval_request_item_stamps_org_fields_when_from_org_policy() {
        let mut request = sample_request("org_abc");
        request.from_org_policy = true;

        let item = to_approval_request_item(request, Some("Acme Inc.".to_string()));

        assert!(item.from_org_policy);
        assert_eq!(item.org_id.as_deref(), Some("org_abc"));
        assert_eq!(item.org_name.as_deref(), Some("Acme Inc."));
    }

    #[test]
    fn to_approval_request_item_omits_org_fields_for_personal_requests() {
        let request = sample_request("user_1");
        // Even if a caller mistakenly passes an org_name for a personal
        // request, the mapper must not stamp org fields — from_org_policy
        // is the authoritative signal.
        let item = to_approval_request_item(request, Some("Irrelevant".to_string()));

        assert!(!item.from_org_policy);
        assert!(item.org_id.is_none());
        assert!(item.org_name.is_none());
    }

    #[test]
    fn to_approval_grant_item_stamps_org_fields_when_org_scoped() {
        use crate::models::approval_grant::ApprovalGrant;
        let grant = ApprovalGrant {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "org_abc".to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            service_name: "OpenAI".to_string(),
            requester_type: "service_account".to_string(),
            requester_id: "sa_123".to_string(),
            requester_label: Some("CI bot".to_string()),
            approval_request_id: uuid::Uuid::new_v4().to_string(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(30),
            revoked: false,
            org_scoped: true,
        };

        let item = to_approval_grant_item(grant, Some("Acme Inc.".to_string()));

        assert!(item.org_scoped);
        assert_eq!(item.org_id.as_deref(), Some("org_abc"));
        assert_eq!(item.org_name.as_deref(), Some("Acme Inc."));
    }

    #[test]
    fn to_approval_grant_item_omits_org_fields_for_personal_grants() {
        use crate::models::approval_grant::ApprovalGrant;
        let grant = ApprovalGrant {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "user_1".to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            service_name: "OpenAI".to_string(),
            requester_type: "service_account".to_string(),
            requester_id: "sa_123".to_string(),
            requester_label: None,
            approval_request_id: uuid::Uuid::new_v4().to_string(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(30),
            revoked: false,
            org_scoped: false,
        };

        let item = to_approval_grant_item(grant, Some("Leaked name".to_string()));

        assert!(!item.org_scoped);
        assert!(item.org_id.is_none());
        assert!(item.org_name.is_none());
    }

    #[test]
    fn create_tool_approval_request_deserializes() {
        let json = r#"{
            "tool_name": "invoke_service",
            "tool_call_id": "call_123",
            "arguments": "{\"key\":\"val\"}",
            "is_destructive": true,
            "approval_mode": "alwaysrequire"
        }"#;
        let parsed: CreateToolApprovalRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.tool_name, "invoke_service");
        assert_eq!(parsed.tool_call_id.as_deref(), Some("call_123"));
        assert_eq!(parsed.is_destructive, Some(true));
    }

    #[test]
    fn create_tool_approval_request_minimal() {
        let json = r#"{"tool_name": "list_services"}"#;
        let parsed: CreateToolApprovalRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.tool_name, "list_services");
        assert!(parsed.tool_call_id.is_none());
        assert!(parsed.arguments.is_none());
        assert!(parsed.is_destructive.is_none());
    }
}
