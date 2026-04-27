//! HTTP handlers for org management.
//!
//! Implements the API table from `docs/ORG_MODEL_IMPLEMENTATION_PLAN.md`:
//! - Create / read / update / delete an org user
//! - List orgs the caller belongs to
//! - Member management (list, change role, revoke)
//! - One-time invite issue / list / cancel / redeem
//!
//! All write operations on a specific org require admin role on that org.
//! Read operations require any active membership. Org creation is open to
//! any authenticated person user.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::Validate;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::org_invite::OrgInvite;
use crate::models::org_membership::{MemberScopeSource, OrgMembership, OrgRole};
use crate::models::user::User;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, org_invite_service, org_role_scope_service, org_service};

/// Maximum invite TTL accepted by `POST /orgs/{id}/invites` and the
/// matching CLI command. Mirrors the 30-day bound the web schema enforces
/// (`frontend/src/schemas/orgs.ts::createInviteRequestSchema`). Bound at
/// the API boundary so non-web callers can't slip an out-of-range integer
/// past `chrono::Duration::hours`, which panics on values that overflow
/// the internal representation.
const ORG_INVITE_MAX_TTL_HOURS: i64 = 24 * 30;

// ─────────────────────────────────────────────────────────────────────────────
// Wire types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema, Validate)]
pub struct CreateOrgRequest {
    #[validate(length(min = 1, max = 128, message = "display_name must be 1-128 characters"))]
    pub display_name: String,
    #[validate(email(message = "contact_email must be a valid email"))]
    pub contact_email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, Validate)]
pub struct UpdateOrgRequest {
    #[validate(length(min = 1, max = 128, message = "display_name must be 1-128 characters"))]
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Update the org's contact email. Pass an empty string to clear back to
    /// the synthetic placeholder used when no contact email was provided at
    /// creation time. Accepts any RFC-compliant email otherwise.
    #[serde(default)]
    pub contact_email: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMemberRequest {
    pub user_id: String,
    pub role: OrgRoleWire,
    /// Service scope mode. If omitted, new members inherit from the role
    /// default unless `allowed_service_ids` is provided, in which case the
    /// request is treated as an explicit override for backwards
    /// compatibility with older callers.
    pub scope_source: Option<MemberScopeSourceWire>,
    #[serde(default)]
    pub allowed_service_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMemberRequest {
    pub role: Option<OrgRoleWire>,
    /// Sending `scope_source` always rewrites `allowed_service_ids`:
    /// - `"inherit"` clears the stored list (role scope applies instead).
    /// - `"override"` with no `allowed_service_ids` field clears the stored
    ///   list and grants full-access override.
    ///
    /// If you want to keep an existing override list, send it explicitly
    /// alongside `"override"`. Clients should always send both fields
    /// together — omit `scope_source` only when you are touching the
    /// scope list under the caller's existing mode.
    pub scope_source: Option<MemberScopeSourceWire>,
    /// Pass `null` to clear the scope (full access). Pass an array to restrict.
    ///
    /// The custom deserializer distinguishes an absent field (leave
    /// existing scope untouched: `None`) from an explicit `null` (clear
    /// the scope: `Some(None)`). Without it, serde's default
    /// `Option<Option<T>>` deserialization collapses both cases to outer
    /// `None`, so `{"allowed_service_ids": null}` is silently ignored
    /// (issue #363).
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub allowed_service_ids: Option<Option<Vec<String>>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateInviteRequest {
    pub role: OrgRoleWire,
    /// Service scope mode applied when the invite is redeemed. If omitted,
    /// the invite inherits from the role default unless `allowed_service_ids`
    /// is provided, in which case it becomes an explicit override.
    pub scope_source: Option<MemberScopeSourceWire>,
    #[serde(default)]
    pub allowed_service_ids: Option<Vec<String>>,
    /// Time-to-live in hours. Defaults to 24.
    #[serde(default)]
    pub ttl_hours: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetPrimaryOrgRequest {
    /// Pass an org_user_id to set, or `null` to clear.
    pub primary_org_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgResponse {
    pub id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// User-visible contact email. `None` when the org was created without an
    /// explicit contact email (the backend stores a synthetic
    /// `org-<uuid>@nyxid.local` placeholder, which is intentionally hidden
    /// from user-facing surfaces).
    pub contact_email: Option<String>,
    pub created_at: String,
    /// Caller's role in this org. Always present in single-org responses.
    pub your_role: OrgRoleWire,
    pub member_count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgListResponse {
    pub orgs: Vec<OrgListItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgListItem {
    pub id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// See [`OrgResponse::contact_email`].
    pub contact_email: Option<String>,
    pub your_role: OrgRoleWire,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemberResponse {
    pub membership_id: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub role: OrgRoleWire,
    pub scope_source: MemberScopeSourceWire,
    pub allowed_service_ids: Option<Vec<String>>,
    pub effective_allowed_service_ids: Option<Vec<String>>,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemberListResponse {
    pub members: Vec<MemberResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteResponse {
    pub id: String,
    pub nonce: String,
    pub role: OrgRoleWire,
    pub scope_source: MemberScopeSourceWire,
    pub allowed_service_ids: Option<Vec<String>>,
    pub created_by: String,
    pub expires_at: String,
    pub redeemed_by: Option<String>,
    /// Email of the user who redeemed the invite. Populated when
    /// `redeemed_by` is set. Lets the admin UI show "Used by foo@bar"
    /// without a per-row user lookup (issue #409).
    pub redeemed_by_email: Option<String>,
    /// Display name of the redeeming user, if set.
    pub redeemed_by_display_name: Option<String>,
    pub redeemed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteListResponse {
    pub invites: Vec<InviteResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RedeemInviteResponse {
    pub org_id: String,
    pub role: OrgRoleWire,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrgRoleWire {
    Admin,
    Member,
    Viewer,
}

impl From<OrgRole> for OrgRoleWire {
    fn from(role: OrgRole) -> Self {
        match role {
            OrgRole::Admin => Self::Admin,
            OrgRole::Member => Self::Member,
            OrgRole::Viewer => Self::Viewer,
        }
    }
}

impl From<OrgRoleWire> for OrgRole {
    fn from(role: OrgRoleWire) -> Self {
        match role {
            OrgRoleWire::Admin => Self::Admin,
            OrgRoleWire::Member => Self::Member,
            OrgRoleWire::Viewer => Self::Viewer,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemberScopeSourceWire {
    Inherit,
    Override,
}

impl From<MemberScopeSource> for MemberScopeSourceWire {
    fn from(source: MemberScopeSource) -> Self {
        match source {
            MemberScopeSource::Inherit => Self::Inherit,
            MemberScopeSource::Override => Self::Override,
        }
    }
}

impl From<MemberScopeSourceWire> for MemberScopeSource {
    fn from(source: MemberScopeSourceWire) -> Self {
        match source {
            MemberScopeSourceWire::Inherit => Self::Inherit,
            MemberScopeSourceWire::Override => Self::Override,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Look up a user by id. Used for member-list enrichment.
async fn fetch_user(db: &mongodb::Database, user_id: &str) -> AppResult<Option<User>> {
    use crate::models::user::COLLECTION_NAME as USERS;
    use mongodb::bson::doc;
    let row = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?;
    Ok(row)
}

async fn membership_to_response(
    db: &mongodb::Database,
    m: OrgMembership,
    member: Option<&User>,
) -> AppResult<MemberResponse> {
    let effective_allowed_service_ids =
        org_role_scope_service::effective_scope_for_membership(db, &m).await?;
    Ok(MemberResponse {
        membership_id: m.id,
        user_id: m.member_user_id,
        display_name: member.and_then(|u| u.display_name.clone()),
        email: member.map(|u| u.email.clone()),
        role: m.role.into(),
        scope_source: m.scope_source.into(),
        allowed_service_ids: m.allowed_service_ids,
        effective_allowed_service_ids,
        created_at: m.created_at.to_rfc3339(),
        revoked_at: m.revoked_at.map(|d| d.to_rfc3339()),
    })
}

fn invite_to_response(invite: OrgInvite, redeemer: Option<&User>) -> InviteResponse {
    InviteResponse {
        id: invite.id,
        nonce: invite.nonce,
        role: invite.role.into(),
        scope_source: invite.scope_source.into(),
        allowed_service_ids: invite.allowed_service_ids,
        created_by: invite.created_by,
        expires_at: invite.expires_at.to_rfc3339(),
        redeemed_by: invite.redeemed_by,
        redeemed_by_email: redeemer.map(|u| u.email.clone()),
        redeemed_by_display_name: redeemer.and_then(|u| u.display_name.clone()),
        redeemed_at: invite.redeemed_at.map(|d| d.to_rfc3339()),
        created_at: invite.created_at.to_rfc3339(),
    }
}

fn resolve_scope_source_for_create(
    explicit: Option<MemberScopeSourceWire>,
    allowed_service_ids: Option<&Vec<String>>,
) -> MemberScopeSource {
    explicit
        .map(Into::into)
        .unwrap_or(if allowed_service_ids.is_some() {
            MemberScopeSource::Override
        } else {
            MemberScopeSource::Inherit
        })
}

fn resolve_scope_source_for_update(
    explicit: Option<MemberScopeSourceWire>,
    allowed_service_ids: Option<&Option<Vec<String>>>,
) -> Option<MemberScopeSource> {
    explicit
        .map(Into::into)
        .or_else(|| allowed_service_ids.map(|_| MemberScopeSource::Override))
}

/// Batch-fetch the users referenced by `redeemed_by` across a list of
/// invites. Uses a single `$in` query so rendering the invites tab stays
/// O(1) round-trip regardless of list size (issue #409).
async fn fetch_invite_redeemers(
    db: &mongodb::Database,
    invites: &[OrgInvite],
) -> AppResult<std::collections::HashMap<String, User>> {
    use crate::models::user::COLLECTION_NAME as USERS;
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use std::collections::HashMap;

    let ids: Vec<&str> = invites
        .iter()
        .filter_map(|i| i.redeemed_by.as_deref())
        .collect();

    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let cursor = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": &ids } })
        .await?;
    let users: Vec<User> = cursor.try_collect().await?;
    Ok(users.into_iter().map(|u| (u.id.clone(), u)).collect())
}

/// Reject if the actor is not admin of this org.
///
/// Verifies the org exists first so a non-existent id returns
/// `OrgNotFound` (404) rather than masking that as a role/membership
/// error. Without that check, a caller poking at arbitrary UUIDs gets
/// `OrgRoleInsufficient` for every id and cannot tell "org does not
/// exist" from "I'm not an admin of this real org".
pub(crate) async fn require_org_admin(
    db: &mongodb::Database,
    actor_user_id: &str,
    org_user_id: &str,
) -> AppResult<()> {
    let _ = org_service::get_org_user(db, org_user_id).await?;
    if !org_service::is_admin(db, actor_user_id, org_user_id).await? {
        return Err(AppError::OrgRoleInsufficient(
            "admin role required for this operation".to_string(),
        ));
    }
    Ok(())
}

/// Reject if the actor is not any kind of active member of this org.
///
/// Verifies the org exists first so a non-existent id returns
/// `OrgNotFound` (404) instead of `OrgMembershipRequired` (403). This
/// lets clients distinguish "org does not exist" from "I'm not a
/// member of this real org" (issue #359).
async fn require_org_member(
    db: &mongodb::Database,
    actor_user_id: &str,
    org_user_id: &str,
) -> AppResult<OrgMembership> {
    let _ = org_service::get_org_user(db, org_user_id).await?;
    let m = org_service::get_active_membership(db, org_user_id, actor_user_id).await?;
    m.ok_or(AppError::OrgMembershipRequired)
}

/// Reject if removing the given member's admin role would leave the org
/// with zero active admins. The check counts admins *other than* the
/// target so a single-admin org cannot dissolve itself by self-demote or
/// self-revocation. Admins who really want to dispose of the org must go
/// through `DELETE /orgs/{id}`, which cascades memberships once live
/// resources are cleared.
async fn ensure_not_last_admin(
    db: &mongodb::Database,
    org_user_id: &str,
    target_member_user_id: &str,
) -> AppResult<()> {
    let admins = org_service::list_admin_user_ids(db, org_user_id).await?;
    let other_admins = admins
        .iter()
        .filter(|id| id.as_str() != target_member_user_id)
        .count();
    if other_admins == 0 {
        return Err(AppError::Conflict(
            "cannot remove or demote the last active admin of this org. Promote another member to admin first, or delete the org via DELETE /orgs/{id}.".to_string(),
        ));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers: Org CRUD
// ─────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/orgs
///
/// Create a new org. Caller becomes the first admin member.
///
/// The actor must be a person user. The `/orgs` route is in the
/// human-only router (`api_v1_human_only`) which rejects delegated and
/// service-account tokens, but it still allows API-key auth -- and an
/// API key may be owned by an org. We reject those upfront so we never
/// get to the membership-create step that would otherwise leave a
/// freshly inserted org user with zero admins.
///
/// As defense-in-depth, the handler ALSO rolls back the org user insert
/// if the membership-create step fails for any other reason. Without
/// that, a partial failure would leave a zero-admin org behind that
/// nobody could reach -- delete_org also requires a current admin.
pub async fn create_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateOrgRequest>,
) -> AppResult<(StatusCode, Json<OrgResponse>)> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let actor = auth_user.user_id.to_string();

    // Reject org-owned actors (API keys whose owner is an org user).
    let actor_user = state
        .db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(mongodb::bson::doc! { "_id": &actor })
        .await?
        .ok_or_else(|| AppError::Unauthorized("actor user not found".to_string()))?;
    crate::services::auth_service::ensure_person_user(&actor_user)?;

    let org = org_service::create_org_user(
        &state.db,
        &body.display_name,
        body.contact_email.as_deref(),
        body.avatar_url.as_deref(),
    )
    .await?;

    // Add the creator as Admin. If this fails (network blip, race, etc.)
    // roll back the org user insert so we never leave behind an org with
    // no admins. The membership-create call is the only step between the
    // org user insert and the audit log; if it succeeds the org is
    // recoverable, if it fails we restore the pre-insert state.
    let membership = match org_service::create_membership(
        &state.db,
        &org.id,
        &actor,
        OrgRole::Admin,
        MemberScopeSource::Inherit,
        None,
    )
    .await
    {
        Ok(m) => m,
        Err(create_err) => {
            if let Err(rollback_err) = state
                .db
                .collection::<User>(crate::models::user::COLLECTION_NAME)
                .delete_one(mongodb::bson::doc! { "_id": &org.id })
                .await
            {
                tracing::error!(
                    org_user_id = %org.id,
                    error = %rollback_err,
                    "Failed to roll back org user after membership-create failure; manual cleanup required"
                );
            }
            return Err(create_err);
        }
    };

    audit_service::log_async(
        state.db.clone(),
        Some(actor.clone()),
        "org_created".to_string(),
        Some(serde_json::json!({
            "org_user_id": org.id,
            "display_name": body.display_name,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    let contact_email = org_service::contact_email_for_display(&org);
    Ok((
        StatusCode::CREATED,
        Json(OrgResponse {
            id: org.id,
            display_name: org.display_name,
            avatar_url: org.avatar_url,
            contact_email,
            created_at: org.created_at.to_rfc3339(),
            your_role: membership.role.into(),
            member_count: 1,
        }),
    ))
}

/// GET /api/v1/orgs
///
/// List orgs the caller is an active member of.
pub async fn list_orgs(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<OrgListResponse>> {
    let actor = auth_user.user_id.to_string();
    let memberships = org_service::list_memberships_for_member(&state.db, &actor, false).await?;

    let mut items = Vec::with_capacity(memberships.len());
    for m in memberships {
        if let Ok(org) = org_service::get_org_user(&state.db, &m.org_user_id).await {
            let contact_email = org_service::contact_email_for_display(&org);
            items.push(OrgListItem {
                id: org.id,
                display_name: org.display_name,
                avatar_url: org.avatar_url,
                contact_email,
                your_role: m.role.into(),
                created_at: org.created_at.to_rfc3339(),
            });
        }
    }

    Ok(Json(OrgListResponse { orgs: items }))
}

/// GET /api/v1/orgs/{org_id}
pub async fn get_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
) -> AppResult<Json<OrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let membership = require_org_member(&state.db, &actor, &org_id).await?;
    let org = org_service::get_org_user(&state.db, &org_id).await?;

    let members = org_service::list_members_for_org(&state.db, &org_id, false).await?;
    let contact_email = org_service::contact_email_for_display(&org);

    Ok(Json(OrgResponse {
        id: org.id,
        display_name: org.display_name,
        avatar_url: org.avatar_url,
        contact_email,
        created_at: org.created_at.to_rfc3339(),
        your_role: membership.role.into(),
        member_count: members.len() as u64,
    }))
}

/// PATCH /api/v1/orgs/{org_id}
pub async fn update_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
    Json(body): Json<UpdateOrgRequest>,
) -> AppResult<Json<OrgResponse>> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    // Validate contact_email when non-empty; empty string clears back to the
    // synthetic placeholder (see `org_service::update_org_user`). Uses the
    // same `validator::ValidateEmail` path as `CreateOrgRequest` so the
    // accept/reject surface matches the create flow.
    if let Some(ref email) = body.contact_email {
        let trimmed = email.trim();
        if !trimmed.is_empty() && !validator::ValidateEmail::validate_email(&trimmed) {
            return Err(AppError::ValidationError(
                "contact_email must be a valid email".to_string(),
            ));
        }
    }

    let org = org_service::update_org_user(
        &state.db,
        &org_id,
        body.display_name.as_deref(),
        body.avatar_url.as_deref(),
        body.contact_email.as_deref(),
    )
    .await?;

    let membership = require_org_member(&state.db, &actor, &org_id).await?;
    let members = org_service::list_members_for_org(&state.db, &org_id, false).await?;
    let contact_email = org_service::contact_email_for_display(&org);

    let contact_email_changed = body.contact_email.is_some();
    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_updated".to_string(),
        Some(serde_json::json!({
            "org_user_id": org_id,
            "contact_email_changed": contact_email_changed,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(Json(OrgResponse {
        id: org.id,
        display_name: org.display_name,
        avatar_url: org.avatar_url,
        contact_email,
        created_at: org.created_at.to_rfc3339(),
        your_role: membership.role.into(),
        member_count: members.len() as u64,
    }))
}

/// DELETE /api/v1/orgs/{org_id}
pub async fn delete_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    org_service::delete_org_user(&state.db, &org_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_deleted".to_string(),
        Some(serde_json::json!({ "org_user_id": org_id })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers: Members
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/orgs/{org_id}/members
pub async fn list_members(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
) -> AppResult<Json<MemberListResponse>> {
    let actor = auth_user.user_id.to_string();
    let _ = require_org_member(&state.db, &actor, &org_id).await?;

    let memberships = org_service::list_members_for_org(&state.db, &org_id, false).await?;

    let mut members = Vec::with_capacity(memberships.len());
    for m in memberships {
        let user = fetch_user(&state.db, &m.member_user_id).await?;
        members.push(membership_to_response(&state.db, m, user.as_ref()).await?);
    }

    Ok(Json(MemberListResponse { members }))
}

/// POST /api/v1/orgs/{org_id}/members
///
/// Direct add (without invite). Useful for admin tooling. End-user flows
/// should prefer the invite path so the new member explicitly opts in.
pub async fn add_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
    Json(body): Json<AddMemberRequest>,
) -> AppResult<(StatusCode, Json<MemberResponse>)> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    let scope_source =
        resolve_scope_source_for_create(body.scope_source, body.allowed_service_ids.as_ref());
    let membership = org_service::create_membership(
        &state.db,
        &org_id,
        &body.user_id,
        body.role.into(),
        scope_source,
        body.allowed_service_ids,
    )
    .await?;

    let user = fetch_user(&state.db, &body.user_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_member_added".to_string(),
        Some(serde_json::json!({
            "org_user_id": org_id,
            "member_user_id": body.user_id,
            "role": body.role,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok((
        StatusCode::CREATED,
        Json(membership_to_response(&state.db, membership, user.as_ref()).await?),
    ))
}

/// PATCH /api/v1/orgs/{org_id}/members/{member_id}
pub async fn update_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((org_id, member_id)): Path<(String, String)>,
    Json(body): Json<UpdateMemberRequest>,
) -> AppResult<Json<MemberResponse>> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    // Find the membership row by org+member to get its id.
    let current = org_service::get_active_membership(&state.db, &org_id, &member_id)
        .await?
        .ok_or_else(|| AppError::NotFound("active membership not found".to_string()))?;

    // Last-admin guard: refuse to demote the last active admin away from
    // the Admin role. Without this, an admin could brick the org by
    // self-demoting (DELETE /orgs/{id} also requires an admin, so the org
    // -- and any resources it still owns -- becomes unrecoverable).
    if let Some(new_role_wire) = body.role.as_ref() {
        let new_role: OrgRole = (*new_role_wire).into();
        if current.role == OrgRole::Admin && new_role != OrgRole::Admin {
            ensure_not_last_admin(&state.db, &org_id, &member_id).await?;
        }
    }

    let scope_source =
        resolve_scope_source_for_update(body.scope_source, body.allowed_service_ids.as_ref());
    let updated = org_service::update_membership(
        &state.db,
        &current.id,
        body.role.map(Into::into),
        scope_source,
        body.allowed_service_ids,
    )
    .await?;

    let user = fetch_user(&state.db, &member_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_member_updated".to_string(),
        Some(serde_json::json!({
            "org_user_id": org_id,
            "member_user_id": member_id,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(Json(
        membership_to_response(&state.db, updated, user.as_ref()).await?,
    ))
}

/// DELETE /api/v1/orgs/{org_id}/members/{member_id}
pub async fn remove_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((org_id, member_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    // Last-admin guard: revoking the last active admin would leave the
    // org unrecoverable -- DELETE /orgs/{id} also requires an admin, so
    // any owned resources (services, keys, policies) get stranded.
    // Admins who want to dissolve an org must `DELETE /orgs/{id}` first,
    // which cascades memberships once the live blockers are clear.
    let target = org_service::get_active_membership(&state.db, &org_id, &member_id)
        .await?
        .ok_or_else(|| AppError::NotFound("active membership not found".to_string()))?;
    if target.role == OrgRole::Admin {
        ensure_not_last_admin(&state.db, &org_id, &member_id).await?;
    }

    org_service::revoke_membership(&state.db, &org_id, &member_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_member_revoked".to_string(),
        Some(serde_json::json!({
            "org_user_id": org_id,
            "member_user_id": member_id,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers: Invites
// ─────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/orgs/{org_id}/invites
pub async fn create_invite(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
    Json(body): Json<CreateInviteRequest>,
) -> AppResult<(StatusCode, Json<InviteResponse>)> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    // Bound `ttl_hours` server-side. The web schema caps it at 30 days but
    // raw API / CLI callers reach this without that gate, and
    // `chrono::Duration::hours` panics on i64 values that don't fit, so a
    // hostile or accidental large integer would crash the process. Reject
    // anything outside (0, 720] hours with a structured error.
    let ttl = match body.ttl_hours {
        None => None,
        Some(h) if (1..=ORG_INVITE_MAX_TTL_HOURS).contains(&h) => Some(chrono::Duration::hours(h)),
        Some(_) => {
            return Err(AppError::ValidationError(format!(
                "ttl_hours must be between 1 and {ORG_INVITE_MAX_TTL_HOURS} (30 days)"
            )));
        }
    };

    let scope_source =
        resolve_scope_source_for_create(body.scope_source, body.allowed_service_ids.as_ref());
    let invite = org_invite_service::create_invite(
        &state.db,
        &org_id,
        &actor,
        body.role.into(),
        scope_source,
        body.allowed_service_ids,
        ttl,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_invite_created".to_string(),
        Some(serde_json::json!({
            "org_user_id": org_id,
            "invite_id": invite.id,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    // A freshly created invite has no redeemer yet, so pass `None`.
    Ok((StatusCode::CREATED, Json(invite_to_response(invite, None))))
}

/// GET /api/v1/orgs/{org_id}/invites
pub async fn list_invites(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(org_id): Path<String>,
) -> AppResult<Json<InviteListResponse>> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    let invites = org_invite_service::list_invites_for_org(&state.db, &org_id).await?;
    let redeemers = fetch_invite_redeemers(&state.db, &invites).await?;

    let out = invites
        .into_iter()
        .map(|i| {
            let redeemer = i.redeemed_by.as_deref().and_then(|id| redeemers.get(id));
            invite_to_response(i, redeemer)
        })
        .collect();

    Ok(Json(InviteListResponse { invites: out }))
}

/// DELETE /api/v1/orgs/{org_id}/invites/{invite_id}
pub async fn cancel_invite(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((org_id, invite_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    require_org_admin(&state.db, &actor, &org_id).await?;

    org_invite_service::cancel_invite(&state.db, &org_id, &invite_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_invite_cancelled".to_string(),
        Some(serde_json::json!({
            "org_user_id": org_id,
            "invite_id": invite_id,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/orgs/join/{nonce}
pub async fn redeem_invite(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(nonce): Path<String>,
) -> AppResult<Json<RedeemInviteResponse>> {
    let actor = auth_user.user_id.to_string();
    let membership = org_invite_service::redeem_invite(&state.db, &nonce, &actor).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "org_member_joined".to_string(),
        Some(serde_json::json!({
            "org_user_id": membership.org_user_id,
            "membership_id": membership.id,
            "role": membership.role,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(Json(RedeemInviteResponse {
        org_id: membership.org_user_id,
        role: membership.role.into(),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler: primary_org_id
// ─────────────────────────────────────────────────────────────────────────────

/// PATCH /api/v1/users/me/primary-org
///
/// Set or clear the caller's `primary_org_id`. Used as a tiebreaker when
/// the user belongs to multiple orgs that share a service.
pub async fn set_primary_org(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<SetPrimaryOrgRequest>,
) -> AppResult<Json<serde_json::Value>> {
    use crate::models::user::COLLECTION_NAME as USERS;
    use mongodb::bson::{self, doc};

    let actor = auth_user.user_id.to_string();

    // If setting (not clearing), confirm the actor is a member of the target org.
    if let Some(target) = body.primary_org_id.as_deref()
        && !org_service::is_member(&state.db, &actor, target).await?
    {
        return Err(AppError::OrgMembershipRequired);
    }

    let value = match body.primary_org_id.as_deref() {
        Some(id) => bson::Bson::String(id.to_string()),
        None => bson::Bson::Null,
    };

    state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! { "_id": &actor },
            doc! { "$set": {
                "primary_org_id": value,
                "updated_at": bson::DateTime::from_chrono(chrono::Utc::now()),
            }},
        )
        .await?;

    Ok(Json(
        serde_json::json!({ "primary_org_id": body.primary_org_id }),
    ))
}

#[cfg(test)]
mod tests {
    use super::UpdateMemberRequest;

    // Regression tests for ChronoAIProject/NyxID#363: `allowed_service_ids:
    // null` in PATCH /orgs/{id}/members/{id} must clear the scope. With
    // serde's default `Option<Option<T>>` deserialization, `null` and
    // "field absent" both collapsed to outer `None`, so the service layer
    // skipped the update entirely. The nullable_field helper disambiguates.

    #[test]
    fn allowed_service_ids_absent_leaves_scope_untouched() {
        let req: UpdateMemberRequest = serde_json::from_str(r#"{"role": "member"}"#).unwrap();
        assert!(req.allowed_service_ids.is_none());
    }

    #[test]
    fn allowed_service_ids_null_clears_scope() {
        let req: UpdateMemberRequest =
            serde_json::from_str(r#"{"role": "member", "allowed_service_ids": null}"#).unwrap();
        assert_eq!(req.allowed_service_ids, Some(None));
    }

    #[test]
    fn allowed_service_ids_array_restricts_scope() {
        let req: UpdateMemberRequest = serde_json::from_str(
            r#"{"role": "member", "allowed_service_ids": ["svc-a", "svc-b"]}"#,
        )
        .unwrap();
        assert_eq!(
            req.allowed_service_ids,
            Some(Some(vec!["svc-a".to_string(), "svc-b".to_string()]))
        );
    }

    #[test]
    fn allowed_service_ids_empty_array_is_zero_scope() {
        // Empty array is a legitimate state meaning "locked out of every
        // service"; distinct from null (clear) and absent (no change).
        let req: UpdateMemberRequest =
            serde_json::from_str(r#"{"role": "member", "allowed_service_ids": []}"#).unwrap();
        assert_eq!(req.allowed_service_ids, Some(Some(vec![])));
    }
}
