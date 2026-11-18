//! Enforce the complete-reply wire budget before consuming a reply token.
use super::channel_relay::{self, AsyncReplyRequest, AsyncReplyResponse};
use crate::{AppState, errors::AppError, mw::auth::OptionalAuthUser};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{FromRequest, Request, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};

pub struct ReplyJson(AsyncReplyRequest);

pub fn request_body_limit(media_max_bytes: u64) -> usize {
    crate::services::channel_media_service::request_body_limit(media_max_bytes)
        .max(crate::services::channel_reply_delivery::MAX_REPLY_REQUEST_BYTES)
}

impl FromRequest<AppState> for ReplyJson {
    type Rejection = Response;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        // Bytes honors the route's existing media limit, including chunked bodies.
        let (parts, body) = request.into_parts();
        let bytes = Bytes::from_request(Request::from_parts(parts.clone(), body), state)
            .await
            .map_err(IntoResponse::into_response)?;
        let wire_bytes = bytes.len();
        let Json(reply) = Json::<AsyncReplyRequest>::from_request(
            Request::from_parts(parts, Body::from(bytes)),
            state,
        )
        .await
        .map_err(IntoResponse::into_response)?;
        if reply.reply.attachments.is_empty()
            && wire_bytes > crate::services::channel_reply_delivery::MAX_REPLY_REQUEST_BYTES
        {
            return Err(AppError::RequestBodyTooLarge {
                max_bytes: crate::services::channel_reply_delivery::MAX_REPLY_REQUEST_BYTES,
                context: "channel reply".into(),
            }
            .into_response());
        }
        Ok(Self(reply))
    }
}

pub async fn reply(
    state: State<AppState>,
    headers: HeaderMap,
    auth: OptionalAuthUser,
    ReplyJson(body): ReplyJson,
) -> crate::errors::AppResult<Json<AsyncReplyResponse>> {
    Box::pin(channel_relay::async_reply(state, headers, auth, Json(body))).await
}

pub async fn thread_reply(
    state: State<AppState>,
    headers: HeaderMap,
    auth: OptionalAuthUser,
    ReplyJson(body): ReplyJson,
) -> crate::errors::AppResult<Json<AsyncReplyResponse>> {
    Box::pin(channel_relay::async_thread_reply(
        state,
        headers,
        auth,
        Json(body),
    ))
    .await
}

/// Distinct routes fail closed on old replicas instead of ignoring an opt-in.
pub fn mark_commonmark(body: &mut channel_relay::AsyncReplyBody) -> crate::errors::AppResult<()> {
    let metadata = body.metadata.get_or_insert_with(|| serde_json::json!({}));
    let object = metadata
        .as_object_mut()
        .ok_or_else(|| AppError::ValidationError("metadata must be an object".into()))?;
    if object.contains_key("entities") || object.contains_key("parse_mode") {
        return Err(AppError::ValidationError(
            "CommonMark cannot be combined with parse_mode or entities".into(),
        ));
    }
    object.insert("text_format".into(), "commonmark".into());
    Ok(())
}

pub async fn commonmark_reply(
    state: State<AppState>,
    headers: HeaderMap,
    auth: OptionalAuthUser,
    ReplyJson(mut body): ReplyJson,
) -> crate::errors::AppResult<Json<AsyncReplyResponse>> {
    mark_commonmark(&mut body.reply)?;
    Box::pin(channel_relay::async_reply(state, headers, auth, Json(body))).await
}

pub async fn commonmark_thread_reply(
    state: State<AppState>,
    headers: HeaderMap,
    auth: OptionalAuthUser,
    ReplyJson(mut body): ReplyJson,
) -> crate::errors::AppResult<Json<AsyncReplyResponse>> {
    mark_commonmark(&mut body.reply)?;
    Box::pin(channel_relay::async_thread_reply(
        state,
        headers,
        auth,
        Json(body),
    ))
    .await
}

pub async fn commonmark_update(
    state: State<AppState>,
    headers: HeaderMap,
    auth: OptionalAuthUser,
    Json(mut body): Json<channel_relay::UpdateReplyRequest>,
) -> crate::errors::AppResult<Json<channel_relay::UpdateReplyResponse>> {
    mark_commonmark(&mut body.reply)?;
    Box::pin(channel_relay::update_reply(
        state,
        headers,
        auth,
        Json(body),
    ))
    .await
}
