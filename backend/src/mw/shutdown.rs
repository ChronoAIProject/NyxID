use std::{
    pin::Pin,
    task::{Context, Poll},
};

use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use hyper::body::{Body as HttpBody, Frame, SizeHint};

use crate::services::shutdown::{RecoverableSubscription, Runtime, Work};

pub async fn track(State(runtime): State<Runtime>, request: Request, next: Next) -> Response {
    if matches!(request.uri().path(), "/health" | "/ready") {
        return next.run(request).await;
    }
    let request_work = runtime.request();
    let response = tokio::select! {
        biased;
        () = runtime.stopped() => return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
        response = next.run(request) => response,
    };
    let (parts, body) = response.into_parts();
    let subscription = parts.extensions.get::<RecoverableSubscription>().is_some();
    let work = (!body.is_end_stream()).then(|| {
        if subscription {
            runtime.subscription()
        } else {
            runtime.stream()
        }
    });
    let stop = Box::pin(async move {
        if subscription {
            tokio::select! {
                () = runtime.subscription_close() => {},
                () = runtime.stopped() => {},
            }
        } else {
            runtime.stopped().await;
        }
    });
    drop(request_work);
    Response::from_parts(
        parts,
        Body::new(TrackedBody {
            body,
            work,
            stop,
            subscription,
        }),
    )
}

struct TrackedBody {
    body: Body,
    subscription: bool,
    work: Option<Work>,
    stop: Pin<Box<dyn std::future::Future<Output = ()> + Send + Sync>>,
}

impl HttpBody for TrackedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.stop.as_mut().poll(cx).is_ready() {
            self.body = Body::empty();
            if self.subscription {
                if let Some(work) = self.work.take() {
                    work.subscription_ended();
                }
                return Poll::Ready(None);
            }
            self.work.take();
            return Poll::Ready(Some(Err(axum::Error::new(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "server shutdown",
            )))));
        }
        let frame = Pin::new(&mut self.body).poll_frame(cx);
        if matches!(frame, Poll::Ready(None | Some(Err(_)))) || self.body.is_end_stream() {
            self.body = Body::empty();
            self.work.take();
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Notify;

    struct CleanupOnDrop {
        runtime: Runtime,
        complete: Arc<Notify>,
        end: bool,
    }

    impl HttpBody for CleanupOnDrop {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
            if self.end {
                Poll::Ready(None)
            } else {
                Poll::Pending
            }
        }
    }

    impl Drop for CleanupOnDrop {
        fn drop(&mut self) {
            let complete = self.complete.clone();
            self.runtime.spawn_cleanup(async move {
                complete.notified().await;
            });
        }
    }

    #[tokio::test]
    async fn shutdown_waits_for_response_body_and_its_drop_cleanup_before_releasing_ownership() {
        let runtime = Runtime::default();
        let complete = Arc::new(Notify::new());
        let mut body = TrackedBody {
            subscription: false,
            body: Body::new(CleanupOnDrop {
                runtime: runtime.clone(),
                complete: complete.clone(),
                end: false,
            }),
            work: Some(runtime.stream()),
            stop: Box::pin(runtime.stop_token().cancelled_owned()),
        };
        runtime.begin_drain();
        runtime.stop();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), runtime.tasks_stopped())
                .await
                .is_err()
        );
        assert!(body.frame().await.unwrap().is_err());
        assert_eq!(runtime.counts().streams, 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), runtime.tasks_stopped())
                .await
                .is_err()
        );
        complete.notify_one();
        tokio::time::timeout(Duration::from_secs(1), runtime.tasks_stopped())
            .await
            .unwrap();
        assert_eq!(runtime.tracked_units(), 0);
    }

    #[tokio::test]
    async fn shutdown_body_eof_registers_drop_cleanup_before_reporting_idle() {
        let runtime = Runtime::default();
        let complete = Arc::new(Notify::new());
        let mut body = TrackedBody {
            subscription: false,
            body: Body::new(CleanupOnDrop {
                runtime: runtime.clone(),
                complete: complete.clone(),
                end: true,
            }),
            work: Some(runtime.stream()),
            stop: Box::pin(runtime.stop_token().cancelled_owned()),
        };
        runtime.begin_drain();
        assert!(body.frame().await.is_none());
        assert_eq!(runtime.counts().streams, 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), runtime.idle())
                .await
                .is_err()
        );
        complete.notify_one();
        tokio::time::timeout(Duration::from_secs(1), runtime.idle())
            .await
            .unwrap();
        assert_eq!(runtime.tracked_units(), 0);
    }
}
