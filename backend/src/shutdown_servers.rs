use std::{future::IntoFuture, io};

use axum::Router;
use futures::{StreamExt, stream::FuturesUnordered};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{downstream_disconnect, services::shutdown::Runtime};

/// Each graceful attempt stops accepting but retains the bound sockets. If an
/// accepted request starts work after the idle observation, a new serving pair
/// handles its callbacks while the previous pair finishes its connections.
pub(crate) struct Servers {
    runtime: Runtime,
    public: downstream_disconnect::SharedListener,
    internal: downstream_disconnect::SharedListener,
    app: Router,
    internal_app: Router,
    accepts: CancellationToken,
    tasks: FuturesUnordered<JoinHandle<io::Result<()>>>,
}

impl Drop for Servers {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Servers {
    pub(crate) fn new(
        runtime: Runtime,
        public: TcpListener,
        internal: TcpListener,
        app: Router,
        internal_app: Router,
    ) -> Self {
        let mut servers = Self {
            runtime,
            public: public.into(),
            internal: internal.into(),
            app,
            internal_app,
            accepts: CancellationToken::new(),
            tasks: FuturesUnordered::new(),
        };
        servers.resume();
        servers
    }

    pub(crate) fn resume(&mut self) {
        self.accepts = CancellationToken::new();
        let public = axum::serve(
            downstream_disconnect::DisconnectAwareListener::new(self.public.clone()),
            downstream_disconnect::DisconnectAwareMakeService::new(self.app.clone()),
        )
        .with_graceful_shutdown(self.accepts.clone().cancelled_owned())
        .into_future();
        let internal = axum::serve(self.internal.clone(), self.internal_app.clone())
            .with_graceful_shutdown(self.accepts.clone().cancelled_owned())
            .into_future();
        self.tasks.push(self.runtime.spawn_worker(public));
        self.tasks.push(self.runtime.spawn_worker(internal));
    }

    pub(crate) fn quiesce(&self) {
        self.accepts.cancel();
    }

    /// Completed graceful epochs are normal; a serving listener returning is
    /// a failure. Neither completed handles nor cancelled futures are repolled.
    pub(crate) async fn failed(&mut self) {
        while let Some(result) = self.tasks.next().await {
            match result {
                Ok(Ok(())) if !self.tasks.is_empty() => continue,
                Ok(Err(error)) => tracing::error!(%error, "HTTP listener failed"),
                Err(error) => tracing::error!(%error, "HTTP listener task failed"),
                Ok(Ok(())) => {}
            }
            return;
        }
    }

    /// True only after Hyper finished every accepted connection and no request
    /// from those connections registered additional work. New TCP connections
    /// wait in the retained listener's backlog during this short observation.
    pub(crate) async fn finish_idle(&mut self) -> bool {
        self.quiesce();
        while !self.tasks.is_empty() {
            tokio::select! {
                biased;
                () = self.runtime.serving_needed() => return false,
                result = self.tasks.next() => {
                    if !matches!(result, Some(Ok(Ok(())))) {
                        tracing::warn!("HTTP connection drain failed");
                    }
                }
            }
        }
        self.runtime.counts().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, middleware, routing::get};
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Notify;

    #[tokio::test]
    async fn shutdown_idle_race_resumes_both_listeners_until_accepted_child_finishes() {
        let runtime = Runtime::default();
        let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let public_url = format!("http://{}", public.local_addr().unwrap());
        let internal_url = format!("http://{}", internal.local_addr().unwrap());
        let entered = Arc::new(Notify::new());
        let admit = Arc::new(Notify::new());
        let call_back = Arc::new(Notify::new());
        let child_done = Arc::new(Notify::new());
        let child_runtime = runtime.clone();
        let child_public = public_url.clone();
        let child_internal = internal_url.clone();
        let child_signal = call_back.clone();
        let done_signal = child_done.clone();
        let app = Router::new()
            .route(
                "/start",
                get(move || {
                    let runtime = child_runtime.clone();
                    let public = child_public.clone();
                    let internal = child_internal.clone();
                    let call_back = child_signal.clone();
                    let done = done_signal.clone();
                    async move {
                        runtime.spawn(async move {
                            call_back.notified().await;
                            for url in [public, internal] {
                                let reply = reqwest::get(format!("{url}/callback")).await.unwrap();
                                assert_eq!(reply.status(), axum::http::StatusCode::OK);
                                assert_eq!(reply.text().await.unwrap(), "callback complete");
                            }
                            done.notify_one();
                        });
                        "accepted"
                    }
                }),
            )
            .route("/callback", get(|| async { "callback complete" }))
            .layer(middleware::from_fn_with_state(
                runtime.clone(),
                crate::mw::shutdown::track,
            ))
            .layer(middleware::from_fn_with_state(
                (entered.clone(), admit.clone()),
                |State((entered, admit)): State<(Arc<Notify>, Arc<Notify>)>,
                 request: axum::extract::Request,
                 next: middleware::Next| async move {
                    if request.uri().path() == "/start" {
                        entered.notify_one();
                        admit.notified().await;
                    }
                    next.run(request).await
                },
            ));
        let mut servers = Servers::new(runtime.clone(), public, internal, app.clone(), app);
        let request = tokio::spawn(async move {
            reqwest::get(format!("{public_url}/start"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        entered.notified().await;
        runtime.begin_drain();
        assert!(runtime.counts().is_empty());
        {
            let closing = servers.finish_idle();
            tokio::pin!(closing);
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut closing)
                    .await
                    .is_err()
            );
            admit.notify_one();
            assert!(
                !tokio::time::timeout(Duration::from_secs(2), closing)
                    .await
                    .unwrap()
            );
        }
        servers.resume();
        assert_eq!(request.await.unwrap(), "accepted");
        call_back.notify_one();
        tokio::time::timeout(Duration::from_secs(2), child_done.notified())
            .await
            .unwrap();
        runtime.idle().await;
        assert!(
            tokio::time::timeout(Duration::from_secs(2), servers.finish_idle())
                .await
                .unwrap()
        );
        runtime.stop();
        runtime.tasks_stopped().await;
        assert_eq!(runtime.tracked_units(), 0);
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;
    use axum::{
        middleware,
        response::{IntoResponse, Sse, sse::Event},
        routing::get,
    };
    use futures::StreamExt;
    use std::{convert::Infallible, time::Duration};

    #[tokio::test]
    async fn shutdown_idle_listener_resumes_for_subscription_admitted_during_drain_grace() {
        let runtime = Runtime::default();
        let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", public.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/subscription",
                get(|| async {
                    let stream = futures::stream::iter([Ok::<_, Infallible>(
                        Event::default().data("ready"),
                    )])
                    .chain(futures::stream::pending());
                    let mut response = Sse::new(stream).into_response();
                    response
                        .extensions_mut()
                        .insert(crate::services::shutdown::RecoverableSubscription);
                    response
                }),
            )
            .route("/request", get(|| async { "accepted" }))
            .layer(middleware::from_fn_with_state(
                runtime.clone(),
                crate::mw::shutdown::track,
            ));
        let mut servers = Servers::new(runtime.clone(), public, internal, app.clone(), app);
        runtime.begin_drain();
        let subscription = reqwest::get(format!("{url}/subscription")).await.unwrap();
        assert!(runtime.counts().is_empty());
        assert_eq!(runtime.counts().subscriptions, 1);
        assert!(
            !tokio::time::timeout(Duration::from_secs(1), servers.finish_idle())
                .await
                .unwrap()
        );
        servers.resume();
        assert_eq!(
            reqwest::get(format!("{url}/request"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "accepted"
        );
        assert!(
            tokio::time::timeout(
                crate::services::shutdown::SUBSCRIPTION_DRAIN_GRACE + Duration::from_secs(1),
                subscription.text()
            )
            .await
            .unwrap()
            .unwrap()
            .contains("ready")
        );
        runtime.subscriptions_finished().await;
        assert!(servers.finish_idle().await);
        runtime.stop();
        runtime.tasks_stopped().await;
        assert_eq!(runtime.tracked_units(), 0);
    }
}
