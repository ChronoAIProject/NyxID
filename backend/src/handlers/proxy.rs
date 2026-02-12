use axum::{
    body::Body,
    extract::{Path, State},
    http::{Method, Request, StatusCode},
    response::Response,
};
use mongodb::bson::doc;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, delegation_service, identity_service, proxy_service};
use crate::AppState;

/// Response headers that are safe to forward back to the client.
/// Uses an allowlist to prevent leaking internal headers from downstream services.
const ALLOWED_RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-disposition",
    "cache-control",
    "etag",
    "last-modified",
    "x-request-id",
    "x-correlation-id",
    "vary",
    "access-control-allow-origin",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-expose-headers",
];

/// ANY /api/v1/proxy/:service_id/*path
///
/// Forward the request to the downstream service with credential injection,
/// identity propagation, and delegated provider credentials.
pub async fn proxy_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let user_id_str = auth_user.user_id.to_string();

    let target = match proxy_service::resolve_proxy_target(
        &state.db,
        &encryption_key,
        &user_id_str,
        &service_id,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            audit_service::log_async(
                state.db.clone(),
                Some(user_id_str.clone()),
                "proxy_request_denied".to_string(),
                Some(serde_json::json!({
                    "service_id": &service_id,
                    "reason": e.to_string(),
                })),
                None,
                None,
            );
            return Err(e);
        }
    };

    // Build identity headers if configured on the service
    let mut identity_headers = Vec::new();

    if target.service.identity_propagation_mode != "none" {
        // Fetch user for identity propagation
        let user = state
            .db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id_str })
            .await?;

        if let Some(ref user) = user {
            // Add identity headers (for "headers" or "both" modes)
            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "headers" | "both"
            ) {
                identity_headers = identity_service::build_identity_headers(user, &target.service);
            }

            // Generate identity JWT assertion (for "jwt" or "both" modes)
            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "jwt" | "both"
            ) {
                match identity_service::generate_identity_assertion(
                    &state.jwt_keys,
                    &state.config,
                    user,
                    &target.service,
                ) {
                    Ok(assertion) => {
                        identity_headers.push((
                            "X-NyxID-Identity-Token".to_string(),
                            assertion,
                        ));
                    }
                    Err(e) => {
                        tracing::warn!(
                            service_id = %service_id,
                            error = %e,
                            "Failed to generate identity assertion"
                        );
                    }
                }
            }
        }
    }

    // Generate delegation token if configured on the service
    if target.service.inject_delegation_token {
        let user_uuid = auth_user.user_id;

        match crate::crypto::jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &user_uuid,
            &target.service.delegation_token_scope,
            &target.service.slug,
            crate::crypto::jwt::MCP_DELEGATION_TOKEN_TTL_SECS,
        ) {
            Ok(delegation_token) => {
                identity_headers.push((
                    "X-NyxID-Delegation-Token".to_string(),
                    delegation_token,
                ));
            }
            Err(e) => {
                tracing::warn!(
                    service_id = %service_id,
                    error = %e,
                    "Failed to generate delegation token for proxy"
                );
            }
        }
    }

    // Resolve delegated credentials (non-fatal: proceed without on error)
    let delegated = delegation_service::resolve_delegated_credentials(
        &state.db,
        &encryption_key,
        &user_id_str,
        &service_id,
    )
    .await
    .unwrap_or_default();

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
        if let Ok(reqwest_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Ok(reqwest_value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                reqwest_headers.insert(reqwest_name, reqwest_value);
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
        identity_headers,
        delegated,
    )
    .await?;

    // Convert reqwest Response back to axum Response
    let status = StatusCode::from_u16(downstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);

    let mut response_builder = Response::builder().status(status);

    // Forward only allowlisted response headers
    for (name, value) in downstream_response.headers().iter() {
        let name_lower = name.as_str().to_lowercase();
        if ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str())
            && let Ok(header_name) =
                axum::http::header::HeaderName::from_bytes(name.as_str().as_bytes())
                && let Ok(header_value) =
                    axum::http::header::HeaderValue::from_bytes(value.as_bytes())
                {
                    response_builder = response_builder.header(header_name, header_value);
                }
    }

    let response_body = downstream_response
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read downstream response: {e}")))?;

    let response = response_builder
        .body(Body::from(response_body))
        .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?;

    // Audit log the proxy request
    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "proxy_request".to_string(),
        Some(serde_json::json!({
            "service_id": &service_id,
            "method": method.as_str(),
            "path": &path,
            "response_status": status.as_u16(),
            "acting_client_id": auth_user.acting_client_id,
        })),
        None,
        None,
    );

    Ok(response)
}
