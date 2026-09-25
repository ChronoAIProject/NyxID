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
        catalog_skill_revision::SkillReference, service_account::ServiceAccountPurpose,
        service_account_key_read_grant::ServiceAccountKeyReadGrant,
    },
    mw::auth::{AuthMethod, AuthUser},
    services::{
        audit_service, catalog_editor_catalog_service as catalog, catalog_editor_service as editor,
        catalog_skill_service, curation_grant_service as grants,
        service_account_key_read_service as reads, service_account_service,
    },
};

#[derive(Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyMetadataResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
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

impl From<reads::KeyMetadata> for KeyMetadataResponse {
    fn from(data: reads::KeyMetadata) -> Self {
        let digest = catalog_skill_service::manifest_digest(&data.skills);
        Self {
            resource_type: None,
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
        }
    }
}

impl From<catalog::CatalogMetadata> for KeyMetadataResponse {
    fn from(data: catalog::CatalogMetadata) -> Self {
        Self {
            resource_type: Some("catalog_service".into()),
            catalog_service_id: Some(data.id.clone()),
            catalog_service_slug: Some(data.slug.clone()),
            catalog_service_name: Some(data.name.clone()),
            id: data.id,
            slug: data.slug,
            label: data.name.clone(),
            name: data.name,
            service_type: data.service_type,
            is_active: data.is_active,
            skills_manifest_digest: catalog_skill_service::manifest_digest(&data.skills),
            recommended_skills: data.skills.recommended_skills,
            recommended_skill_refs: data.skills.recommended_skill_refs,
            skills_revision: Some(data.skills_revision),
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct KeyMetadataListResponse {
    pub keys: Vec<KeyMetadataResponse>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum KeyListReadResponse {
    User(super::keys::KeyListResponse),
    ServiceAccount(KeyMetadataListResponse),
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum KeyReadResponse {
    User(Box<super::keys::KeyResponse>),
    ServiceAccount(Box<KeyMetadataResponse>),
}

pub async fn list_keys(
    State(state): State<AppState>,
    auth: AuthUser,
    method: Method,
    headers: HeaderMap,
) -> AppResult<Response> {
    if auth.auth_method != AuthMethod::ServiceAccount {
        let Json(response) = super::keys::list_keys(State(state), auth).await?;
        return Ok(Json(KeyListReadResponse::User(response)).into_response());
    }
    if method != Method::GET || headers.contains_key(axum::http::header::UPGRADE) {
        return Err(AppError::Forbidden(
            "Key metadata access requires an ordinary GET".into(),
        ));
    }
    let sa =
        service_account_service::get_service_account(&state.db, &auth.user_id.to_string()).await?;
    let keys = if sa.purpose == ServiceAccountPurpose::CatalogEditor {
        if !sa.catalog_scope_authorized {
            grants::require_scope(&sa, &auth.scope, reads::READ_SCOPE)?;
        }
        editor::authorize(&state.db, &sa, &auth.scope, grants::READ_SCOPE).await?;
        catalog::list(&state.db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect()
    } else {
        reads::list(&state.db, &auth.user_id.to_string(), &auth.scope)
            .await?
            .into_iter()
            .map(Into::into)
            .collect()
    };
    Ok((
        [(axum::http::header::CACHE_CONTROL, "private, no-store")],
        Json(KeyListReadResponse::ServiceAccount(
            KeyMetadataListResponse { keys },
        )),
    )
        .into_response())
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
    let sa =
        service_account_service::get_service_account(&state.db, &auth.user_id.to_string()).await?;
    let data: KeyMetadataResponse = if sa.purpose == ServiceAccountPurpose::CatalogEditor {
        if !sa.catalog_scope_authorized {
            grants::require_scope(&sa, &auth.scope, reads::READ_SCOPE)?;
        }
        editor::authorize(&state.db, &sa, &auth.scope, grants::READ_SCOPE).await?;
        catalog::read(&state.db, &id).await?.into()
    } else {
        reads::read(&state.db, &auth.user_id.to_string(), &auth.scope, &id)
            .await?
            .into()
    };
    let response = KeyReadResponse::ServiceAccount(Box::new(data));
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
