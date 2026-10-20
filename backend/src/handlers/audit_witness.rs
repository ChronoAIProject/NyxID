//! Independent sink HTTP adapter. No application-admin or enrollment routes.
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Serialize;

use crate::{
    models::audit_witness::*,
    services::audit_witness::{MAX_HTTP_BYTES, ReferenceWitness, WitnessError, WitnessResult},
};

#[derive(Serialize)]
struct SafeError {
    error: &'static str,
}

impl IntoResponse for WitnessError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid_request"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Conflict => (StatusCode::CONFLICT, "history_conflict"),
            _ => (StatusCode::SERVICE_UNAVAILABLE, "witness_unavailable"),
        };
        (
            status,
            [("cache-control", "no-store")],
            Json(SafeError { error }),
        )
            .into_response()
    }
}

pub fn router(witness: Arc<ReferenceWitness>) -> Router {
    Router::new()
        .route("/v1/advance", post(advance))
        .route("/v1/observe", post(observe))
        .route("/v1/history", post(history))
        .route("/v1/dispatch", post(dispatch))
        .layer(DefaultBodyLimit::max(MAX_HTTP_BYTES))
        .with_state(witness)
}

fn authenticate(witness: &ReferenceWitness, headers: &HeaderMap) -> WitnessResult<()> {
    let mut values = headers.get_all("authorization").iter();
    let value = values.next().ok_or(WitnessError::Unauthorized)?;
    if values.next().is_some() {
        return Err(WitnessError::Unauthorized);
    }
    let token = value
        .as_bytes()
        .strip_prefix(b"Bearer ")
        .ok_or(WitnessError::Unauthorized)?;
    witness.authenticate(token)
}

// Disk fsync and journal serialization happen on blocking threads, never on the
// async reactor. Responses are only emitted after the durable commit returns.
async fn run<T: Send + Serialize + 'static>(
    witness: Arc<ReferenceWitness>,
    headers: HeaderMap,
    f: impl FnOnce(&ReferenceWitness) -> WitnessResult<T> + Send + 'static,
) -> WitnessResult<impl IntoResponse> {
    authenticate(&witness, &headers)?;
    let value = tokio::task::spawn_blocking(move || f(&witness))
        .await
        .map_err(|_| WitnessError::Unavailable)??;
    Ok(([("cache-control", "no-store")], Json(value)))
}

async fn advance(
    State(w): State<Arc<ReferenceWitness>>,
    h: HeaderMap,
    Json(r): Json<AdvanceRequest>,
) -> WitnessResult<impl IntoResponse> {
    run(w, h, move |w| w.advance(r)).await
}
async fn observe(
    State(w): State<Arc<ReferenceWitness>>,
    h: HeaderMap,
    Json(r): Json<ObserveRequest>,
) -> WitnessResult<impl IntoResponse> {
    run(w, h, move |w| w.observe(r)).await
}
async fn history(
    State(w): State<Arc<ReferenceWitness>>,
    h: HeaderMap,
    Json(r): Json<HistoryRequest>,
) -> WitnessResult<impl IntoResponse> {
    run(w, h, move |w| w.history(r)).await
}
async fn dispatch(
    State(w): State<Arc<ReferenceWitness>>,
    h: HeaderMap,
    Json(r): Json<DispatchRequest>,
) -> WitnessResult<impl IntoResponse> {
    run(w, h, move |w| w.claim_dispatch(r)).await
}
