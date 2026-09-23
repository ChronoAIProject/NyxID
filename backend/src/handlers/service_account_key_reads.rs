use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, Method},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        catalog_skill_revision::SkillReference,
        service_account_key_read_grant::ServiceAccountKeyReadGrant,
    },
    mw::auth::{AuthMethod, AuthUser},
    services::{
        audit_service, catalog_skill_service, service_account_key_read_service as reads,
        service_account_service,
    },
};

#[derive(Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyMetadataResponse {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub label: String,
    pub service_type: String,
    pub is_active: bool,
    pub catalog_service_id: Option<String>,
    pub catalog_service_slug: Option<String>,
    pub catalog_service_name: Option<String>,
    pub recommended_skills: Option<Vec<String>>,
    pub recommended_skill_refs: Option<Vec<SkillReference>>,
    pub skills_revision: Option<i64>,
    pub skills_manifest_digest: String,
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum KeyReadResponse {
    User(Box<super::keys::KeyResponse>),
    ServiceAccount(Box<KeyMetadataResponse>),
}

pub async fn get_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> AppResult<Response> {
    if auth.auth_method != AuthMethod::ServiceAccount {
        let Json(response) = super::keys::get_key(State(state), auth, Path(id)).await?;
        return Ok(Json(KeyReadResponse::User(Box::new(response))).into_response());
    }
    if method != Method::GET || headers.contains_key(axum::http::header::UPGRADE) {
        return Err(AppError::Forbidden(
            "Key metadata access requires an ordinary GET".into(),
        ));
    }
    let data = reads::read(&state.db, &auth.user_id.to_string(), &auth.scope, &id).await?;
    let digest = catalog_skill_service::manifest_digest(&data.skills);
    let response = KeyReadResponse::ServiceAccount(Box::new(KeyMetadataResponse {
        id: data.id,
        slug: data.slug,
        name: data
            .catalog_service_name
            .clone()
            .unwrap_or_else(|| data.label.clone()),
        label: data.label,
        service_type: data.service_type,
        is_active: data.is_active,
        catalog_service_id: data.catalog_service_id,
        catalog_service_slug: data.catalog_service_slug,
        catalog_service_name: data.catalog_service_name,
        recommended_skills: data.skills.recommended_skills,
        recommended_skill_refs: data.skills.recommended_skill_refs,
        skills_revision: data.skills_revision,
        skills_manifest_digest: digest,
    }));
    Ok((
        [(axum::http::header::CACHE_CONTROL, "private, no-store")],
        Json(response),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub user_service_ids: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct GrantResponse {
    pub service_account_id: String,
    pub user_service_ids: Vec<String>,
    pub issued_by: String,
    pub issued_at: String,
    pub expires_at: Option<String>,
}

impl From<ServiceAccountKeyReadGrant> for GrantResponse {
    fn from(grant: ServiceAccountKeyReadGrant) -> Self {
        Self {
            service_account_id: grant.service_account_id,
            user_service_ids: grant
                .targets
                .into_iter()
                .map(|t| t.user_service_id)
                .collect(),
            issued_by: grant.issued_by,
            issued_at: grant.issued_at.to_rfc3339(),
            expires_at: grant.expires_at.map(|t| t.to_rfc3339()),
        }
    }
}

pub async fn get_grant(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Option<GrantResponse>>> {
    super::admin_helpers::require_admin_or_operator(
        &state,
        &auth,
        "admin.service_accounts.key_read_grant.get",
    )
    .await?;
    service_account_service::get_service_account(&state.db, &id).await?;
    Ok(Json(
        reads::get_grant(&state.db, &id).await?.map(Into::into),
    ))
}

pub async fn issue_grant(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<GrantRequest>,
) -> AppResult<Json<GrantResponse>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    let grant = reads::issue(
        &state.db,
        &id,
        &auth.user_id.to_string(),
        &body.user_service_ids,
        body.expires_at,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "admin.sa.key_read_granted",
        Some(
            serde_json::json!({"target_sa_id": id, "owner_id": grant.owner_id,
            "targets": grant.targets, "expires_at": grant.expires_at}),
        ),
    );
    Ok(Json(grant.into()))
}

pub async fn revoke_grant(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    service_account_service::get_service_account(&state.db, &id).await?;
    let previous = reads::revoke(&state.db, &id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "admin.sa.key_read_revoked",
        Some(
            serde_json::json!({"target_sa_id": id, "previous_grant": previous.map(GrantResponse::from)}),
        ),
    );
    Ok(Json(
        serde_json::json!({"message": "Key read grant revoked"}),
    ))
}
