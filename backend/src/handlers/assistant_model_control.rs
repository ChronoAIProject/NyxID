//! Private runtime protocol: every operation requires both the original live
//! conversation credential and a server-issued capability. No GET endpoints.
use axum::{
    Json,
    body::Body,
    extract::State,
    http::{Request, header},
    response::{IntoResponse, Response},
};
use serde::{Serialize, de::DeserializeOwned};
use zeroize::Zeroizing;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
    services::{assistant_model_control as control, assistant_model_control_protocol as wire},
};

struct RequestContext<T> {
    chat: std::sync::Arc<crate::services::assistant_acknowledgement_service::ChatAuthority>,
    token: wire::SecretToken,
    value: T,
}
async fn context<T: DeserializeOwned>(
    auth: AuthUser,
    request: Request<Body>,
    max: usize,
) -> AppResult<RequestContext<T>> {
    let invalid = || {
        AppError::Forbidden(
            "A live owner conversation credential and model control capability are required".into(),
        )
    };
    if auth.auth_method != AuthMethod::ApiKey || auth.api_key_id.is_none() {
        return Err(invalid());
    }
    let chat = auth.assistant_chat.ok_or_else(invalid)?;
    if chat.guest
        || !chat.turn_live
        || chat.turn_stopped
        || chat.api_key_id != auth.api_key_id.as_deref().unwrap_or_default()
        || chat.user_id != auth.user_id.to_string()
    {
        return Err(invalid());
    }
    let mut values = request.headers().get_all(wire::CONTROL_HEADER).iter();
    let raw = values
        .next()
        .and_then(|h| h.to_str().ok())
        .ok_or_else(invalid)?;
    if values.next().is_some() {
        return Err(invalid());
    }
    let token = wire::SecretToken::parse(raw.into()).map_err(|_| invalid())?;
    let bytes = super::body_limit::read_request_body(request, max, "Model control").await?;
    let private = Zeroizing::new(bytes.to_vec());
    let value = serde_json::from_slice(&private)
        .map_err(|_| AppError::BadRequest("Invalid model control request".into()))?;
    Ok(RequestContext { chat, token, value })
}

fn private_reply<T: Serialize>(reply: T) -> Response {
    let mut response = Json(reply).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

pub async fn verify(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let request = context::<wire::VerifyRequest>(auth, request, 64 * 1024).await?;
    Ok(private_reply(
        control::verify(&state, &request.chat, &request.token, request.value).await?,
    ))
}
pub async fn prepare(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let request = context::<wire::PrepareRequest>(auth, request, wire::MAX_PREPARE_BYTES).await?;
    Ok(private_reply(
        control::prepare(&state, &request.chat, &request.token, request.value).await?,
    ))
}
pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let request = context::<wire::StatusRequest>(auth, request, 64 * 1024).await?;
    Ok(private_reply(
        control::status(&state, &request.chat, &request.token, request.value).await?,
    ))
}

pub async fn tool_prepare(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let request = context::<wire::ToolPrepareRequest>(auth, request, wire::MAX_ITEM_BYTES).await?;
    Ok(private_reply(
        control::tool_prepare(&state, &request.chat, &request.token, request.value).await?,
    ))
}
pub async fn tool_status(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let request = context::<wire::ToolStatusRequest>(auth, request, 64 * 1024).await?;
    Ok(private_reply(
        control::tool_status(&state, &request.chat, &request.token, request.value).await?,
    ))
}
