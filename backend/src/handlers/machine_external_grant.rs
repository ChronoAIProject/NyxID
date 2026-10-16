//! Human-only Machine resource grant management. Route registration is deferred
//! until the coordinated external authority protocol and delegated denials land.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::machine_external_grant::{ExternalGrantBinding, MachineExternalGrant},
    mw::auth::AuthUser,
    services::machine_external_grant_service::{
        self as grants, GrantSelection, SourceReference, ValidityReason,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyGrantPut {
    /// None creates a new contract; Some edits exactly this active revision.
    pub expected_revision: Option<i64>,
    pub expected_grant_id: Option<String>,
    pub expected_state_version: i64,
    pub selection: GrantSelection,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthGrantPut {
    pub expected_revision: i64,
    pub expected_grant_id: String,
    pub selection: GrantSelection,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRevoke {
    pub expected_revision: i64,
    pub expected_grant_id: String,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceResponse {
    #[serde(rename = "oauth")]
    OAuth {
        oauth_client_id: String,
        consent_id: String,
        consent_revision: String,
    },
    ApiKey {
        api_key_id: String,
        api_key_state_version_at_approval: i64,
    },
}
#[derive(Serialize)]
pub struct AssignmentResponse {
    pub capabilities: nyxid_machine::authority::Capabilities,
    pub mode: String,
    pub revision: i64,
    pub saved_login_ids: Vec<String>,
    pub file_roots: Vec<String>,
    pub service_ids: Vec<String>,
}
#[derive(Serialize)]
pub struct GrantResponse {
    pub id: String,
    pub source: SourceResponse,
    pub revision: i64,
    pub policy_revision: i64,
    pub status: &'static str,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_reason: Option<String>,
    pub assignments: BTreeMap<String, AssignmentResponse>,
}
impl From<MachineExternalGrant> for GrantResponse {
    fn from(value: MachineExternalGrant) -> Self {
        let source = match value.binding {
            ExternalGrantBinding::OAuth {
                oauth_client_id,
                consent_id,
                consent_revision,
            } => SourceResponse::OAuth {
                oauth_client_id,
                consent_id,
                consent_revision,
            },
            ExternalGrantBinding::ApiKey {
                api_key_id,
                api_key_state_version_at_approval,
            } => SourceResponse::ApiKey {
                api_key_id,
                api_key_state_version_at_approval,
            },
        };
        let assignments = value
            .policy
            .assignments
            .into_iter()
            .map(|(id, a)| {
                let r = value.restrictions.get(&id);
                (
                    id,
                    AssignmentResponse {
                        capabilities: a.capabilities,
                        mode: a.mode,
                        revision: a.revision,
                        saved_login_ids: a.saved_login_ids.unwrap_or_default(),
                        file_roots: r.map(|r| r.file_roots.clone()).unwrap_or_default(),
                        service_ids: r.map(|r| r.service_ids.clone()).unwrap_or_default(),
                    },
                )
            })
            .collect();
        Self {
            id: value.id,
            source,
            revision: value.revision,
            policy_revision: value.policy.revision,
            status: match value.status {
                crate::models::machine_external_grant::GrantStatus::Active => "active",
                crate::models::machine_external_grant::GrantStatus::Revoked => "revoked",
            },
            expires_at: value.expires_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
            revoked_at: value.revoked_at,
            revoked_reason: value.revoked_reason,
            assignments,
        }
    }
}
#[derive(Serialize)]
pub struct GrantReadResponse {
    pub grant: Option<GrantResponse>,
    /// Additive independent live checks. The combined field below remains for
    /// older consumers and is exactly `source_valid && grant_valid`.
    pub source_valid: bool,
    pub source_validity_reason: Option<ValidityReason>,
    pub grant_valid: bool,
    pub grant_validity_reason: Option<ValidityReason>,
    pub source_and_grant_valid: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineGrantListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct MachineGrantListResponse {
    pub external_grants: Vec<GrantResponse>,
    pub next_cursor: Option<String>,
}

/// Additive replacement for the existing per-Machine owner list route.
/// Route registration is owned by the integration coordinator.
pub async fn list_machine(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(machine): Path<String>,
    Query(query): Query<MachineGrantListQuery>,
) -> AppResult<Json<MachineGrantListResponse>> {
    let user = owner(&auth)?;
    let page = grants::list_for_machine_page(
        &state.db,
        &user,
        &machine,
        query.cursor.as_deref(),
        query.limit.unwrap_or(100),
    )
    .await?;
    Ok(Json(MachineGrantListResponse {
        external_grants: page.grants.into_iter().map(Into::into).collect(),
        next_cursor: page.next_cursor,
    }))
}
fn owner(auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    Ok(auth.user_id.to_string())
}
fn changed() -> AppError {
    AppError::Conflict("Machine grant changed; reload and review the current grant".into())
}
fn audit(state: &AppState, user: &str, row: &MachineExternalGrant, action: &str) {
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(user.into()),
        action.into(),
        Some(serde_json::json!({"grant_id":row.id,"revision":row.revision})),
        None,
        None,
        None,
        None,
    );
}

async fn read_response(
    state: &AppState,
    user: &str,
    source: SourceReference,
    row: Option<MachineExternalGrant>,
) -> AppResult<GrantReadResponse> {
    let validity = grants::evaluate_validity(&state.db, user, &source, row.as_ref()).await?;
    Ok(GrantReadResponse {
        grant: row.map(Into::into),
        source_valid: validity.source.valid,
        source_validity_reason: validity.source.reason,
        grant_valid: validity.grant.valid,
        grant_validity_reason: validity.grant.reason,
        source_and_grant_valid: validity.combined,
    })
}
pub async fn get_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
) -> AppResult<Json<GrantReadResponse>> {
    let user = owner(&auth)?;
    let row = grants::get_latest_for_key(&state.db, &user, &key).await?;
    Ok(Json(
        read_response(
            &state,
            &user,
            SourceReference::ApiKey { api_key_id: key },
            row,
        )
        .await?,
    ))
}
pub async fn put_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
    Json(body): Json<KeyGrantPut>,
) -> AppResult<Json<GrantResponse>> {
    let user = owner(&auth)?;
    let row = match (body.expected_revision, body.expected_grant_id.as_deref()) {
        (None, None) => {
            grants::approve_api_key(
                &state.db,
                &user,
                &key,
                body.expected_state_version,
                &body.selection,
            )
            .await?
        }
        (Some(revision), Some(expected_id)) => {
            let old = grants::get_active_for_key(&state.db, &user, &key)
                .await?
                .ok_or_else(changed)?;
            if old.id != expected_id {
                return Err(changed());
            }
            grants::edit_api_key(
                &state.db,
                &user,
                &old.id,
                revision,
                body.expected_state_version,
                &body.selection,
            )
            .await?
        }
        _ => {
            return Err(AppError::ValidationError(
                "Grant ID and revision must be supplied together".into(),
            ));
        }
    };
    audit(&state, &user, &row, "machine_external_grant_changed");
    Ok(Json(row.into()))
}
pub async fn revoke_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
    Json(body): Json<GrantRevoke>,
) -> AppResult<Json<GrantResponse>> {
    let user = owner(&auth)?;
    let old = grants::get_active_for_key(&state.db, &user, &key)
        .await?
        .ok_or_else(changed)?;
    if old.id != body.expected_grant_id {
        return Err(changed());
    }
    let row = grants::revoke(&state.db, &user, &old.id, body.expected_revision).await?;
    audit(&state, &user, &row, "machine_external_grant_revoked");
    Ok(Json(row.into()))
}
pub async fn get_oauth(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(client): Path<String>,
) -> AppResult<Json<GrantReadResponse>> {
    let user = owner(&auth)?;
    let row = grants::get_latest_for_oauth(&state.db, &user, &client).await?;
    let consent = row.as_ref().and_then(|grant| match &grant.binding {
        ExternalGrantBinding::OAuth {
            consent_id,
            consent_revision,
            ..
        } => Some(crate::models::consent::ProtectedConsentRef {
            consent_id: consent_id.clone(),
            consent_revision: consent_revision.clone(),
        }),
        ExternalGrantBinding::ApiKey { .. } => None,
    });
    Ok(Json(
        read_response(
            &state,
            &user,
            SourceReference::OAuth {
                oauth_client_id: client,
                consent,
            },
            row,
        )
        .await?,
    ))
}
pub async fn put_oauth(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(client): Path<String>,
    Json(body): Json<OAuthGrantPut>,
) -> AppResult<Json<GrantResponse>> {
    let user = owner(&auth)?;
    // Never creates/revives an OAuth grant. First approval/reconsent must use
    // the server-signed, one-time OAuth consent request.
    let old = grants::get_active_for_oauth(&state.db, &user, &client)
        .await?
        .ok_or_else(changed)?;
    if old.id != body.expected_grant_id {
        return Err(changed());
    }
    let row = grants::edit(
        &state.db,
        &user,
        &old.id,
        body.expected_revision,
        &body.selection,
    )
    .await?;
    audit(&state, &user, &row, "machine_external_grant_changed");
    Ok(Json(row.into()))
}
pub async fn revoke_oauth(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(client): Path<String>,
    Json(body): Json<GrantRevoke>,
) -> AppResult<Json<GrantResponse>> {
    let user = owner(&auth)?;
    let old = grants::get_active_for_oauth(&state.db, &user, &client)
        .await?
        .ok_or_else(changed)?;
    if old.id != body.expected_grant_id {
        return Err(changed());
    }
    let row = grants::revoke(&state.db, &user, &old.id, body.expected_revision).await?;
    audit(&state, &user, &row, "machine_external_grant_revoked");
    Ok(Json(row.into()))
}
