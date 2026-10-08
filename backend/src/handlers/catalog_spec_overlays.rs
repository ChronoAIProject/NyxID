use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::downstream_service::CatalogImportSource,
    mw::auth::AuthUser,
    services::catalog_spec_overlay_service,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct OverlayRequest {
    pub document: serde_json::Value,
    pub source: Option<CatalogImportSource>,
}

#[derive(Serialize)]
pub struct OverlayResponse {
    pub service_id: String,
    pub document: serde_json::Value,
    pub previous_document: Option<serde_json::Value>,
    pub sha256: String,
    pub revision: i64,
    pub source: Option<CatalogImportSource>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub operations_synced: Option<usize>,
    pub operations_added: Option<usize>,
    pub operations_changed: Option<usize>,
}

fn response(
    row: crate::models::catalog_spec_overlay::CatalogSpecOverlay,
    counts: Option<catalog_spec_overlay_service::SyncCounts>,
) -> OverlayResponse {
    OverlayResponse {
        service_id: row.service_id,
        document: row.document,
        previous_document: row.previous_document,
        sha256: row.sha256,
        revision: row.revision,
        source: row.source,
        created_by: row.created_by,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
        operations_synced: counts.as_ref().map(|c| c.operations_synced),
        operations_added: counts.as_ref().map(|c| c.operations_added),
        operations_changed: counts.as_ref().map(|c| c.operations_changed),
    }
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<OverlayResponse>> {
    let service = super::services_helpers::fetch_service(&state, &id).await?;
    crate::services::catalog_services_access::authorize_tool(&state.db, &auth, &service, false)
        .await?;
    let row = catalog_spec_overlay_service::get(&state.db, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("Spec overlay not found".into()))?;
    Ok(Json(response(row, None)))
}

pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<OverlayRequest>,
) -> AppResult<Json<OverlayResponse>> {
    let service = super::services_helpers::fetch_service(&state, &id).await?;
    crate::services::catalog_services_access::authorize_tool(&state.db, &auth, &service, true)
        .await?;
    super::services_helpers::require_http_service(&service)?;
    let (row, count) = catalog_spec_overlay_service::put(
        &state.db,
        &service,
        body.document,
        body.source,
        &auth.user_id.to_string(),
        &state.config.base_url,
    )
    .await?;
    Ok(Json(response(row, Some(count))))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let service = super::services_helpers::fetch_service(&state, &id).await?;
    crate::services::catalog_services_access::authorize_tool(&state.db, &auth, &service, true)
        .await?;
    catalog_spec_overlay_service::delete(&state.db, &id).await?;
    Ok(Json(
        serde_json::json!({"message":"Overlay removed; endpoint rows retained"}),
    ))
}

pub async fn hosted(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    if let Some(row) = catalog_spec_overlay_service::get(&state.db, &id).await? {
        return Ok(Json(row.document));
    }
    let service = super::services_helpers::fetch_service(&state, &id).await?;
    crate::services::catalog_spec_registry::spec_for_slug(&service.slug)
        .map(|s| Json((*s).clone()))
        .ok_or_else(|| AppError::NotFound("Spec overlay not found".into()))
}
