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
