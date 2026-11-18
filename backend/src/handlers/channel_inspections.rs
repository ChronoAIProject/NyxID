//! HMAC-authenticated, metadata-only production inspection API.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::services::channel_inspection_service::{self, DispatchRequest, InspectionStatus};

#[derive(Serialize)]
pub struct DispatchResponse {
    pub protocol_version: u8,
    pub dispatch_id: String,
    pub case_id: String,
    pub run_id: String,
    pub transport: &'static str,
    pub phase: &'static str,
    pub idempotent_replay: bool,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub protocol_version: u8,
    pub dispatch_id: String,
    pub case_id: String,
    pub run_id: String,
    pub transport: &'static str,
    pub phase: &'static str,
    pub terminal: bool,
    pub evidence_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_admitted: Option<bool>,
    pub correlated_outbound_count: u8,
    pub provider_accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub deadline_at: String,
}

/// POST /api/v1/channel-inspections/{case_id}/dispatch
pub async fn dispatch(
    State(state): State<AppState>,
    Path(case_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<impl IntoResponse> {
    let runtime = state
        .channel_inspection
        .as_deref()
        .ok_or_else(|| AppError::NotFound("channel inspection is not configured".into()))?;
    let path = format!("/api/v1/channel-inspections/{case_id}/dispatch");
    if !runtime.authenticate(&headers, "POST", &path, &body).await {
        return Err(AppError::Unauthorized(
            "inspection authentication failed".into(),
        ));
    }
    let request: DispatchRequest = serde_json::from_slice(&body)
        .map_err(|_| AppError::ValidationError("invalid inspection request".into()))?;
    let accepted =
        channel_inspection_service::dispatch(&state, runtime, &case_id, &request).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(DispatchResponse {
            protocol_version: channel_inspection_service::PROTOCOL_VERSION,
            dispatch_id: accepted.dispatch_id,
            case_id: accepted.case_id,
            run_id: accepted.run_id,
            transport: "synthetic_ingress",
            phase: "dispatched",
            idempotent_replay: accepted.idempotent_replay,
        }),
    ))
}

/// GET /api/v1/channel-inspections/{dispatch_id}/status
pub async fn status(
    State(state): State<AppState>,
    Path(dispatch_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<StatusResponse>> {
    let runtime = state
        .channel_inspection
        .as_deref()
        .ok_or_else(|| AppError::NotFound("channel inspection is not configured".into()))?;
    let path = format!("/api/v1/channel-inspections/{dispatch_id}/status");
    if !runtime.authenticate(&headers, "GET", &path, &[]).await {
        return Err(AppError::Unauthorized(
            "inspection authentication failed".into(),
        ));
    }
    let status = channel_inspection_service::status(&state.db, runtime, &dispatch_id).await?;
    Ok(Json(status_response(status)))
}

fn status_response(status: InspectionStatus) -> StatusResponse {
    StatusResponse {
        protocol_version: channel_inspection_service::PROTOCOL_VERSION,
        dispatch_id: status.dispatch_id,
        case_id: status.case_id,
        run_id: status.run_id,
        transport: "synthetic_ingress",
        phase: status.phase,
        terminal: status.terminal,
        evidence_complete: status.evidence_complete,
        outcome: status.outcome,
        callback_admitted: status.callback_admitted,
        correlated_outbound_count: status.correlated_outbound_count,
        provider_accepted: status.provider_accepted,
        latency_ms: status.latency_ms,
        deadline_at: status.deadline_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_dto_contains_no_routing_identifiers() {
        let response = status_response(InspectionStatus {
            dispatch_id: uuid::Uuid::new_v4().to_string(),
            case_id: "telegram_private".into(),
            run_id: "run-1".into(),
            phase: "terminal",
            terminal: true,
            evidence_complete: true,
            outcome: Some("ok"),
            callback_admitted: Some(true),
            correlated_outbound_count: 1,
            provider_accepted: true,
            latency_ms: Some(42),
            deadline_at: chrono::Utc::now(),
        });
        let value = serde_json::to_value(response).unwrap();
        for forbidden in [
            "conversation_id",
            "channel_bot_id",
            "bot_id",
            "platform_id",
            "user_id",
            "agent_api_key_id",
            "inbound_message_id",
            "platform_message_id",
        ] {
            assert!(value.get(forbidden).is_none(), "leaked {forbidden}");
        }
    }
}
