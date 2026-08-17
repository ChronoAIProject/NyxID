use axum::body::{Body, Bytes, to_bytes};
use axum::http::Request;

use crate::errors::{AppError, AppResult};

pub(crate) async fn read_request_body(
    request: Request<Body>,
    max_bytes: usize,
    context: &'static str,
) -> AppResult<Bytes> {
    read_body(request.into_body(), max_bytes, context).await
}

pub(crate) async fn read_body(
    body: Body,
    max_bytes: usize,
    context: &'static str,
) -> AppResult<Bytes> {
    to_bytes(body, max_bytes).await.map_err(|error| {
        if caused_by_length_limit(&error) {
            AppError::RequestBodyTooLarge {
                max_bytes,
                context: context.to_string(),
            }
        } else {
            AppError::BadRequest(format!("Failed to read {context} request body"))
        }
    })
}

fn caused_by_length_limit(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(source) = current {
        if source.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        current = source.source();
    }
    false
}
