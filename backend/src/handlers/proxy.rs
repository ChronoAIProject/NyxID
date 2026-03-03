use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{Method, Request, StatusCode},
    response::Response,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::AppState;
use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::mw::auth::AuthUser;
use crate::services::{
    approval_service, audit_service, delegation_service, identity_service, notification_service,
    proxy_service,
};

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
    execute_proxy(&state, &auth_user, &service_id, &path, request).await
}

/// ANY /api/v1/proxy/s/:slug/*path
///
/// Resolve the service by slug, then forward via the shared proxy pipeline.
pub async fn proxy_request_by_slug(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let service = proxy_service::resolve_service_by_slug(&state.db, &slug).await?;
    execute_proxy(&state, &auth_user, &service.id, &path, request).await
}

/// Core proxy execution logic shared by UUID and slug handlers.
///
/// Takes the resolved service_id (UUID string) and executes the full proxy
/// pipeline: resolve target, build identity headers, inject delegation token,
/// resolve delegated credentials, forward request, and return response.
async fn execute_proxy(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
) -> AppResult<Response> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let user_id_str = auth_user.user_id.to_string();
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();

    let target = match proxy_service::resolve_proxy_target(
        &state.db,
        &encryption_key,
        &user_id_str,
        service_id,
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
                    "service_id": service_id,
                    "reason": e.to_string(),
                })),
                None,
                None,
            );
            return Err(e);
        }
    };

    // Check approval if user has it enabled.
    // Only direct browser sessions bypass approval; API keys, delegated tokens,
    // and service accounts all require approval since they represent programmatic access.
    let requires_approval =
        approval_service::user_requires_approval(&state.db, &approval_owner_user_id).await?;

    if requires_approval && auth_user.auth_method != crate::mw::auth::AuthMethod::Session {
        let requester_type = auth_user.approval_requester_type().ok_or_else(|| {
            AppError::Forbidden("Session auth does not require approval".to_string())
        })?;
        let requester_id = auth_user.approval_requester_id();

        let has_grant = approval_service::check_approval(
            &state.db,
            &approval_owner_user_id,
            service_id,
            requester_type,
            &requester_id,
        )
        .await?;

        if !has_grant {
            let channel =
                notification_service::get_or_create_channel(&state.db, &approval_owner_user_id)
                    .await?;

            let request_method = request.method().as_str().to_string();
            let timeout_secs = channel.approval_timeout_secs;
            let approval_request = approval_service::create_approval_request(
                &state.db,
                &state.config,
                &state.http_client,
                &approval_owner_user_id,
                service_id,
                &target.service.name,
                &target.service.slug,
                requester_type,
                &requester_id,
                None,
                &format!("proxy:{} {}", request_method, path),
                timeout_secs,
            )
            .await?;

            // Block until the user approves/rejects or timeout expires
            approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
                .await?;
        }
    }

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
                        identity_headers.push(("X-NyxID-Identity-Token".to_string(), assertion));
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
                identity_headers.push(("X-NyxID-Delegation-Token".to_string(), delegation_token));
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
        service_id,
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
            && let Ok(reqwest_value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
        {
            reqwest_headers.insert(reqwest_name, reqwest_value);
        }
    }

    // Reuse the shared reqwest::Client from AppState for connection pooling
    let downstream_response = proxy_service::forward_request(
        &state.http_client,
        &target,
        reqwest_method,
        path,
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
            && let Ok(header_value) = axum::http::header::HeaderValue::from_bytes(value.as_bytes())
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
            "service_id": service_id,
            "method": method.as_str(),
            "path": path,
            "response_status": status.as_u16(),
            "acting_client_id": &auth_user.acting_client_id,
        })),
        None,
        None,
    );

    Ok(response)
}

#[derive(Debug, Serialize)]
pub struct ProxyServiceItem {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub service_category: String,
    /// Whether the user has an active connection to this service
    pub connected: bool,
    /// Whether a connection is required before proxying
    pub requires_connection: bool,
    /// UUID-based proxy URL
    pub proxy_url: String,
    /// Slug-based proxy URL (developer-friendly)
    pub proxy_url_slug: String,
}

#[derive(Debug, Deserialize)]
pub struct ProxyServicesQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ProxyServicesResponse {
    pub services: Vec<ProxyServiceItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

/// GET /api/v1/proxy/services
///
/// List downstream services available for proxying with their proxy URLs.
/// Excludes "provider" category services (not proxyable).
/// Supports pagination via `page` and `per_page` query parameters.
pub async fn list_proxy_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ProxyServicesQuery>,
) -> AppResult<Json<ProxyServicesResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let base = state.config.base_url.trim_end_matches('/');

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let filter = doc! {
        "is_active": true,
        "service_category": { "$ne": "provider" },
    };

    // Get total count for pagination metadata
    let total = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .count_documents(filter.clone())
        .await?;

    // Get paginated active, non-provider services
    let services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    // Get user's active connections in a single query
    let service_ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    let connections: Vec<UserServiceConnection> = if service_ids.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .find(doc! {
                "user_id": &user_id_str,
                "service_id": { "$in": &service_ids },
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?
    };

    let connected_set: HashSet<&str> = connections.iter().map(|c| c.service_id.as_str()).collect();

    let items: Vec<ProxyServiceItem> = services
        .iter()
        .map(|s| ProxyServiceItem {
            id: s.id.clone(),
            name: s.name.clone(),
            slug: s.slug.clone(),
            description: s.description.clone(),
            service_category: s.service_category.clone(),
            connected: connected_set.contains(s.id.as_str()),
            requires_connection: s.requires_user_credential,
            proxy_url: format!("{base}/api/v1/proxy/{}/{{path}}", s.id),
            proxy_url_slug: format!("{base}/api/v1/proxy/s/{}/{{path}}", s.slug),
        })
        .collect();

    Ok(Json(ProxyServicesResponse {
        services: items,
        total,
        page,
        per_page,
    }))
}
