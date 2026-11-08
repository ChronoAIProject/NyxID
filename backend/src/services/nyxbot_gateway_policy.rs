//! CMAEG authors routing separately from the channel definition. Keep the
//! desired policy for acknowledgement, but never serialize it as a channel PUT.
//! This applies equally to native Discord and WhatsApp; relay bodies are unchanged.
use super::nyxbot_gateway_service::{GatewayResponse, gateway_call_at_base};
use crate::errors::AppResult;
use serde_json::{Value, json};

pub(crate) fn channel_request(policy: &Value) -> Value {
    let mut request = policy.clone();
    if let Some(fields) = request.as_object_mut() {
        fields.remove("routing");
    }
    request
}

/// A routing write is the only management write with this exact retry contract:
/// the immediately following identical revision confirms a lost response. Never
/// advance expected_version on a retry or retry an HTTP refusal (especially 409).
pub(crate) async fn put_routing_at_base(
    base: &str,
    creator: &str,
    channel_id: &str,
    expected_version: i64,
    routing: &Value,
) -> AppResult<GatewayResponse> {
    let id: String = url::form_urlencoded::byte_serialize(channel_id.as_bytes()).collect();
    let path = format!("/channels/{id}/routing");
    let body = json!({"expected_version":expected_version,"routing":routing});
    let send = || {
        gateway_call_at_base(
            base,
            reqwest::Method::PUT,
            &path,
            creator,
            Some(&body),
            None,
        )
    };
    match send().await {
        Ok(response) => Ok(response),
        Err(_) => {
            tracing::info!(
                operation = "routing_update",
                reason = "response_unavailable",
                "NyxBot gateway policy response unavailable; confirming exact revision once"
            );
            send().await
        }
    }
}

pub(crate) fn next_version(body: &Value, expected_version: i64) -> Option<i64> {
    body["version"]
        .as_i64()
        .filter(|version| *version > 0 && expected_version.checked_add(1) == Some(*version))
}
