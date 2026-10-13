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
