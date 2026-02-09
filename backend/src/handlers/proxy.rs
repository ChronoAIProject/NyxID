use axum::{
    body::Body,
    extract::{Path, State},
    http::{Method, Request, StatusCode},
    response::Response,
};
use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::proxy_service;
use crate::AppState;

/// ANY /api/v1/proxy/:service_id/*path
///
/// Forward the request to the downstream service with credential injection.
/// Supports all HTTP methods. Strips the proxy prefix and forwards the
/// remainder of the path to the downstream service.
pub async fn proxy_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let target = proxy_service::resolve_proxy_target(
        &state.db,
        &encryption_key,
        &auth_user.user_id.to_string(),
        &service_id,
    )
    .await?;

    let method = request.method().clone();
    let query = request.uri().query().map(String::from);
    let headers = request.headers().clone();

    // Read the request body (10MB limit for proxy requests)
    let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read request body: {e}")))?;

    let body = if body_bytes.is_empty() {
        None
    } else {
        Some(body_bytes)
    };

    let reqwest_method = match method {
        Method::GET => reqwest::Method::GET,
        Method::POST => reqwest::Method::POST,
        Method::PUT => reqwest::Method::PUT,
        Method::DELETE => reqwest::Method::DELETE,
        Method::PATCH => reqwest::Method::PATCH,
        Method::HEAD => reqwest::Method::HEAD,
        Method::OPTIONS => reqwest::Method::OPTIONS,
        _ => return Err(AppError::BadRequest("Unsupported HTTP method".to_string())),
    };

    // Convert axum HeaderMap to reqwest HeaderMap
    let mut reqwest_headers = reqwest::header::HeaderMap::new();
    for (name, value) in headers.iter() {
        if let Ok(reqwest_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(reqwest_value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                reqwest_headers.insert(reqwest_name, reqwest_value);
            }
        }
    }

    // Reuse the shared reqwest::Client from AppState for connection pooling
    let downstream_response = proxy_service::forward_request(
        &state.http_client,
        &target,
        reqwest_method,
        &path,
        query.as_deref(),
        reqwest_headers,
        body,
    )
    .await?;

    // Convert reqwest Response back to axum Response
    let status = StatusCode::from_u16(downstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);

    let mut response_builder = Response::builder().status(status);

    // Forward response headers
    for (name, value) in downstream_response.headers().iter() {
        let name_str = name.as_str();
        // Skip hop-by-hop headers
        if !matches!(
            name_str,
            "transfer-encoding" | "connection" | "keep-alive"
        ) {
            if let Ok(header_name) = axum::http::header::HeaderName::from_bytes(name_str.as_bytes()) {
                if let Ok(header_value) = axum::http::header::HeaderValue::from_bytes(value.as_bytes()) {
                    response_builder = response_builder.header(header_name, header_value);
                }
            }
        }
    }

    let response_body = downstream_response
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read downstream response: {e}")))?;

    let response = response_builder
        .body(Body::from(response_body))
        .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?;

    Ok(response)
}
