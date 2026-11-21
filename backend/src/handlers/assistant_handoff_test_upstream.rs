use crate::services::assistant_nyxagent as engine;
use axum::{
    Json, Router,
    http::HeaderMap,
    response::{IntoResponse, Sse, sse::Event},
    routing::post,
};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex;

#[derive(Default, Clone)]
pub(crate) struct Requests {
    capabilities: Arc<AtomicUsize>,
    models: Arc<AtomicUsize>,
}

impl Requests {
    pub(crate) fn counts(&self) -> (usize, usize) {
        (
            self.capabilities.load(Ordering::SeqCst),
            self.models.load(Ordering::SeqCst),
        )
    }

    pub(crate) fn router(&self, fail_models: bool) -> Router {
        let capabilities = self.capabilities.clone();
        let models = self.models.clone();
        Router::new().route("/v1/capabilities", axum::routing::get(move |headers: HeaderMap| async move {
            assert!(headers["authorization"].to_str().unwrap().starts_with("Bearer nyx"));
            capabilities.fetch_add(1, Ordering::SeqCst);
            Json(json!({"reattach":{"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"recent_events":32,"max_events":64,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32}}))
        })).route("/v1/models", axum::routing::get(move |headers: HeaderMap, uri: axum::http::Uri| async move {
            assert_eq!(uri.query(), Some("include_capabilities=true"));
            assert!(headers["authorization"].to_str().unwrap().starts_with("Bearer nyx"));
            models.fetch_add(1, Ordering::SeqCst);
            if fail_models { return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(); }
            Json(json!({"data":[{"id":engine::DEFAULT_MODEL,"inference_credential":"thread"}]})).into_response()
        }))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Cut {
    None,
    BeforeCreated,
    MidAnswer,
    Stop,
    Lost,
    Remove,
    Delete,
    DeletePending,
    TextReport,
}

pub(crate) const SESSION: &str = "conv_11111111111111111111111111111111";
pub(crate) const RESPONSE: &str =
    "resp_11111111111111111111111111111111_22222222222222222222222222222222";

fn snapshot(status: &str) -> Value {
    json!({"status":status,"conversation_id":SESSION,"response_id":RESPONSE,"last_sequence_number":2,
        "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":if status=="lost" {"failed"}else{status},
        "error":(status=="lost").then(||json!({"code":"executor_lost","tool_dispatch":"none"})),
        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer @NyxBot"}]}]}})
}

#[derive(Default, Clone)]
pub(crate) struct Upstream {
    pub(crate) discovery: Requests,
    pub(crate) posts: Arc<AtomicUsize>,
    pub(crate) posted: Arc<tokio::sync::Notify>,
    pub(crate) lookups: Arc<AtomicUsize>,
    pub(crate) cancels: Arc<AtomicUsize>,
    pub(crate) cancel_started: Arc<tokio::sync::Notify>,
    pub(crate) release_cancel: Arc<tokio::sync::Notify>,
    pub(crate) expected: Arc<Mutex<Option<(axum::http::HeaderValue, axum::http::HeaderValue)>>>,
}

impl Upstream {
    pub(crate) fn router(&self, cut: Cut, background: bool) -> Router {
        let posts = self.posts.clone();
        let posted = self.posted.clone();
        let captured = self.expected.clone();
        let expected = self.expected.clone();
        let lookups = self.lookups.clone();
        let cancels = self.cancels.clone();
        let cancel_started = self.cancel_started.clone();
        let release_cancel = self.release_cancel.clone();
        let cancel = post(move || {
            cancels.fetch_add(1, Ordering::SeqCst);
            let release_cancel = release_cancel.clone();
            cancel_started.notify_one();
            async move {
                if cut == Cut::DeletePending {
                    release_cancel.notified().await;
                    axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response()
                } else {
                    Json(snapshot("cancelled")).into_response()
                }
            }
        });
        self.discovery.router(false).route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let posts = posts.clone(); let posted = posted.clone(); let captured = captured.clone(); async move {
                assert_eq!(body["background"] == true, background);
                posts.fetch_add(1, Ordering::SeqCst);
                posted.notify_one();
                if background { *captured.lock().await = Some((headers["authorization"].clone(),headers["idempotency-key"].clone())); }
                let stream = async_stream::stream! {
                    if cut != Cut::BeforeCreated {
                        let created = json!({"type":"response.created","sequence_number":0,"response":{"id":RESPONSE,"conversation":{"id":SESSION}}});
                        yield Ok::<_, Infallible>(Event::default().data(created.to_string()));
                        let delta = json!({"type":"response.output_text.delta","sequence_number":1,"delta":"ans"});
                        yield Ok(Event::default().data(delta.to_string()));
                    }
                    if cut != Cut::None { std::future::pending::<()>().await; }
                    let terminal = json!({"type":"response.completed","sequence_number":2,"response":snapshot("completed")["response"]});
                    yield Ok(Event::default().data(terminal.to_string()));
                };
                Sse::new(stream)
            }
        })).route("/v1/responses/lookup", axum::routing::get(move |headers: HeaderMap| {
            let expected = expected.clone(); let lookups = lookups.clone(); async move {
                let held = expected.lock().await;
                let (credential,key) = held.as_ref().expect("execution POST");
                assert!(headers["authorization"] == credential && headers["idempotency-key"] == key);
                lookups.fetch_add(1,Ordering::SeqCst);
                Json(snapshot(match cut { Cut::Lost => "lost", Cut::Stop => "running", _ => "completed" }))
            }
        })).route(&format!("/v1/responses/{RESPONSE}/events"), axum::routing::get(|| async {
            ([("content-type","text/event-stream")],axum::body::Body::from_stream(futures::stream::pending::<Result<axum::body::Bytes, Infallible>>()))
        })).route("/v1/responses/cancel", cancel.clone())
            .route(&format!("/v1/responses/{RESPONSE}/cancel"), cancel)
    }
}
