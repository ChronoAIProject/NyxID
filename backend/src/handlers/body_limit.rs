use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, header};

use crate::errors::{AppError, AppResult};

pub(crate) async fn read_request_body(
    request: Request<Body>,
    max_bytes: usize,
    context: &'static str,
) -> AppResult<Bytes> {
    let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|declared_bytes| declared_bytes > max_bytes_u64)
    {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes,
            context: context.to_string(),
        });
    }

    read_body(request.into_body(), max_bytes, context).await
}

/// Read a body after its request parts have already been split. Callers using
/// this form cannot preflight Content-Length but remain bounded by `max_bytes`.
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

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::task::Poll;

    use super::*;
    #[tokio::test]
    async fn declared_oversize_is_rejected_without_polling_body() {
        let polled = Arc::new(AtomicBool::new(false));
        let stream_polled = polled.clone();
        let stream = futures::stream::poll_fn(move |_| {
            stream_polled.store(true, Ordering::SeqCst);
            Poll::Ready(None::<Result<Bytes, Infallible>>)
        });
        let request = Request::builder()
            .header(header::CONTENT_LENGTH, "5")
            .body(Body::from_stream(stream))
            .unwrap();

        let error = read_request_body(request, 4, "Proxy")
            .await
            .expect_err("declared oversize body must fail");

        assert!(matches!(
            error,
            AppError::RequestBodyTooLarge { max_bytes: 4, .. }
        ));
        assert!(!polled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unparseable_content_length_falls_through_to_bounded_read() {
        let request = Request::builder()
            .header(header::CONTENT_LENGTH, "not-a-number")
            .body(Body::from("ok"))
            .unwrap();

        let body = read_request_body(request, 4, "Proxy").await.unwrap();

        assert_eq!(body, "ok");
    }
}
