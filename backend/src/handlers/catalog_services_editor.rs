use super::{
    services::{CreateServiceRequest, ServiceResponse, UpdateServiceRequest},
    services_helpers,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::downstream_service::OfferingKind,
    mw::auth::AuthUser,
    services::catalog_services_access,
    telemetry::TelemetryContext,
};
use axum::{
    Json,
    extract::{Path, State},
};

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    Json(mut value): Json<serde_json::Value>,
) -> AppResult<Json<ServiceResponse>> {
    if !services_helpers::is_admin(&state, &auth).await? {
        catalog_services_access::authorize(&state.db, &auth, true).await?;
        catalog_services_access::validate_editor_fields(&value, true)?;
        if value["offering_kind"] != "tool" {
            return Err(AppError::Forbidden(
                "Editors can create tool offerings only".into(),
            ));
        }
        value["base_url"] = serde_json::json!("https://example.invalid");
        value["service_category"] = serde_json::json!("internal");
        value["auth_method"] = serde_json::json!("none");
    }
    let body: CreateServiceRequest =
        serde_json::from_value(value).map_err(|e| AppError::ValidationError(e.to_string()))?;
    super::services::create_service(State(state), auth, tele, Json(body)).await
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    Path(id): Path<String>,
    Json(value): Json<serde_json::Value>,
) -> AppResult<Json<ServiceResponse>> {
    let service = services_helpers::fetch_service(&state, &id).await?;
    if service.offering_kind == OfferingKind::Tool
        && !services_helpers::is_admin(&state, &auth).await?
    {
        catalog_services_access::authorize(&state.db, &auth, true).await?;
        catalog_services_access::validate_editor_fields(&value, false)?;
        if value
            .get("offering_kind")
            .is_some_and(|kind| kind != "tool")
        {
            return Err(AppError::Forbidden(
                "offering_kind: only admins can convert tools into AI services".into(),
            ));
        }
    }
    let body: UpdateServiceRequest =
        serde_json::from_value(value).map_err(|e| AppError::ValidationError(e.to_string()))?;
    super::services::update_service(State(state), auth, tele, Path(id), Json(body)).await
}
