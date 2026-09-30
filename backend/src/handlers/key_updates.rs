use axum::{
    Json,
    extract::{DefaultBodyLimit, FromRequest, Path, Request, State},
    http::{Method, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use utoipa::{PartialSchema, ToSchema};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{catalog_skill_revision::SkillReference, service_account::ServiceAccountPurpose},
    mw::auth::{AuthMethod, AuthUser},
    services::{
        catalog_editor_catalog_service::CatalogMetadata,
        catalog_editor_service,
        catalog_skill_service::{self, SkillActor},
        curation_grant_service, service_account_service,
    },
};

/// CatalogEditor only: replace all pinned recommendations on a catalog UUID.
/// An empty array clears recommendations; package content and visibility stay in Ornn.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogSkillRefsUpdateRequest {
    pub recommended_skill_refs: Vec<SkillReference>,
}

/// The verified caller selects the request contract. Human callers continue to
/// ignore recommended_skill_refs and process their supported settings normally.
pub struct KeyUpdateRequest;

impl PartialSchema for KeyUpdateRequest {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::AnyOf::builder()
            .item(utoipa::openapi::Ref::from_schema_name(super::keys::UpdateKeyRequest::name()))
            .item(utoipa::openapi::Ref::from_schema_name(CatalogSkillRefsUpdateRequest::name()))
            .description(Some("Human callers update their connections; CatalogEditor service accounts replace catalog recommendations using only recommended_skill_refs and catalog:skills:write. The authenticated caller determines the contract."))
            .into()
    }
}

impl ToSchema for KeyUpdateRequest {
    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        schemas.push((
            super::keys::UpdateKeyRequest::name().into_owned(),
            super::keys::UpdateKeyRequest::schema(),
        ));
        super::keys::UpdateKeyRequest::schemas(schemas);
        schemas.push((
            CatalogSkillRefsUpdateRequest::name().into_owned(),
            CatalogSkillRefsUpdateRequest::schema(),
        ));
        CatalogSkillRefsUpdateRequest::schemas(schemas);
    }
}

pub async fn update_key(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    mut request: Request,
) -> AppResult<Response> {
    if auth.auth_method != AuthMethod::ServiceAccount {
        let body = match Json::<super::keys::UpdateKeyRequest>::from_request(request, &state).await
        {
            Ok(body) => body,
            Err(rejection) => return Ok(rejection.into_response()),
        };
        return Ok(super::keys::update_key(State(state), auth, Path(id), body)
            .await?
            .into_response());
    }
    let sa =
        service_account_service::get_service_account(&state.db, &auth.user_id.to_string()).await?;
    if sa.purpose != ServiceAccountPurpose::CatalogEditor {
        return Err(AppError::Forbidden(
            "Service accounts cannot access this endpoint".into(),
        ));
    }
    catalog_editor_service::authorize(
        &state.db,
        &sa,
        &auth.scope,
        curation_grant_service::WRITE_SCOPE,
    )
    .await?;
    if request.method() != Method::PUT || request.headers().contains_key(header::UPGRADE) {
        return Err(AppError::Forbidden(
            "Catalog skill updates require an ordinary PUT".into(),
        ));
    }
    let actor = SkillActor::Curation {
        id: sa.id,
        scope: auth.scope.clone(),
        token_jti: auth
            .token_jti
            .clone()
            .ok_or_else(|| AppError::Unauthorized("Service account token required".into()))?,
    };
    DefaultBodyLimit::max(70_000).apply(&mut request);
    let Json(body) =
        match Json::<CatalogSkillRefsUpdateRequest>::from_request(request, &state).await {
            Ok(body) => body,
            Err(rejection) => return Ok(rejection.into_response()),
        };
    let (committed, request_id) = catalog_skill_service::replace_refs_at_current_revision(
        &state.db,
        &id,
        &actor,
        body.recommended_skill_refs,
    )
    .await?;
    if committed.changed {
        super::catalog_curation::audit_change(&state, &auth, &id, committed.revision, &request_id);
    }
    let metadata =
        CatalogMetadata::from_committed(committed.service, committed.state, committed.revision);
    let response = super::service_account_key_reads::KeyReadResponse::ServiceAccount(Box::new(
        metadata.into(),
    ));
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(response),
    )
        .into_response())
}
