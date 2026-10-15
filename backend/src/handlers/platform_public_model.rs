//! Dedicated callback ingress. It never constructs ordinary AuthUser authority.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    services::{
        key_service,
        platform_public_proxy_service::{self, ModelRequest},
        platform_public_tools,
    },
};
use axum::{
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{Method, Request},
    response::Response,
};
use zeroize::Zeroizing;

/// POST /api/v1/public-nyxbot/models/{service_id}/{*path}
/// Mount without ordinary AuthUser middleware; this extractor accepts only an
/// exact public-purpose primary key and explicit immutable callback headers.
pub async fn dispatch(
    State(state): State<AppState>,
    Path((service_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let denied = || AppError::Forbidden("Invalid public model callback".into());
    if request.method() != Method::POST
        || request.uri().query().is_some()
        || request.headers().contains_key("upgrade")
        || request.headers().contains_key("mcp-session-id")
    {
        return Err(denied());
    }
    let headers = request.headers();
    if headers.get_all("authorization").iter().count() != 1
        || headers.contains_key("x-api-key")
        || headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("application/json")
    {
        return Err(denied());
    }
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(denied)?;
    let (_, key, child) = key_service::validate_api_key(&state.db, raw).await?;
    let authority = platform_public_tools::authenticate(&state.db, &key, child.as_deref()).await?;
    authority.validate_callback_headers(headers)?;
    if headers.get_all("x-nyx-public-call-id").iter().count() != 1 {
        return Err(denied());
    }
    let call = headers
        .get("x-nyx-public-call-id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(denied)?
        .to_owned();
    let id = uuid::Uuid::parse_str(&call).map_err(|_| denied())?;
    if id.get_version_num() != 4 {
        return Err(denied());
    }
    // Authority comes from storage; headers only select the exact stored tuple.
    // An old callback using a reused key cannot attach to the sender's new turn.
    let maximum = authority
        .context()
        .model_operations
        .iter()
        .filter(|o| {
            o.service_id == service_id && o.canonical_path == path && o.method == Method::POST
        })
        .map(|o| o.max_input_bytes as usize)
        .min()
        .ok_or_else(denied)?
        .min(state.config.proxy_max_body_size);
    let body = Zeroizing::new(
        to_bytes(request.into_body(), maximum)
            .await
            .map_err(|_| denied())?
            .to_vec(),
    );
    platform_public_proxy_service::dispatch_model(
        &state,
        authority.context(),
        ModelRequest {
            key_id: authority.key_id(),
            service_id: &service_id,
            path: &path,
            body: &body,
            call_id: &call,
        },
    )
    .await
}
