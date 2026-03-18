use axum::{
    Json,
    extract::{Path, State},
    http::header,
    response::{Html, IntoResponse, Response},
};
use utoipa::OpenApi;

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::services_helpers::fetch_service;
use crate::mw::auth::AuthUser;
use crate::services::api_docs_service;

#[utoipa::path(
    get,
    path = "/api/v1/docs",
    responses(
        (status = 200, description = "Scalar API reference for NyxID", content_type = "text/html")
    ),
    tag = "Docs"
)]
pub async fn docs_ui(State(state): State<AppState>, _auth_user: AuthUser) -> impl IntoResponse {
    let spec_url = format!(
        "{}/api/v1/docs/openapi.json",
        state.config.base_url.trim_end_matches('/')
    );
    html_response_with_csp(
        api_docs_service::render_scalar_html("NyxID API Docs", &spec_url),
        &api_docs_service::scalar_docs_csp(),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/catalog",
    responses(
        (status = 200, description = "NyxID unified API catalog", content_type = "text/html")
    ),
    tag = "Docs"
)]
pub async fn catalog_ui(_state: State<AppState>, _auth_user: AuthUser) -> impl IntoResponse {
    html_response_with_csp(
        api_docs_service::render_catalog_html().to_string(),
        api_docs_service::catalog_csp(),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/openapi.json",
    responses(
        (status = 200, description = "NyxID OpenAPI 3.1 specification", content_type = "application/json")
    ),
    tag = "Docs"
)]
pub async fn openapi_json(
    State(state): State<AppState>,
    _auth_user: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let mut openapi = crate::api_docs::ApiDoc::openapi();
    openapi.openapi = utoipa::openapi::OpenApiVersion::Version31;
    openapi.servers = Some(vec![utoipa::openapi::Server::new(
        state.config.base_url.trim_end_matches('/'),
    )]);

    let value = serde_json::to_value(openapi).map_err(|e| {
        crate::errors::AppError::Internal(format!("Failed to serialize OpenAPI document: {e}"))
    })?;
    Ok(Json(value))
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/asyncapi.json",
    responses(
        (status = 200, description = "NyxID AsyncAPI 3.0 specification", content_type = "application/json")
    ),
    tag = "Docs"
)]
pub async fn asyncapi_json(
    State(state): State<AppState>,
    _auth_user: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(api_docs_service::build_asyncapi_document(
        &state.config.base_url,
    )))
}

#[utoipa::path(
    get,
    path = "/api/v1/proxy/services/{service_id}/docs",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Scalar API reference for the downstream service", content_type = "text/html"),
        (status = 404, description = "Service or documentation not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy Docs"
)]
pub async fn service_docs_ui(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Response> {
    let service = fetch_service(&state, &service_id).await?;
    let base = state.config.base_url.trim_end_matches('/');

    let spec_url = if service.openapi_spec_url.is_some() {
        format!("{base}/api/v1/proxy/services/{service_id}/openapi.json")
    } else if service.asyncapi_spec_url.is_some() {
        format!("{base}/api/v1/proxy/services/{service_id}/asyncapi.json")
    } else {
        return Err(crate::errors::AppError::NotFound(
            "Service has no documentation spec configured".to_string(),
        ));
    };

    Ok(html_response_with_csp(
        api_docs_service::render_scalar_html(&format!("{} API Docs", service.name), &spec_url),
        &api_docs_service::scalar_docs_csp(),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/proxy/services/{service_id}/openapi.json",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Proxied OpenAPI document for a downstream service", content_type = "application/json"),
        (status = 404, description = "Service or spec not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy Docs"
)]
pub async fn service_openapi_json(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let service = fetch_service(&state, &service_id).await?;
    let spec = api_docs_service::fetch_downstream_openapi_spec(
        &state.http_client,
        &service,
        &state.config.base_url,
    )
    .await?;

    Ok(Json(spec))
}

#[utoipa::path(
    get,
    path = "/api/v1/proxy/services/{service_id}/asyncapi.json",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Proxied AsyncAPI document for a downstream service", content_type = "application/json"),
        (status = 404, description = "Service or spec not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy Docs"
)]
pub async fn service_asyncapi_json(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let service = fetch_service(&state, &service_id).await?;
    let spec = api_docs_service::fetch_downstream_asyncapi_spec(
        &state.http_client,
        &service,
        &state.config.base_url,
    )
    .await?;

    Ok(Json(spec))
}

fn html_response_with_csp(html: String, csp: &str) -> Response {
    let mut response = Html(html).into_response();
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        csp.parse().expect("valid docs CSP header"),
    );
    response
}
