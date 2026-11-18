use super::catalog_authority;
use crate::{AppState, errors::AppResult, mw::auth::AuthUser, services::catalog_icon_service};
use axum::{
    Json,
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Upload {
    pub svg: String,
}
#[derive(Serialize)]
pub struct Uploaded {
    pub icon_url: String,
}

pub async fn upload(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<Upload>,
) -> AppResult<Json<Uploaded>> {
    // Icons are display only, so a Catalog Manager may set one on any provider.
    catalog_authority::require_catalog_writer(&state, &auth).await?;
    crate::services::provider_service::get_provider(&state.db, &provider_id).await?;
    let id = catalog_icon_service::store(&state.db, &body.svg).await?;
    Ok(Json(Uploaded {
        icon_url: format!(
            "{}/catalog-icons/{}",
            state.config.base_url.trim_end_matches('/'),
            id
        ),
    }))
}

pub async fn serve(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Response> {
    let svg = catalog_icon_service::load(&state.db, &id).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox",
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        svg,
    )
        .into_response())
}

/// Store and attach a provider-less Tool asset with live authority rechecked.
pub async fn upload_tool(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<Upload>,
) -> AppResult<Json<Uploaded>> {
    let service = super::services_helpers::fetch_service(&state, &service_id).await?;
    if service.offering_kind != crate::models::downstream_service::OfferingKind::Tool {
        return Err(crate::errors::AppError::ValidationError(
            "Tool icon route requires a Tool".into(),
        ));
    }
    catalog_authority::require_tool_writer(&state, &auth, Some(&service)).await?;
    let id = catalog_icon_service::store(&state.db, &body.svg).await?;
    let icon_url = format!(
        "{}/catalog-icons/{id}",
        state.config.base_url.trim_end_matches('/')
    );
    let actor = crate::services::audit_service::AuditActor::from_auth_user(&auth);
    crate::services::catalog_mutation_service::commit_update(
        &state.db,
        &service,
        &crate::services::catalog_mutation_service::Authority {
            actor,
            first_party_human: true,
            admin_required: super::services_helpers::is_admin(&state, &auth).await?,
            catalog_base_aliases: state.config.domain_aliases.base_urls.clone(),
            catalog_base_url: state.config.base_url.clone(),
        },
        &mongodb::bson::doc! {"icon_url":&icon_url,"updated_at":mongodb::bson::DateTime::now()},
        &mongodb::bson::doc! {"_id":&service.id},
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await?;
    Ok(Json(Uploaded { icon_url }))
}
