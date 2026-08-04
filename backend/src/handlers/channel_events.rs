//! HTTP Event Gateway handler.
//!
//! Accepts device event envelopes from external analyzers/devices and forwards
//! them to the agent bound to the target conversation. See
//! `services/channel_event_service.rs` for orchestration details, and
//! `docs/CHANNEL_EVENT_GATEWAY.md` for the design.

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::channel_event_service::{self, EventEnvelope, ForwardOutcome};
use crate::telemetry::TelemetryContext;

const MAX_ID_LEN: usize = 64;
const MAX_LABEL_LEN: usize = 128;

#[derive(Debug, Serialize)]
pub struct EventAcceptedResponse {
    pub status: &'static str,
    pub event_id: String,
}

/// POST /api/v1/channel-events/{conversation_id}
///
/// Accepts a device event envelope and forwards it through the channel relay
/// pipeline as a `CallbackPayload` with `platform = "device"`. See
/// NyxID#221 + ADR-013 for background.
pub async fn post_event(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(envelope): Json<EventEnvelope>,
) -> AppResult<Json<EventAcceptedResponse>> {
    // Auth: this endpoint only accepts a genuine `nyxid_ag_...` API key.
    //
    // Checking `api_key_id.is_some()` alone is not sufficient because relay
    // JWTs issued to bot callback recipients also carry an api_key_id in
    // their claims. If we allowed them through, a webhook callback target
    // could reuse its `X-NyxID-User-Token` as Bearer auth and synthesize
    // device events on any conversation it learned about — a privilege
    // escalation beyond the relay token's intent. Gate strictly on
    // `AuthMethod::ApiKey`.
    if auth_user.auth_method != AuthMethod::ApiKey {
        return Err(AppError::Unauthorized(
            "API key required for channel events".to_string(),
        ));
    }

    // Envelope shape validation. Per design doc §NOT in Scope, there is no
    // payload-size limit; only structural validation.
    validate_envelope(&envelope)?;

    let outcome = channel_event_service::forward_event(
        &state.db,
        &state.http_client,
        &state.config,
        &state.jwt_keys,
        &state.per_channel_event_limiter,
        &state.event_dedup_cache,
        state.telemetry.as_deref(),
        &tele,
        &auth_user,
        &conversation_id,
        &envelope,
    )
    .await?;

    let status = match outcome {
        ForwardOutcome::Delivered => "accepted",
        ForwardOutcome::Deduped => "duplicate",
    };

    Ok(Json(EventAcceptedResponse {
        status,
        event_id: envelope.event_id,
    }))
}

fn validate_envelope(envelope: &EventEnvelope) -> AppResult<()> {
    if envelope.event_id.is_empty() || envelope.event_id.len() > MAX_ID_LEN {
        return Err(AppError::ValidationError(
            "event_id must be 1-64 characters".to_string(),
        ));
    }
    if uuid::Uuid::parse_str(&envelope.event_id).is_err() {
        return Err(AppError::ValidationError(
            "event_id must be a UUID".to_string(),
        ));
    }
    if !is_valid_label(&envelope.source) {
        return Err(AppError::ValidationError(
            "source must be 1-128 chars matching [a-zA-Z0-9_\\-./]+".to_string(),
        ));
    }
    if !is_valid_type(&envelope.event_type) {
        return Err(AppError::ValidationError(
            "type must be 1-128 chars matching [a-zA-Z0-9_\\-.:]+".to_string(),
        ));
    }
    Ok(())
}

fn is_valid_label(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_LABEL_LEN {
        return false;
    }
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
}

fn is_valid_type(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_LABEL_LEN {
        return false;
    }
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn envelope() -> EventEnvelope {
        EventEnvelope {
            event_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            source: "camera-analyzer".to_string(),
            event_type: "person_detected".to_string(),
            timestamp: Utc::now(),
            payload: None,
            metadata: None,
        }
    }

    #[test]
    fn valid_envelope_passes() {
        assert!(validate_envelope(&envelope()).is_ok());
    }

    #[test]
    fn rejects_non_uuid_event_id() {
        let mut env = envelope();
        env.event_id = "not-a-uuid".to_string();
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn rejects_empty_source() {
        let mut env = envelope();
        env.source = String::new();
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn rejects_source_with_bad_chars() {
        let mut env = envelope();
        env.source = "camera analyzer".to_string(); // space
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn rejects_type_with_bad_chars() {
        let mut env = envelope();
        env.event_type = "person detected!".to_string();
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn rejects_oversized_source() {
        let mut env = envelope();
        env.source = "a".repeat(MAX_LABEL_LEN + 1);
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn accepts_colon_in_type() {
        let mut env = envelope();
        env.event_type = "sensor:temperature".to_string();
        assert!(validate_envelope(&env).is_ok());
    }

    #[test]
    fn rejects_empty_event_id() {
        let mut env = envelope();
        env.event_id = String::new();
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn rejects_oversized_event_id() {
        let mut env = envelope();
        env.event_id = "a".repeat(MAX_ID_LEN + 1);
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn rejects_empty_type() {
        let mut env = envelope();
        env.event_type = String::new();
        assert!(validate_envelope(&env).is_err());
    }

    #[test]
    fn is_valid_label_accepts_alphanumeric_and_special() {
        assert!(is_valid_label("camera-01/main_feed"));
        assert!(is_valid_label("sensor.temp"));
        assert!(!is_valid_label("has space"));
        assert!(!is_valid_label(""));
    }

    #[test]
    fn is_valid_type_accepts_colons_and_dots() {
        assert!(is_valid_type("sensor:temperature.celsius"));
        assert!(!is_valid_type("bad type!"));
        assert!(!is_valid_type(""));
    }
}
