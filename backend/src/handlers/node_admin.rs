use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use base64::Engine;
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult, PENDING_CREDENTIAL_NODE_OFFLINE_CODE};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::node::NodeMetadata;
use crate::models::node_pending_credential::{
    FanOutNodeState, InjectionMethod, NodePendingCredential, RemoteCryptoState,
};
use crate::mw::auth::AuthUser;
use crate::services::node_pending_credential_service::{
    IntegrityVerificationAudit, PendingCredentialIntegrityVerificationRequest,
};
use crate::services::{
    audit_service, node_pending_credential_service, node_routing_service, node_service,
    org_service,
    rci_audit_service::{self, RciAuditDelivery, RciAuditEventKind, RciAuditSubject},
};
use crate::telemetry::{
    context::{TelemetryContext, emit_event},
    sampling::hash_short_id,
    schema::TelemetryEvent,
};

// NodeCredentialConfigured is emitted from the nyxid CLI, not backend -- see TELEMETRY.md §6.5

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct CreateRegistrationTokenRequest {
    pub name: String,
    pub owner_user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBindingRequest {
    pub service_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBindingRequest {
    pub priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct TransferNodeRequest {
    pub new_owner_user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct PushPendingCredentialRequest {
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub remote_crypto: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct PushPendingCredentialFanOutRequest {
    pub owner_user_id: String,
    pub service_id: String,
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub remote_crypto: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct PendingCredentialCiphertextRequest {
    pub version: String,
    pub admin_pubkey: String,
    pub nonce: String,
    pub ciphertext: String,
    #[serde(default)]
    pub integrity_verification: Option<PendingCredentialIntegrityVerificationRequest>,
}

#[derive(Debug, Deserialize)]
pub struct PendingCredentialFanOutCiphertextItemRequest {
    pub node_id: String,
    pub generation: i64,
    pub version: String,
    pub admin_pubkey: String,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Deserialize)]
pub struct PendingCredentialFanOutCiphertextRequest {
    pub fan_out_revision: i64,
    pub items: Vec<PendingCredentialFanOutCiphertextItemRequest>,
    #[serde(default)]
    pub integrity_verification: Option<PendingCredentialIntegrityVerificationRequest>,
}

#[derive(Debug, Deserialize)]
pub struct RetryFanOutPendingCredentialRequest {
    pub fan_out_revision: i64,
}

#[derive(Debug, Deserialize, Default)]
pub struct PendingCredentialListQuery {
    pub include_history: Option<bool>,
}

#[derive(Debug, PartialEq, Eq)]
enum PendingCiphertextValidation {
    Valid(Vec<u8>),
    TooLarge,
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct CreateRegistrationTokenResponse {
    pub token_id: String,
    pub token: String,
    pub name: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct NodeListResponse {
    pub nodes: Vec<NodeInfo>,
}

#[derive(Debug, Serialize)]
pub struct NodeMetricsInfo {
    pub total_requests: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub owner: node_service::NodeOwnerInfo,
    pub status: String,
    pub is_connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<NodeMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<NodeMetricsInfo>,
    pub binding_count: u64,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct RotateTokenResponse {
    pub auth_token: String,
    pub signing_secret: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct BindingListResponse {
    pub bindings: Vec<BindingInfo>,
}

#[derive(Debug, Serialize)]
pub struct BindingInfo {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    pub service_slug: String,
    pub is_active: bool,
    pub priority: i32,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct CreateBindingResponse {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct NodeAdminsResponse {
    pub admins: Vec<node_service::NodeAdminInfo>,
}

#[derive(Debug, Serialize)]
pub struct TransferNodeResponse {
    pub node_id: String,
    pub previous_owner: node_service::NodeOwnerInfo,
    pub new_owner: node_service::NodeOwnerInfo,
    pub deactivated_bindings_count: u64,
    pub cleared_user_service_count: u64,
}

#[derive(Debug, Serialize)]
pub struct PendingCredentialInfo {
    pub id: String,
    pub node_id: String,
    pub service_slug: String,
    pub injection_method: String,
    pub field_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_by_user_id: String,
    pub owner_user_id: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declined_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_state: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct PendingCredentialListResponse {
    pub pending_credentials: Vec<PendingCredentialInfo>,
}

#[derive(Debug, Serialize)]
pub struct PendingCredentialPubkeyResponse {
    pub pending_id: String,
    pub node_id: String,
    pub service_slug: String,
    pub version: String,
    pub node_pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_state: Option<String>,
    pub integrity_verification_opt_out: bool,
}

#[derive(Debug, Serialize)]
pub struct PendingCredentialCiphertextResponse {
    pub delivery_status: String,
    pub remote_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct FanOutTargetInfo {
    pub node_id: String,
    pub generation: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FanOutPendingCredentialResponse {
    pub fanout_id: String,
    pub fan_out_revision: i64,
    pub target_count: usize,
    pub service_slug: String,
    pub injection_method: String,
    pub field_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_state: Option<String>,
    pub targets: Vec<FanOutTargetInfo>,
}

#[derive(Debug, Serialize)]
pub struct FanOutPendingCredentialPubkeyTarget {
    pub node_id: String,
    pub generation: i64,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct FanOutPendingCredentialPubkeysResponse {
    pub fanout_id: String,
    pub fan_out_revision: i64,
    pub target_count: usize,
    pub integrity_verification_opt_out: bool,
    pub targets: Vec<FanOutPendingCredentialPubkeyTarget>,
}

#[derive(Debug, Serialize)]
pub struct FanOutPendingCredentialCiphertextResponse {
    pub fanout_id: String,
    pub fan_out_revision: i64,
    pub remote_state: String,
    pub targets: Vec<FanOutTargetInfo>,
}

// --- Helpers ---

/// Build NodeMetricsInfo from the embedded metrics on a Node model.
pub fn build_metrics_info(metrics: &crate::models::node::NodeMetrics) -> NodeMetricsInfo {
    let success_rate = if metrics.total_requests > 0 {
        metrics.success_count as f64 / metrics.total_requests as f64
    } else {
        0.0
    };

    NodeMetricsInfo {
        total_requests: metrics.total_requests,
        success_count: metrics.success_count,
        error_count: metrics.error_count,
        success_rate,
        avg_latency_ms: metrics.avg_latency_ms,
        last_error: metrics.last_error.clone(),
        last_error_at: metrics.last_error_at.map(|dt| dt.to_rfc3339()),
        last_success_at: metrics.last_success_at.map(|dt| dt.to_rfc3339()),
    }
}

fn audit_event_data_with_owner(
    actor_user_id: &str,
    owner_user_id: &str,
    mut event_data: serde_json::Value,
) -> serde_json::Value {
    if actor_user_id != owner_user_id
        && let serde_json::Value::Object(ref mut object) = event_data
    {
        object.insert(
            "owner_user_id".to_string(),
            serde_json::Value::String(owner_user_id.to_string()),
        );
    }
    event_data
}

fn transfer_audit_event_data(
    actor_user_id: &str,
    result: &node_service::TransferNodeResult,
) -> serde_json::Value {
    audit_event_data_with_owner(
        actor_user_id,
        &result.new_owner_user_id,
        serde_json::json!({
            "actor_user_id": actor_user_id,
            "node_id": &result.node_id,
            "previous_owner_user_id": &result.previous_owner_user_id,
            "new_owner_user_id": &result.new_owner_user_id,
            "deactivated_bindings_count": result.deactivated_bindings_count,
            "cleared_user_service_count": result.cleared_user_service_count,
            "deactivated_pending_credentials_count": result.deactivated_pending_credentials_count,
        }),
    )
}

fn pending_credential_info(pending: NodePendingCredential) -> PendingCredentialInfo {
    let remote_state = pending_remote_state(&pending);
    PendingCredentialInfo {
        id: pending.id,
        node_id: pending.node_id,
        service_slug: pending.service_slug,
        injection_method: pending.injection_method.as_str().to_string(),
        field_name: pending.field_name,
        target_url: pending.target_url,
        label: pending.label,
        created_by_user_id: pending.created_by_user_id,
        owner_user_id: pending.owner_user_id,
        created_at: pending.created_at.to_rfc3339(),
        expires_at: pending.expires_at.to_rfc3339(),
        consumed_at: pending.consumed_at.map(|dt| dt.to_rfc3339()),
        declined_at: pending.declined_at.map(|dt| dt.to_rfc3339()),
        remote_state,
        is_active: pending.is_active,
    }
}

fn remote_state_name(state: &RemoteCryptoState) -> &'static str {
    match state {
        RemoteCryptoState::PubkeyAwaiting => "pubkey_awaiting",
        RemoteCryptoState::PubkeyPosted => "pubkey_posted",
        RemoteCryptoState::CiphertextReceived => "ciphertext_received",
        RemoteCryptoState::CiphertextQueued => "ciphertext_queued",
        RemoteCryptoState::Consumed => "consumed",
        RemoteCryptoState::PartialDecrypted => "partial_decrypted",
        RemoteCryptoState::DecryptFailed => "decrypt_failed",
        RemoteCryptoState::Expired => "expired",
        RemoteCryptoState::Declined => "declined",
    }
}

fn pending_remote_state(pending: &NodePendingCredential) -> Option<String> {
    pending
        .remote_state
        .as_ref()
        .map(remote_state_name)
        .map(str::to_string)
}

fn pending_pubkey_response(
    pending: NodePendingCredential,
    integrity_verification_opt_out: bool,
) -> AppResult<PendingCredentialPubkeyResponse> {
    let (version, node_pubkey) = {
        let crypto = pending
            .crypto
            .as_ref()
            .ok_or_else(|| AppError::PendingCredentialPubkeyAwaiting(pending.id.clone()))?;
        if crypto.node_pubkey.is_empty() {
            return Err(AppError::PendingCredentialPubkeyAwaiting(pending.id));
        }
        (crypto.version.clone(), crypto.node_pubkey.clone())
    };
    let remote_state = pending_remote_state(&pending);

    Ok(PendingCredentialPubkeyResponse {
        pending_id: pending.id,
        node_id: pending.node_id,
        service_slug: pending.service_slug,
        version,
        node_pubkey,
        remote_state,
        integrity_verification_opt_out,
    })
}

fn fan_out_target_info(
    target: &node_pending_credential_service::FanOutTargetStatus,
) -> FanOutTargetInfo {
    FanOutTargetInfo {
        node_id: target.node_id.clone(),
        generation: target.generation,
        remote_state: target
            .remote_state
            .as_ref()
            .map(remote_state_name)
            .map(str::to_string),
        delivery_status: target
            .delivery_status
            .map(|status| status.as_str().to_string()),
        error_code: target.error_code,
        error_kind: target.error_kind.clone(),
    }
}

fn fan_out_pending_response(
    result: node_pending_credential_service::FanOutPendingCredentialResult,
) -> FanOutPendingCredentialResponse {
    let remote_state = pending_remote_state(&result.pending);
    FanOutPendingCredentialResponse {
        fanout_id: result.pending.id,
        fan_out_revision: result.pending.fan_out_revision,
        target_count: result.targets.len(),
        service_slug: result.pending.service_slug,
        injection_method: result.pending.injection_method.as_str().to_string(),
        field_name: result.pending.field_name,
        target_url: result.pending.target_url,
        label: result.pending.label,
        remote_state,
        targets: result.targets.iter().map(fan_out_target_info).collect(),
    }
}

fn fan_out_status_response(pending: NodePendingCredential) -> FanOutPendingCredentialResponse {
    let targets = pending
        .fan_out_nodes
        .iter()
        .map(
            |target| node_pending_credential_service::FanOutTargetStatus {
                node_id: target.node_id.clone(),
                generation: target.generation,
                remote_state: target.remote_state.clone(),
                error_code: target.error_code,
                error_kind: target.error_kind.clone(),
                delivery_status: match target.remote_state {
                    Some(RemoteCryptoState::CiphertextReceived) => {
                        Some(node_pending_credential_service::FanOutDeliveryStatus::Sent)
                    }
                    Some(RemoteCryptoState::CiphertextQueued) => {
                        Some(node_pending_credential_service::FanOutDeliveryStatus::Queued)
                    }
                    _ => None,
                },
            },
        )
        .collect::<Vec<_>>();
    fan_out_pending_response(
        node_pending_credential_service::FanOutPendingCredentialResult { pending, targets },
    )
}

fn fan_out_pubkeys_response(
    pending: NodePendingCredential,
    integrity_verification_opt_out: bool,
) -> FanOutPendingCredentialPubkeysResponse {
    FanOutPendingCredentialPubkeysResponse {
        fanout_id: pending.id,
        fan_out_revision: pending.fan_out_revision,
        target_count: pending.fan_out_nodes.len(),
        integrity_verification_opt_out,
        targets: pending
            .fan_out_nodes
            .iter()
            .map(|target| FanOutPendingCredentialPubkeyTarget {
                node_id: target.node_id.clone(),
                generation: target.generation,
                version: target.crypto.version.clone(),
                node_pubkey: (!target.crypto.node_pubkey.is_empty())
                    .then(|| target.crypto.node_pubkey.clone()),
                remote_state: target
                    .remote_state
                    .as_ref()
                    .map(remote_state_name)
                    .map(str::to_string),
                error_code: if target.crypto.node_pubkey.is_empty() {
                    Some(crate::errors::PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE)
                } else {
                    target.error_code
                },
            })
            .collect(),
    }
}

fn decode_base64url_no_pad(value: &str, field: &str) -> AppResult<Vec<u8>> {
    if value.contains('=') {
        return Err(AppError::ValidationError(format!(
            "{field} must be base64url without padding"
        )));
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .map_err(|_| AppError::ValidationError(format!("{field} must be valid base64url")))
}

fn decode_base64url_no_pad_exact(
    value: &str,
    field: &str,
    expected_len: usize,
) -> AppResult<Vec<u8>> {
    let decoded = decode_base64url_no_pad(value, field)?;
    if decoded.len() != expected_len {
        return Err(AppError::ValidationError(format!(
            "{field} must decode to {expected_len} bytes"
        )));
    }
    Ok(decoded)
}

fn validate_pending_ciphertext_request(
    body: &PendingCredentialCiphertextRequest,
) -> AppResult<PendingCiphertextValidation> {
    if body.version != "v1" {
        return Err(AppError::PendingCredentialVersionUnsupported(
            body.version.clone(),
        ));
    }
    let _ = decode_base64url_no_pad_exact(&body.admin_pubkey, "admin_pubkey", 32)?;
    let _ = decode_base64url_no_pad_exact(&body.nonce, "nonce", 24)?;
    let ciphertext = decode_base64url_no_pad(&body.ciphertext, "ciphertext")?;
    if ciphertext.len() > node_pending_credential_service::MAX_CIPHERTEXT_SIZE {
        return Ok(PendingCiphertextValidation::TooLarge);
    }
    Ok(PendingCiphertextValidation::Valid(ciphertext))
}

fn validate_fan_out_ciphertext_request(
    body: PendingCredentialFanOutCiphertextRequest,
) -> AppResult<node_pending_credential_service::StoreFanOutCiphertextsInput> {
    if body.items.len() > node_pending_credential_service::MAX_FAN_OUT_TARGETS {
        return Err(AppError::ValidationError(format!(
            "items must contain {} or fewer fan-out ciphertexts",
            node_pending_credential_service::MAX_FAN_OUT_TARGETS
        )));
    }
    let mut total = 0usize;
    let mut items = Vec::with_capacity(body.items.len());
    for item in body.items {
        if item.version != "v1" {
            return Err(AppError::PendingCredentialVersionUnsupported(item.version));
        }
        let _ = decode_base64url_no_pad_exact(&item.admin_pubkey, "admin_pubkey", 32)?;
        let _ = decode_base64url_no_pad_exact(&item.nonce, "nonce", 24)?;
        let ciphertext = decode_base64url_no_pad(&item.ciphertext, "ciphertext")?;
        if ciphertext.len() > node_pending_credential_service::MAX_CIPHERTEXT_SIZE {
            return Err(AppError::PendingCredentialCiphertextTooLarge(
                ciphertext.len(),
            ));
        }
        total = total.saturating_add(ciphertext.len());
        if total > node_pending_credential_service::MAX_FAN_OUT_CIPHERTEXT_TOTAL_SIZE {
            return Err(AppError::PendingCredentialCiphertextTooLarge(total));
        }
        items.push(
            node_pending_credential_service::StoreFanOutCiphertextItemInput::new(
                item.node_id,
                item.generation,
                item.version,
                item.admin_pubkey,
                item.nonce,
                ciphertext,
            ),
        );
    }
    Ok(
        node_pending_credential_service::StoreFanOutCiphertextsInput {
            fan_out_revision: body.fan_out_revision,
            items,
            online_node_ids: HashSet::new(),
        },
    )
}

fn send_pending_ciphertext_to_node(
    state: &AppState,
    node_id: &str,
    pending: &NodePendingCredential,
) -> AppResult<()> {
    let crypto = pending.crypto.as_ref().ok_or_else(|| {
        AppError::Internal("pending credential ciphertext missing crypto bundle".to_string())
    })?;
    let admin_pubkey = crypto.admin_pubkey.as_deref().ok_or_else(|| {
        AppError::Internal("pending credential ciphertext missing admin_pubkey".to_string())
    })?;
    let nonce = crypto.nonce.as_deref().ok_or_else(|| {
        AppError::Internal("pending credential ciphertext missing nonce".to_string())
    })?;
    let ciphertext = crypto.ciphertext.as_ref().ok_or_else(|| {
        AppError::Internal("pending credential ciphertext missing ciphertext".to_string())
    })?;
    let ciphertext_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ciphertext.as_slice());
    let params = crate::services::node_ws_manager::PendingCredentialCiphertextParams {
        pending_id: &pending.id,
        version: &crypto.version,
        admin_pubkey,
        nonce,
        ciphertext: &ciphertext_b64,
    };
    state
        .node_ws_manager
        .send_pending_credential_ciphertext(node_id, &params)
}

fn send_fan_out_ciphertext_to_node(
    state: &AppState,
    pending: &NodePendingCredential,
    target: &FanOutNodeState,
) -> AppResult<()> {
    let admin_pubkey = target.crypto.admin_pubkey.as_deref().ok_or_else(|| {
        AppError::Internal("fan-out pending credential missing admin_pubkey".to_string())
    })?;
    let nonce = target.crypto.nonce.as_deref().ok_or_else(|| {
        AppError::Internal("fan-out pending credential missing nonce".to_string())
    })?;
    let ciphertext = target.crypto.ciphertext.as_ref().ok_or_else(|| {
        AppError::Internal("fan-out pending credential missing ciphertext".to_string())
    })?;
    if ciphertext.len() > node_pending_credential_service::MAX_CIPHERTEXT_SIZE {
        return Err(AppError::PendingCredentialCiphertextTooLarge(
            ciphertext.len(),
        ));
    }
    let ciphertext_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ciphertext.as_slice());
    let params = crate::services::node_ws_manager::PendingCredentialCiphertextParams {
        pending_id: &pending.id,
        version: &target.crypto.version,
        admin_pubkey,
        nonce,
        ciphertext: &ciphertext_b64,
    };
    state
        .node_ws_manager
        .send_pending_credential_ciphertext(&target.node_id, &params)
}

fn pending_ciphertext_state(pending: &NodePendingCredential, fallback: &'static str) -> String {
    pending
        .remote_state
        .as_ref()
        .map(remote_state_name)
        .unwrap_or(fallback)
        .to_string()
}

fn pending_ciphertext_sent_response(
    pending: &NodePendingCredential,
) -> (StatusCode, Json<PendingCredentialCiphertextResponse>) {
    (
        StatusCode::ACCEPTED,
        Json(PendingCredentialCiphertextResponse {
            delivery_status: "sent".to_string(),
            remote_state: pending_ciphertext_state(pending, "ciphertext_received"),
            error_code: None,
        }),
    )
}

fn pending_ciphertext_queued_response(
    pending: &NodePendingCredential,
) -> (StatusCode, Json<PendingCredentialCiphertextResponse>) {
    (
        StatusCode::ACCEPTED,
        Json(PendingCredentialCiphertextResponse {
            delivery_status: "queued".to_string(),
            remote_state: pending_ciphertext_state(pending, "ciphertext_queued"),
            error_code: Some(PENDING_CREDENTIAL_NODE_OFFLINE_CODE),
        }),
    )
}

fn log_rci_for_pending_user(
    state: &AppState,
    auth_user: &AuthUser,
    pending: &NodePendingCredential,
    kind: RciAuditEventKind,
) {
    let subject = RciAuditSubject::from_pending(pending);
    rci_audit_service::log_rci_for_user(state.db.clone(), auth_user, &subject, kind);
}

fn log_rci_for_pending_fan_out_target(
    state: &AppState,
    auth_user: &AuthUser,
    pending: &NodePendingCredential,
    target: &FanOutNodeState,
    kind: RciAuditEventKind,
) {
    let subject = RciAuditSubject::from_fan_out_target(pending, target);
    rci_audit_service::log_rci_for_user(state.db.clone(), auth_user, &subject, kind);
}

fn log_rci_for_summary_user(
    state: &AppState,
    auth_user: &AuthUser,
    summary: &node_pending_credential_service::PendingCredentialAuditSummary,
    kind: RciAuditEventKind,
) {
    let subject = RciAuditSubject::from_summary(summary);
    rci_audit_service::log_rci_for_user(state.db.clone(), auth_user, &subject, kind);
}

fn integrity_audit_value(integrity: &IntegrityVerificationAudit) -> serde_json::Value {
    serde_json::json!({
        "mode": integrity.mode,
        "fingerprint_sha384_prefix": integrity.fingerprint_sha384_prefix,
        "verified_at": integrity.verified_at,
        "manifest_url_configured": integrity.manifest_url_configured,
    })
}

fn log_integrity_ciphertext_submitted_for_pending(
    state: &AppState,
    auth_user: &AuthUser,
    pending: &NodePendingCredential,
    integrity: &IntegrityVerificationAudit,
) {
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "node_credential_ciphertext_submitted",
        Some(serde_json::json!({
            "node_id": &pending.node_id,
            "pending_credential_id": &pending.id,
            "service_slug": &pending.service_slug,
            "owner_user_id": &pending.owner_user_id,
            "integrity_verification": integrity_audit_value(integrity),
        })),
    );
}

fn log_integrity_ciphertext_submitted_for_fan_out(
    state: &AppState,
    auth_user: &AuthUser,
    pending: &NodePendingCredential,
    integrity: &IntegrityVerificationAudit,
) {
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "node_credential_ciphertext_submitted",
        Some(serde_json::json!({
            "fan_out": true,
            "fanout_id": &pending.id,
            "service_slug": &pending.service_slug,
            "owner_user_id": &pending.owner_user_id,
            "integrity_verification": integrity_audit_value(integrity),
        })),
    );
}

fn log_fan_out_ciphertext_audit(
    state: &AppState,
    auth_user: &AuthUser,
    pending: &NodePendingCredential,
    targets: &[node_pending_credential_service::FanOutTargetStatus],
) {
    for target_status in targets {
        if let Some(target) =
            node_pending_credential_service::fan_out_target(pending, &target_status.node_id)
        {
            log_rci_for_pending_fan_out_target(
                state,
                auth_user,
                pending,
                target,
                RciAuditEventKind::CiphertextReceived,
            );
            if matches!(
                target_status.delivery_status,
                Some(node_pending_credential_service::FanOutDeliveryStatus::Queued)
            ) {
                log_rci_for_pending_fan_out_target(
                    state,
                    auth_user,
                    pending,
                    target,
                    RciAuditEventKind::CiphertextQueued {
                        delivery: RciAuditDelivery::OfflineQueue,
                        node_offline: true,
                    },
                );
            }
        }
    }
}

// --- Handlers ---

/// POST /api/v1/nodes/register-token
pub async fn create_registration_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateRegistrationTokenRequest>,
) -> AppResult<Json<CreateRegistrationTokenResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let owner_user_id = body.owner_user_id.as_deref().unwrap_or(&user_id_str);

    if let Some(requested_owner) = body.owner_user_id.as_deref() {
        let access =
            org_service::resolve_owner_access(&state.db, &user_id_str, requested_owner).await?;
        if !matches!(
            access,
            org_service::OwnerAccess::Direct | org_service::OwnerAccess::AsOrgAdmin { .. }
        ) {
            return Err(AppError::Forbidden(
                "Only org admins can create registration tokens for that owner".to_string(),
            ));
        }
    }

    let (token_id, raw_token, expires_at): (String, String, chrono::DateTime<chrono::Utc>) =
        node_service::create_registration_token(
            &state.db,
            owner_user_id,
            &body.name,
            state.config.node_max_per_user,
            state.config.node_registration_token_ttl_secs,
        )
        .await?;
    let owner_differs = owner_user_id != user_id_str;
    let owner_user_id_for_audit = owner_user_id.to_string();
    let event_data = if owner_differs {
        serde_json::json!({
            "token_id": &token_id,
            "name": &body.name,
            "owner_user_id": &owner_user_id_for_audit,
        })
    } else {
        serde_json::json!({
            "token_id": &token_id,
            "name": &body.name,
        })
    };

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_registration_token_created",
        Some(event_data),
    );

    Ok(Json(CreateRegistrationTokenResponse {
        token_id,
        token: raw_token,
        name: body.name,
        expires_at: expires_at.to_rfc3339(),
    }))
}

/// GET /api/v1/nodes
pub async fn list_nodes(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<NodeListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let nodes = node_service::list_user_nodes(&state.db, &user_id_str).await?;

    // Batch-fetch binding counts in a single aggregation instead of N+1 queries
    let binding_counts: HashMap<String, u64> = if nodes.is_empty() {
        HashMap::new()
    } else {
        let node_id_array: bson::Array = nodes
            .iter()
            .map(|n| bson::Bson::String(n.node.id.clone()))
            .collect();
        let pipeline = vec![
            doc! { "$match": { "node_id": { "$in": node_id_array }, "is_active": true } },
            doc! { "$group": { "_id": "$node_id", "count": { "$sum": 1 } } },
        ];
        let mut cursor = state
            .db
            .collection::<mongodb::bson::Document>("node_service_bindings")
            .aggregate(pipeline)
            .await?;
        let mut counts = HashMap::new();
        while let Some(result) = cursor.try_next().await? {
            if let Ok(node_id) = result.get_str("_id") {
                // $sum may return Int32 or Int64 depending on value size
                let count = result
                    .get("count")
                    .and_then(|v| match v {
                        bson::Bson::Int32(n) => Some(*n as u64),
                        bson::Bson::Int64(n) => Some(*n as u64),
                        _ => None,
                    })
                    .unwrap_or(0);
                counts.insert(node_id.to_string(), count);
            }
        }
        counts
    };

    let node_infos: Vec<NodeInfo> = nodes
        .iter()
        .map(|node_with_owner| {
            let node = &node_with_owner.node;
            NodeInfo {
                id: node.id.clone(),
                name: node.name.clone(),
                owner: node_with_owner.owner.clone(),
                status: node.status.as_str().to_string(),
                is_connected: state.node_ws_manager.is_connected(&node.id),
                last_heartbeat_at: node.last_heartbeat_at.map(|dt| dt.to_rfc3339()),
                connected_at: node.connected_at.map(|dt| dt.to_rfc3339()),
                metadata: node.metadata.clone(),
                metrics: Some(build_metrics_info(&node.metrics)),
                binding_count: binding_counts.get(&node.id).copied().unwrap_or(0),
                created_at: node.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(NodeListResponse { nodes: node_infos }))
}

/// GET /api/v1/nodes/{node_id}
pub async fn get_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<NodeInfo>> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;
    let owner = node_service::owner_info_for_node(&state.db, &node).await?;

    let binding_count = state
        .db
        .collection::<mongodb::bson::Document>("node_service_bindings")
        .count_documents(doc! { "node_id": &node.id, "is_active": true })
        .await?;

    Ok(Json(NodeInfo {
        id: node.id.clone(),
        name: node.name,
        owner,
        status: node.status.as_str().to_string(),
        is_connected: state.node_ws_manager.is_connected(&node.id),
        last_heartbeat_at: node.last_heartbeat_at.map(|dt| dt.to_rfc3339()),
        connected_at: node.connected_at.map(|dt| dt.to_rfc3339()),
        metadata: node.metadata,
        metrics: Some(build_metrics_info(&node.metrics)),
        binding_count,
        created_at: node.created_at.to_rfc3339(),
    }))
}

/// DELETE /api/v1/nodes/{node_id}
pub async fn delete_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(node_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    node_service::delete_node(&state.db, &user_id_str, &node_id).await?;

    // Disconnect WebSocket if connected
    if state.node_ws_manager.is_connected(&node_id) {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4006, "node deleted")
            .await;
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_deleted",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({ "node_id": &node_id }),
        )),
    );

    emit_event(
        state.telemetry.as_deref(),
        &user_id_str,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::NodeDeleted {
            // Raw UUID would be scrubbed to `[UUID_REDACTED]`; hash keeps
            // per-node granularity without leaking the UUID.
            node_id: hash_short_id(&node_id),
        },
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/nodes/{node_id}/rotate-token
pub async fn rotate_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<RotateTokenResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    let (raw_token, raw_signing_secret) =
        node_service::rotate_auth_token(&state.db, &state.encryption_keys, &user_id_str, &node_id)
            .await?;

    // Disconnect the node since its old token is now invalid
    if state.node_ws_manager.is_connected(&node_id) {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4002, "node credentials rotated")
            .await;
        node_service::set_node_status(
            &state.db,
            &node_id,
            crate::models::node::NodeStatus::Offline,
        )
        .await?;
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_token_rotated",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({ "node_id": &node_id }),
        )),
    );

    Ok(Json(RotateTokenResponse {
        auth_token: raw_token,
        signing_secret: raw_signing_secret,
        message:
            "Auth token and signing secret rotated. The node must reconnect with the new token."
                .to_string(),
    }))
}

/// GET /api/v1/nodes/{node_id}/admins
pub async fn list_admins(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<NodeAdminsResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let admins = node_service::list_node_admins(&state.db, &user_id_str, &node_id).await?;

    Ok(Json(NodeAdminsResponse { admins }))
}

/// POST /api/v1/nodes/{node_id}/transfer
pub async fn transfer_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Json(body): Json<TransferNodeRequest>,
) -> AppResult<Json<TransferNodeResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;
    let previous_owner = node_service::owner_info_for_node(&state.db, &node).await?;

    let result = node_service::transfer_node_owner(
        &state.db,
        &user_id_str,
        &node_id,
        &body.new_owner_user_id,
        state.config.node_max_per_user,
    )
    .await?;

    let mut transferred_node = node.clone();
    transferred_node.user_id = result.new_owner_user_id.clone();
    let new_owner = node_service::owner_info_for_node(&state.db, &transferred_node).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_transferred",
        Some(transfer_audit_event_data(&user_id_str, &result)),
    );

    Ok(Json(TransferNodeResponse {
        node_id: result.node_id,
        previous_owner,
        new_owner,
        deactivated_bindings_count: result.deactivated_bindings_count,
        cleared_user_service_count: result.cleared_user_service_count,
    }))
}

/// POST /api/v1/nodes/{node_id}/credentials/push
pub async fn push_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Json(body): Json<PushPendingCredentialRequest>,
) -> AppResult<Json<PendingCredentialInfo>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::create_pending_credential(
        &state.db,
        &user_id_str,
        &node_id,
        node_pending_credential_service::CreatePendingCredentialInput {
            service_slug: body.service_slug,
            injection_method: body.injection_method,
            field_name: body.field_name,
            target_url: body.target_url,
            label: body.label,
            ttl_secs: state.config.node_pending_credential_ttl_secs,
            remote_crypto: body.remote_crypto.unwrap_or(false),
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_credential_push_created",
        Some(serde_json::json!({
            "node_id": &pending.node_id,
            "service_slug": &pending.service_slug,
            "injection_method": pending.injection_method.as_str(),
            "owner_user_id": &pending.owner_user_id,
        })),
    );

    if state.node_ws_manager.is_connected(&node_id)
        && let Err(err) = state
            .node_ws_manager
            .send_pending_credentials_available(&node_id)
    {
        tracing::warn!(
            node_id = %node_id,
            error = %err,
            "Failed to nudge node about pending credential"
        );
    }

    Ok(Json(pending_credential_info(pending)))
}

/// POST /api/v1/nodes/credentials/push/fan-out
pub async fn push_pending_credential_fan_out(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<PushPendingCredentialFanOutRequest>,
) -> AppResult<Json<FanOutPendingCredentialResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let result = node_pending_credential_service::create_fan_out_pending_credential(
        &state.db,
        &user_id_str,
        node_pending_credential_service::CreateFanOutPendingCredentialInput {
            owner_user_id: body.owner_user_id,
            service_id: body.service_id,
            service_slug: body.service_slug,
            injection_method: body.injection_method,
            field_name: body.field_name,
            target_url: body.target_url,
            label: body.label,
            ttl_secs: state.config.node_pending_credential_ttl_secs,
            remote_crypto: body.remote_crypto.unwrap_or(true),
        },
    )
    .await?;

    rci_audit_service::log_rci_fan_out_for_user(
        state.db.clone(),
        &auth_user,
        &rci_audit_service::RciFanOutAuditSubject::from_pending(&result.pending),
        rci_audit_service::RciFanOutAuditEventKind::Created,
    );

    for target in &result.targets {
        if state.node_ws_manager.is_connected(&target.node_id)
            && let Err(err) = state
                .node_ws_manager
                .send_pending_credentials_available(&target.node_id)
        {
            tracing::warn!(
                node_id = %target.node_id,
                error = %err,
                "Failed to nudge node about fan-out pending credential"
            );
        }
    }

    Ok(Json(fan_out_pending_response(result)))
}

/// GET /api/v1/nodes/credentials/pending/{fanout_id}/fan-out
pub async fn get_fan_out_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(fanout_id): Path<String>,
) -> AppResult<Json<FanOutPendingCredentialResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::get_fan_out_pending_credential_for_admin(
        &state.db,
        &user_id_str,
        &fanout_id,
    )
    .await?;

    Ok(Json(fan_out_status_response(pending)))
}

/// GET /api/v1/nodes/credentials/pending/{fanout_id}/fan-out/pubkeys
pub async fn get_fan_out_pending_credential_pubkeys(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(fanout_id): Path<String>,
) -> AppResult<Json<FanOutPendingCredentialPubkeysResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::get_fan_out_pending_credential_for_admin(
        &state.db,
        &user_id_str,
        &fanout_id,
    )
    .await?;
    let opt_out = node_pending_credential_service::owner_integrity_verification_opt_out(
        &state.db,
        &pending.owner_user_id,
    )
    .await?;

    Ok(Json(fan_out_pubkeys_response(pending, opt_out)))
}

/// POST /api/v1/nodes/credentials/pending/{fanout_id}/fan-out/ciphertexts
pub async fn post_fan_out_pending_credential_ciphertexts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(fanout_id): Path<String>,
    Json(body): Json<PendingCredentialFanOutCiphertextRequest>,
) -> AppResult<(StatusCode, Json<FanOutPendingCredentialCiphertextResponse>)> {
    let integrity_request = body.integrity_verification.clone();
    let mut input = validate_fan_out_ciphertext_request(body)?;
    let user_id_str = auth_user.user_id.to_string();
    let pending_for_policy =
        node_pending_credential_service::get_fan_out_pending_credential_for_admin(
            &state.db,
            &user_id_str,
            &fanout_id,
        )
        .await?;
    let now = chrono::Utc::now();
    let integrity_audit =
        node_pending_credential_service::validate_integrity_verification_for_owner(
            &state.db,
            &pending_for_policy.owner_user_id,
            state.config.release_integrity_manifest_url.as_deref(),
            state.config.jwt_relay_reply_ttl_secs,
            integrity_request.as_ref(),
            now,
        )
        .await?;
    input.online_node_ids = input
        .items
        .iter()
        .filter(|item| {
            state.node_ws_manager.is_connected(&item.node_id)
                && state
                    .node_ws_manager
                    .supports_remote_credential_crypto(&item.node_id)
        })
        .map(|item| item.node_id.clone())
        .collect::<HashSet<_>>();

    let outcome = node_pending_credential_service::store_fan_out_ciphertexts_revision_guard(
        &state.db,
        &user_id_str,
        &fanout_id,
        input,
        now,
    )
    .await?;

    log_integrity_ciphertext_submitted_for_fan_out(
        &state,
        &auth_user,
        &outcome.pending,
        &integrity_audit,
    );
    log_fan_out_ciphertext_audit(&state, &auth_user, &outcome.pending, &outcome.targets);

    let mut latest_pending = outcome.pending.clone();
    for target in outcome.pending.fan_out_nodes.iter().filter(|target| {
        matches!(
            target.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        )
    }) {
        match send_fan_out_ciphertext_to_node(&state, &outcome.pending, target) {
            Ok(()) => {
                log_rci_for_pending_fan_out_target(
                    &state,
                    &auth_user,
                    &outcome.pending,
                    target,
                    RciAuditEventKind::CiphertextForwarded {
                        delivery: RciAuditDelivery::OnlineForward,
                    },
                );
            }
            Err(err) => {
                tracing::warn!(
                    node_id = %target.node_id,
                    fanout_id = %outcome.pending.id,
                    error = %err,
                    "Failed to send fan-out pending credential ciphertext; queueing for retry"
                );
                latest_pending =
                    node_pending_credential_service::mark_fan_out_ciphertext_queued_after_send_failure(
                        &state.db,
                        &target.node_id,
                        &outcome.pending.id,
                        chrono::Utc::now(),
                    )
                    .await?;
            }
        }
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(FanOutPendingCredentialCiphertextResponse {
            fanout_id,
            fan_out_revision: latest_pending.fan_out_revision,
            remote_state: pending_remote_state(&latest_pending)
                .unwrap_or_else(|| "ciphertext_received".to_string()),
            targets: fan_out_status_response(latest_pending).targets,
        }),
    ))
}

/// POST /api/v1/nodes/credentials/pending/{fanout_id}/fan-out/retry-failed
pub async fn retry_failed_fan_out_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(fanout_id): Path<String>,
    Json(body): Json<RetryFanOutPendingCredentialRequest>,
) -> AppResult<Json<FanOutPendingCredentialResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let result = node_pending_credential_service::retry_failed_fan_out_nodes(
        &state.db,
        &user_id_str,
        &fanout_id,
        body.fan_out_revision,
        chrono::Utc::now(),
    )
    .await?;

    rci_audit_service::log_rci_fan_out_for_user(
        state.db.clone(),
        &auth_user,
        &rci_audit_service::RciFanOutAuditSubject::from_pending(&result.pending),
        rci_audit_service::RciFanOutAuditEventKind::RetryStarted,
    );

    for target in &result.targets {
        if state.node_ws_manager.is_connected(&target.node_id)
            && let Err(err) = state
                .node_ws_manager
                .send_pending_credentials_available(&target.node_id)
        {
            tracing::warn!(
                node_id = %target.node_id,
                error = %err,
                "Failed to nudge node about fan-out retry"
            );
        }
    }

    Ok(Json(fan_out_pending_response(result)))
}

/// GET /api/v1/nodes/{node_id}/credentials/pending
pub async fn list_pending_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Query(query): Query<PendingCredentialListQuery>,
) -> AppResult<Json<PendingCredentialListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::list_pending_credentials_for_admin(
        &state.db,
        &user_id_str,
        &node_id,
        query.include_history.unwrap_or(false),
    )
    .await?;

    Ok(Json(PendingCredentialListResponse {
        pending_credentials: pending.into_iter().map(pending_credential_info).collect(),
    }))
}

/// GET /api/v1/nodes/{node_id}/credentials/pending/{pending_id}
pub async fn get_pending_credential_pubkey(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, pending_id)): Path<(String, String)>,
) -> AppResult<Json<PendingCredentialPubkeyResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::get_pending_credential_for_admin(
        &state.db,
        &user_id_str,
        &node_id,
        &pending_id,
    )
    .await?;
    let opt_out = node_pending_credential_service::owner_integrity_verification_opt_out(
        &state.db,
        &pending.owner_user_id,
    )
    .await?;

    Ok(Json(pending_pubkey_response(pending, opt_out)?))
}

/// POST /api/v1/nodes/{node_id}/credentials/pending/{pending_id}/remote-crypto
pub async fn init_pending_credential_remote_crypto(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, pending_id)): Path<(String, String)>,
) -> AppResult<Json<PendingCredentialInfo>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::init_pending_remote_crypto_for_admin(
        &state.db,
        &user_id_str,
        &node_id,
        &pending_id,
    )
    .await?;

    if state.node_ws_manager.is_connected(&node_id)
        && let Err(err) = state
            .node_ws_manager
            .send_pending_credentials_available(&node_id)
    {
        tracing::warn!(
            node_id = %node_id,
            pending_id = %pending_id,
            error = %err,
            "Failed to nudge node about pending remote crypto initialization"
        );
    }

    Ok(Json(pending_credential_info(pending)))
}

/// POST /api/v1/nodes/{node_id}/credentials/pending/{pending_id}/ciphertext
pub async fn post_pending_credential_ciphertext(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, pending_id)): Path<(String, String)>,
    Json(body): Json<PendingCredentialCiphertextRequest>,
) -> AppResult<(StatusCode, Json<PendingCredentialCiphertextResponse>)> {
    let validation = validate_pending_ciphertext_request(&body)?;
    let user_id_str = auth_user.user_id.to_string();
    let subject_summary =
        node_pending_credential_service::get_pending_credential_audit_summary_for_admin(
            &state.db,
            &user_id_str,
            &node_id,
            &pending_id,
        )
        .await?;
    let ciphertext = match validation {
        PendingCiphertextValidation::Valid(ciphertext) => ciphertext,
        PendingCiphertextValidation::TooLarge => {
            log_rci_for_summary_user(
                &state,
                &auth_user,
                &subject_summary,
                RciAuditEventKind::CiphertextTooLarge,
            );
            return Err(AppError::PendingCredentialCiphertextTooLarge(
                node_pending_credential_service::MAX_CIPHERTEXT_SIZE + 1,
            ));
        }
    };
    let now = chrono::Utc::now();
    let integrity_audit =
        node_pending_credential_service::validate_integrity_verification_for_owner(
            &state.db,
            &subject_summary.owner_user_id,
            state.config.release_integrity_manifest_url.as_deref(),
            state.config.jwt_relay_reply_ttl_secs,
            body.integrity_verification.as_ref(),
            now,
        )
        .await?;
    let node_can_receive_now = state.node_ws_manager.is_connected(&node_id)
        && state
            .node_ws_manager
            .supports_remote_credential_crypto(&node_id);

    let outcome = node_pending_credential_service::store_pending_ciphertext_first_writer_wins(
        &state.db,
        &user_id_str,
        &node_id,
        &pending_id,
        node_pending_credential_service::StorePendingCiphertextInput::new(
            body.admin_pubkey,
            body.nonce,
            ciphertext,
        ),
        node_can_receive_now,
        now,
    )
    .await
    .map_err(|err| {
        if matches!(err, AppError::PendingCredentialPubkeyAwaiting(_)) {
            log_rci_for_summary_user(
                &state,
                &auth_user,
                &subject_summary,
                RciAuditEventKind::PubkeyAwaiting,
            );
        }
        err
    })?;

    match outcome {
        node_pending_credential_service::StorePendingCiphertextOutcome::QueueFull(summary) => {
            log_rci_for_summary_user(&state, &auth_user, &summary, RciAuditEventKind::QueueFull);
            Err(AppError::PendingCredentialQueueFull(node_id))
        }
        node_pending_credential_service::StorePendingCiphertextOutcome::QueuedOffline(pending) => {
            log_integrity_ciphertext_submitted_for_pending(
                &state,
                &auth_user,
                &pending,
                &integrity_audit,
            );
            log_rci_for_pending_user(
                &state,
                &auth_user,
                &pending,
                RciAuditEventKind::CiphertextReceived,
            );
            log_rci_for_pending_user(
                &state,
                &auth_user,
                &pending,
                RciAuditEventKind::CiphertextQueued {
                    delivery: RciAuditDelivery::OfflineQueue,
                    node_offline: true,
                },
            );
            Ok(pending_ciphertext_queued_response(&pending))
        }
        node_pending_credential_service::StorePendingCiphertextOutcome::StoredForOnlineNode(
            pending,
        ) => {
            log_integrity_ciphertext_submitted_for_pending(
                &state,
                &auth_user,
                &pending,
                &integrity_audit,
            );
            log_rci_for_pending_user(
                &state,
                &auth_user,
                &pending,
                RciAuditEventKind::CiphertextReceived,
            );
            match send_pending_ciphertext_to_node(&state, &node_id, &pending) {
                Ok(()) => {
                    log_rci_for_pending_user(
                        &state,
                        &auth_user,
                        &pending,
                        RciAuditEventKind::CiphertextForwarded {
                            delivery: RciAuditDelivery::OnlineForward,
                        },
                    );
                    Ok(pending_ciphertext_sent_response(&pending))
                }
                Err(err) => {
                    tracing::warn!(
                        node_id = %node_id,
                        pending_id = %pending.id,
                        error = %err,
                        "Failed to send pending credential ciphertext; queueing for retry"
                    );
                    let queued =
                    node_pending_credential_service::mark_pending_ciphertext_queued_after_send_failure(
                        &state.db,
                        &node_id,
                        &pending.id,
                        chrono::Utc::now(),
                    )
                    .await?;
                    log_rci_for_pending_user(
                        &state,
                        &auth_user,
                        &queued,
                        RciAuditEventKind::CiphertextQueued {
                            delivery: RciAuditDelivery::OfflineQueue,
                            node_offline: true,
                        },
                    );
                    Ok(pending_ciphertext_queued_response(&queued))
                }
            }
        }
    }
}

/// DELETE /api/v1/nodes/{node_id}/credentials/pending/{pending_id}
pub async fn cancel_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, pending_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::cancel_pending_credential(
        &state.db,
        &user_id_str,
        &node_id,
        &pending_id,
    )
    .await?;

    if RciAuditSubject::pending_is_rci(&pending) {
        log_rci_for_pending_user(&state, &auth_user, &pending, RciAuditEventKind::Canceled);
    } else {
        audit_service::log_for_user(
            state.db.clone(),
            &auth_user,
            "node_credential_push_canceled",
            Some(serde_json::json!({
                "node_id": &pending.node_id,
                "pending_credential_id": &pending.id,
                "service_slug": &pending.service_slug,
                "owner_user_id": &pending.owner_user_id,
            })),
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/nodes/{node_id}/bindings
pub async fn list_bindings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<BindingListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let bindings = node_service::list_bindings(&state.db, &user_id_str, &node_id).await?;

    // M3: Batch-fetch all referenced services in a single query instead of N+1
    let service_id_array: bson::Array = bindings
        .iter()
        .map(|b| bson::Bson::String(b.service_id.clone()))
        .collect();

    let services: Vec<DownstreamService> = if service_id_array.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": service_id_array } })
            .await?
            .try_collect()
            .await?
    };

    let service_map: HashMap<&str, &DownstreamService> =
        services.iter().map(|s| (s.id.as_str(), s)).collect();

    let binding_infos: Vec<BindingInfo> = bindings
        .iter()
        .map(|binding| {
            let (service_name, service_slug) = match service_map.get(binding.service_id.as_str()) {
                Some(s) => (s.name.clone(), s.slug.clone()),
                None => ("Unknown".to_string(), "unknown".to_string()),
            };

            BindingInfo {
                id: binding.id.clone(),
                service_id: binding.service_id.clone(),
                service_name,
                service_slug,
                is_active: binding.is_active,
                priority: binding.priority,
                created_at: binding.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(BindingListResponse {
        bindings: binding_infos,
    }))
}

/// POST /api/v1/nodes/{node_id}/bindings
pub async fn create_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Json(body): Json<CreateBindingRequest>,
) -> AppResult<Json<CreateBindingResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Verify the service exists
    let service = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": &body.service_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    let binding =
        node_service::create_binding(&state.db, &user_id_str, &node_id, &body.service_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_binding_created",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &binding.user_id,
            serde_json::json!({
                "binding_id": &binding.id,
                "node_id": &node_id,
                "service_id": &body.service_id,
            }),
        )),
    );

    Ok(Json(CreateBindingResponse {
        id: binding.id,
        service_id: body.service_id,
        service_name: service.name,
        message: "Service binding created".to_string(),
    }))
}

/// PATCH /api/v1/nodes/{node_id}/bindings/{binding_id}
pub async fn update_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, binding_id)): Path<(String, String)>,
    Json(body): Json<UpdateBindingRequest>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    if let Some(priority) = body.priority {
        node_service::update_binding_priority(
            &state.db,
            &user_id_str,
            &node_id,
            &binding_id,
            priority,
        )
        .await?;
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_binding_updated",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({
                "binding_id": &binding_id,
                "node_id": &node_id,
                "priority": body.priority,
            }),
        )),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/nodes/{node_id}/bindings/{binding_id}
pub async fn delete_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, binding_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    node_service::delete_binding(&state.db, &user_id_str, &node_id, &binding_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_binding_deleted",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({
                "binding_id": &binding_id,
                "node_id": &node_id,
            }),
        )),
    );

    Ok(StatusCode::NO_CONTENT)
}

// --- My Bindings ---

#[derive(Debug, Serialize)]
pub struct MyBoundServicesResponse {
    pub service_ids: Vec<String>,
}

/// GET /api/v1/nodes/my-bindings
///
/// List all service IDs for which the authenticated user currently has a viable node route.
pub async fn list_my_bound_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<MyBoundServicesResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let service_ids = node_routing_service::list_routable_service_ids(
        &state.db,
        &user_id_str,
        state.node_ws_manager.as_ref(),
    )
    .await?;

    Ok(Json(MyBoundServicesResponse { service_ids }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{
        PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE, PENDING_CREDENTIAL_QUEUE_FULL_CODE,
    };
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_pending_credential::{
        COLLECTION_NAME as NODE_PENDING_CREDENTIALS, NodePendingCredential,
    };
    use crate::models::node_registration_token::{
        COLLECTION_NAME as NODE_REG_TOKENS, NodeRegistrationToken,
    };
    use crate::models::node_service_binding::{
        COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
    };
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{
        COLLECTION_NAME as USERS, ReleaseIntegrityProfileConfig, User, UserProfileConfig, UserType,
    };
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::{
        audit_service, node_pending_credential_service,
        node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage},
    };
    use crate::test_utils::{
        assert_rci_audit_row, connect_test_database, test_app_state, test_auth_user,
        test_membership, test_user, test_user_service,
    };
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use base64::Engine;
    use chrono::Utc;
    use serde_json::Value;
    use tokio::sync::mpsc;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn test_node(owner_id: &str, name: &str) -> Node {
        let now = Utc::now();
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_binding(owner_id: &str, node_id: &str, service_id: &str) -> NodeServiceBinding {
        let now = Utc::now();
        NodeServiceBinding {
            id: Uuid::new_v4().to_string(),
            node_id: node_id.to_string(),
            user_id: owner_id.to_string(),
            service_id: service_id.to_string(),
            is_active: true,
            priority: 0,
            created_at: now,
            updated_at: now,
        }
    }

    async fn insert_users(db: &mongodb::Database, users: Vec<User>) {
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert users");
    }

    async fn test_db(prefix: &str) -> mongodb::Database {
        connect_test_database(prefix)
            .await
            .expect("local MongoDB required for node pending handler tests")
    }

    async fn insert_node(db: &mongodb::Database, node: &Node) {
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");
    }

    async fn load_pending(db: &mongodb::Database, pending_id: &str) -> NodePendingCredential {
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": pending_id })
            .await
            .expect("query pending credential")
            .expect("pending credential exists")
    }

    async fn load_audit_entry(
        db: &mongodb::Database,
        receiver: tokio::sync::oneshot::Receiver<String>,
    ) -> AuditLog {
        let audit_id = receiver.await.expect("audit write notification");
        db.collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! { "_id": audit_id })
            .await
            .expect("query audit log")
            .expect("audit log exists")
    }

    fn assert_pubkey_only_pending(pending: &NodePendingCredential, expected_node_pubkey: &str) {
        assert!(pending.is_active);
        assert_eq!(pending.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert!(pending.ciphertext_queued_at.is_none());
        assert!(pending.ciphertext_expires_at.is_none());
        let crypto = pending.crypto.as_ref().expect("crypto metadata");
        assert_eq!(crypto.version, "v1");
        assert_eq!(crypto.node_pubkey, expected_node_pubkey);
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
    }

    fn api_app(mut state: AppState) -> axum::Router {
        if state.config.release_integrity_manifest_url.is_none() {
            state.config.release_integrity_manifest_url =
                Some("https://release.example.test/releases.json".to_string());
        }
        api_app_preserving_config(state)
    }

    fn api_app_preserving_config(state: AppState) -> axum::Router {
        let (_, private) = crate::routes::build_router(state.config.proxy_max_body_size);
        private.with_state(state)
    }

    fn access_token(state: &AppState, user_id: &str) -> String {
        let user_id = Uuid::parse_str(user_id).expect("valid user id");
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            "",
            None,
            None,
            None,
            None,
        )
        .expect("sign test access token")
    }

    async fn route_json(
        app: axum::Router,
        method: Method,
        uri: String,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = app
            .oneshot(builder.body(body).expect("build request"))
            .await;
        let response = response.expect("route response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("json response")
        };
        (status, value)
    }

    async fn route_raw(
        app: axum::Router,
        method: Method,
        uri: String,
        token: &str,
        body: String,
    ) -> (StatusCode, Vec<u8>) {
        let response = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("build request"),
            )
            .await
            .expect("route response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        (status, bytes.to_vec())
    }

    fn b64url(byte: u8, len: usize) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![byte; len])
    }

    fn ciphertext_request(ciphertext: Vec<u8>) -> Value {
        serde_json::json!({
            "version": "v1",
            "admin_pubkey": b64url(10, 32),
            "nonce": b64url(11, 24),
            "ciphertext": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ciphertext),
            "integrity_verification": valid_integrity_verification(),
        })
    }

    fn valid_integrity_verification() -> Value {
        serde_json::json!({
            "mode": "admin_verified",
            "fingerprint_sha384_hex": "a".repeat(96),
            "verified_at": Utc::now().to_rfc3339(),
            "manifest_url_configured": true,
        })
    }

    fn test_org_with_integrity_opt_out(org_id: &str) -> User {
        let mut org = test_user(org_id, UserType::Org);
        org.profile_config = UserProfileConfig {
            release_integrity: ReleaseIntegrityProfileConfig {
                remote_credential_integrity_verification_opt_out: true,
            },
            ..UserProfileConfig::default()
        };
        org
    }

    fn fan_out_ciphertext_request(ciphertext: Vec<u8>) -> Value {
        serde_json::json!({
            "fan_out_revision": 1,
            "items": [
                {
                    "node_id": "node-a",
                    "generation": 0,
                    "version": "v1",
                    "admin_pubkey": b64url(10, 32),
                    "nonce": b64url(11, 24),
                    "ciphertext": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ciphertext),
                }
            ],
            "integrity_verification": valid_integrity_verification(),
        })
    }

    fn fan_out_ciphertext_request_for_targets(
        revision: i64,
        first_node_id: &str,
        second_node_id: &str,
    ) -> Value {
        serde_json::json!({
            "fan_out_revision": revision,
            "items": [
                {
                    "node_id": first_node_id,
                    "generation": 0,
                    "version": "v1",
                    "admin_pubkey": b64url(10, 32),
                    "nonce": b64url(11, 24),
                    "ciphertext": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3]),
                },
                {
                    "node_id": second_node_id,
                    "generation": 0,
                    "version": "v1",
                    "admin_pubkey": b64url(10, 32),
                    "nonce": b64url(11, 24),
                    "ciphertext": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3, 4]),
                }
            ],
            "integrity_verification": valid_integrity_verification(),
        })
    }

    fn assert_no_fan_out_secret_fields(value: &Value) {
        fn assert_no_forbidden_keys(value: &Value) {
            match value {
                Value::Object(object) => {
                    for (key, value) in object {
                        for forbidden_key in [
                            "admin_pubkey",
                            "nonce",
                            "ciphertext",
                            "plaintext",
                            "secret",
                            "node_pubkey_secret",
                        ] {
                            assert_ne!(key, forbidden_key, "{forbidden_key}");
                        }
                        assert_no_forbidden_keys(value);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        assert_no_forbidden_keys(value);
                    }
                }
                _ => {}
            }
        }

        assert_no_forbidden_keys(value);
        let json = value.to_string();
        for forbidden in [
            "plaintext",
            "secret-value-fixture",
            &b64url(10, 32),
            &b64url(11, 24),
            &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3]),
            &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3, 4]),
        ] {
            assert!(!json.contains(forbidden), "{forbidden}");
        }
    }

    fn response_target<'a>(body: &'a Value, node_id: &str) -> &'a Value {
        body["targets"]
            .as_array()
            .expect("targets array")
            .iter()
            .find(|target| target["node_id"] == node_id)
            .expect("target present")
    }

    fn fan_out_target_pending_for_audit(
        pending: &NodePendingCredential,
        node_id: &str,
    ) -> NodePendingCredential {
        let target = node_pending_credential_service::fan_out_target(pending, node_id)
            .expect("fan-out target exists");
        let mut target_pending = pending.clone();
        target_pending.node_id = target.node_id.clone();
        target_pending.remote_state = target.remote_state.clone();
        target_pending.ciphertext_queued_at = target.ciphertext_queued_at;
        target_pending.ciphertext_expires_at = target.ciphertext_expires_at;
        target_pending
    }

    fn assert_fan_out_target_audit_row(
        entry: &AuditLog,
        expected_event_type: &str,
        pending: &NodePendingCredential,
        node_id: &str,
        expected_remote_state: Option<&str>,
        extra_keys: &[&str],
    ) {
        let target_pending = fan_out_target_pending_for_audit(pending, node_id);
        let mut expected_extra = vec!["fan_out", "fanout_id", "generation"];
        expected_extra.extend(extra_keys.iter().copied());
        assert_rci_audit_row(
            entry,
            expected_event_type,
            &target_pending,
            expected_remote_state,
            &expected_extra,
        );
        let event_data = entry.event_data.as_ref().expect("audit event data");
        let target = node_pending_credential_service::fan_out_target(pending, node_id)
            .expect("fan-out target exists");
        assert_eq!(event_data["fan_out"], true);
        assert_eq!(event_data["fanout_id"], pending.id);
        assert_eq!(event_data["generation"], target.generation);
    }

    fn sorted_keys(keys: &[&str]) -> Vec<String> {
        let mut keys: Vec<String> = keys.iter().map(|key| (*key).to_string()).collect();
        keys.sort();
        keys
    }

    fn assert_fan_out_aggregate_audit_row(
        entry: &AuditLog,
        expected_event_type: &str,
        expected_user_id: &str,
        pending: &NodePendingCredential,
        expected_remote_state: Option<&str>,
    ) {
        assert_eq!(entry.event_type, expected_event_type);
        assert_eq!(entry.user_id.as_deref(), Some(expected_user_id));
        let event_data = entry.event_data.as_ref().expect("audit event data");
        let object = event_data.as_object().expect("audit event data object");
        let mut expected_keys = vec![
            "event_at",
            "failed_count",
            "fan_out",
            "fan_out_revision",
            "fanout_id",
            "flow",
            "owner_user_id",
            "pending_created_at",
            "pending_expires_at",
            "queued_count",
            "service_slug",
            "succeeded_count",
            "target_count",
        ];
        if expected_remote_state.is_some() {
            expected_keys.push("remote_state");
        }
        let mut actual_keys: Vec<String> = object.keys().cloned().collect();
        actual_keys.sort();
        assert_eq!(actual_keys, sorted_keys(&expected_keys));

        assert_eq!(event_data["flow"], "remote_credential_injection");
        assert_eq!(event_data["fan_out"], true);
        assert_eq!(event_data["fanout_id"], pending.id);
        assert_eq!(event_data["service_slug"], pending.service_slug);
        assert_eq!(event_data["owner_user_id"], pending.owner_user_id);
        assert_eq!(event_data["target_count"], pending.fan_out_nodes.len());
        assert_eq!(event_data["fan_out_revision"], pending.fan_out_revision);
        assert_eq!(
            event_data["succeeded_count"],
            pending
                .fan_out_nodes
                .iter()
                .filter(|target| matches!(target.remote_state, Some(RemoteCryptoState::Consumed)))
                .count()
        );
        assert_eq!(
            event_data["failed_count"],
            pending
                .fan_out_nodes
                .iter()
                .filter(|target| {
                    matches!(
                        target.remote_state,
                        Some(
                            RemoteCryptoState::DecryptFailed
                                | RemoteCryptoState::Declined
                                | RemoteCryptoState::Expired
                        )
                    )
                })
                .count()
        );
        assert_eq!(
            event_data["queued_count"],
            pending
                .fan_out_nodes
                .iter()
                .filter(|target| {
                    matches!(
                        target.remote_state,
                        Some(RemoteCryptoState::CiphertextQueued)
                    )
                })
                .count()
        );
        if let Some(remote_state) = expected_remote_state {
            assert_eq!(event_data["remote_state"], remote_state);
        } else {
            assert!(event_data.get("remote_state").is_none());
        }
        let pending_created_at = chrono::DateTime::parse_from_rfc3339(
            event_data["pending_created_at"]
                .as_str()
                .expect("pending_created_at string"),
        )
        .expect("pending_created_at timestamp");
        assert_eq!(
            pending_created_at.timestamp_millis(),
            pending.created_at.timestamp_millis()
        );
        let pending_expires_at = chrono::DateTime::parse_from_rfc3339(
            event_data["pending_expires_at"]
                .as_str()
                .expect("pending_expires_at string"),
        )
        .expect("pending_expires_at timestamp");
        assert_eq!(
            pending_expires_at.timestamp_millis(),
            pending.expires_at.timestamp_millis()
        );
        assert!(
            chrono::DateTime::parse_from_rfc3339(
                event_data["event_at"].as_str().expect("event_at string")
            )
            .is_ok()
        );
        assert_no_fan_out_secret_fields(event_data);
        for forbidden_key in [
            "node_id",
            "pending_credential_id",
            "routed_via",
            "delivery",
            "error_code",
            "error_kind",
        ] {
            assert!(!object.contains_key(forbidden_key), "{forbidden_key}");
        }
    }

    async fn fan_out_route_fixture(
        prefix: &str,
    ) -> (
        mongodb::Database,
        AppState,
        axum::Router,
        String,
        String,
        Node,
        Node,
    ) {
        let db = test_db(prefix).await;
        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let first = test_node(&actor_id, "fanout-first-node");
        let second = test_node(&actor_id, "fanout-second-node");
        insert_node(&db, &first).await;
        insert_node(&db, &second).await;
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_many([
                test_binding(&actor_id, &first.id, "catalog-svc"),
                test_binding(&actor_id, &second.id, "catalog-svc"),
            ])
            .await
            .expect("insert fan-out bindings");

        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let app = api_app(state.clone());
        (db, state, app, token, actor_id, first, second)
    }

    async fn route_create_fan_out_pending(
        app: axum::Router,
        token: &str,
        owner_user_id: &str,
    ) -> Value {
        let (status, body) = route_json(
            app,
            Method::POST,
            "/api/v1/nodes/credentials/push/fan-out".to_string(),
            token,
            Some(serde_json::json!({
                "owner_user_id": owner_user_id,
                "service_id": "catalog-svc",
                "service_slug": "openclaw",
                "injection_method": "header",
                "field_name": "X-API-Key",
                "target_url": "https://gateway.example.com/v1",
                "label": "Production",
                "remote_crypto": true,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        body
    }

    async fn record_all_fan_out_pubkeys(
        db: &mongodb::Database,
        pending_id: &str,
        first_node_id: &str,
        second_node_id: &str,
    ) -> NodePendingCredential {
        node_pending_credential_service::record_fan_out_pubkey(
            db,
            first_node_id,
            pending_id,
            "v1",
            &b64url(12, 32),
        )
        .await
        .expect("record first fan-out pubkey");
        node_pending_credential_service::record_fan_out_pubkey(
            db,
            second_node_id,
            pending_id,
            "v1",
            &b64url(13, 32),
        )
        .await
        .expect("record second fan-out pubkey");
        load_pending(db, pending_id).await
    }

    async fn create_remote_pending(
        db: &mongodb::Database,
        actor_id: &str,
        node_id: &str,
        service_slug: &str,
    ) -> NodePendingCredential {
        node_pending_credential_service::create_pending_credential(
            db,
            actor_id,
            node_id,
            node_pending_credential_service::CreatePendingCredentialInput {
                service_slug: service_slug.to_string(),
                injection_method: InjectionMethod::Header,
                field_name: "X-API-Key".to_string(),
                target_url: None,
                label: Some("Production".to_string()),
                ttl_secs: 86_400,
                remote_crypto: true,
            },
        )
        .await
        .expect("create remote pending credential")
    }

    async fn create_remote_pending_with_pubkey(
        db: &mongodb::Database,
        actor_id: &str,
        node_id: &str,
        service_slug: &str,
    ) -> NodePendingCredential {
        let pending = create_remote_pending(db, actor_id, node_id, service_slug).await;
        node_pending_credential_service::record_pending_credential_pubkey(
            db,
            node_id,
            &pending.id,
            "v1",
            &b64url(12, 32),
        )
        .await
        .expect("record node pubkey");
        pending
    }

    async fn pending_route_fixture(
        prefix: &str,
        node_name: &str,
        service_slug: &str,
        with_pubkey: bool,
    ) -> (
        mongodb::Database,
        AppState,
        axum::Router,
        String,
        Node,
        NodePendingCredential,
    ) {
        let db = test_db(prefix).await;
        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, node_name);
        insert_node(&db, &node).await;
        let pending = if with_pubkey {
            create_remote_pending_with_pubkey(&db, &actor_id, &node.id, service_slug).await
        } else {
            create_remote_pending(&db, &actor_id, &node.id, service_slug).await
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let app = api_app(state.clone());
        (db, state, app, token, node, pending)
    }

    #[tokio::test]
    async fn route_post_push_pending_credential_nudges_connected_node() {
        let db = test_db("pending_route_push_nudge").await;
        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "push-nudge-node");
        insert_node(&db, &node).await;

        let state = test_app_state(db.clone());
        let (tx, mut rx) = mpsc::channel(1);
        state.node_ws_manager.register_connection(&node.id, tx);
        let token = access_token(&state, &actor_id);
        let app = api_app(state);

        let (status, body) = route_json(
            app,
            Method::POST,
            format!("/api/v1/nodes/{}/credentials/push", node.id),
            &token,
            Some(serde_json::json!({
                "service_slug": "openclaw",
                "injection_method": "header",
                "field_name": "X-API-Key",
                "target_url": "https://gateway.example.com/v1",
                "label": "Production",
                "remote_crypto": false,
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["node_id"], node.id);
        assert_eq!(body["service_slug"], "openclaw");
        assert_eq!(body["injection_method"], "header");
        assert_eq!(body["field_name"], "X-API-Key");
        assert_eq!(body["target_url"], "https://gateway.example.com/v1");
        assert_eq!(body["label"], "Production");
        assert_eq!(body["is_active"], true);

        let NodeOutboundMessage::Text(frame) = rx.try_recv().expect("nudge frame") else {
            panic!("expected text outbound frame");
        };
        let frame: Value = serde_json::from_str(&frame).expect("frame json");
        assert_eq!(
            frame,
            serde_json::json!({ "type": "pending_credentials_available" })
        );

        let stored = load_pending(&db, body["id"].as_str().expect("pending id")).await;
        assert_eq!(stored.node_id, node.id);
        assert_eq!(stored.service_slug, "openclaw");
    }

    #[tokio::test]
    async fn route_get_pending_credential_pubkey_returns_awaiting_then_public_fields() {
        let (db, _state, app, token, node, pending) =
            pending_route_fixture("pending_route_get_pubkey", "pubkey-node", "openclaw", false)
                .await;
        let uri = format!(
            "/api/v1/nodes/{}/credentials/pending/{}",
            node.id, pending.id
        );

        let (status, body) = route_json(app.clone(), Method::GET, uri.clone(), &token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error_code"], 8009);
        assert_eq!(body["error"], "pending_credential_pubkey_awaiting");

        let node_pubkey = b64url(13, 32);
        node_pending_credential_service::record_pending_credential_pubkey(
            &db,
            &node.id,
            &pending.id,
            "v1",
            &node_pubkey,
        )
        .await
        .expect("record node pubkey");

        let (status, body) = route_json(app, Method::GET, uri, &token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["pending_id"], pending.id);
        assert_eq!(body["node_id"], node.id);
        assert_eq!(body["service_slug"], "openclaw");
        assert_eq!(body["version"], "v1");
        assert_eq!(body["node_pubkey"], node_pubkey);
        assert_eq!(body["remote_state"], "pubkey_posted");
        assert_eq!(body["integrity_verification_opt_out"], false);
        assert!(body.get("admin_pubkey").is_none());
        assert!(body.get("nonce").is_none());
        assert!(body.get("ciphertext").is_none());
    }

    #[tokio::test]
    async fn route_post_pending_remote_crypto_initializes_metadata_and_nudges_connected_node() {
        let db = test_db("pending_route_init_remote_crypto").await;
        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let node = test_node(&actor_id, "init-remote-node");
        insert_node(&db, &node).await;
        let pending = node_pending_credential_service::create_pending_credential(
            &db,
            &actor_id,
            &node.id,
            node_pending_credential_service::CreatePendingCredentialInput {
                service_slug: "openclaw".to_string(),
                injection_method: crate::models::node_pending_credential::InjectionMethod::Header,
                field_name: "X-API-Key".to_string(),
                target_url: None,
                label: Some("Production".to_string()),
                ttl_secs: 86_400,
                remote_crypto: false,
            },
        )
        .await
        .expect("create legacy pending credential");
        assert!(pending.crypto.is_none());
        assert!(pending.remote_state.is_none());

        let state = test_app_state(db.clone());
        let (tx, mut rx) = mpsc::channel(1);
        state.node_ws_manager.register_connection(&node.id, tx);
        let token = access_token(&state, &actor_id);
        let app = api_app(state);

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/remote-crypto",
                node.id, pending.id
            ),
            &token,
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], pending.id);
        assert_eq!(body["node_id"], node.id);
        assert_eq!(body["service_slug"], "openclaw");
        assert_eq!(body["remote_state"], "pubkey_awaiting");
        assert_eq!(body["is_active"], true);

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::PubkeyAwaiting));
        assert!(stored.ciphertext_queued_at.is_none());
        assert!(stored.ciphertext_expires_at.is_none());
        let crypto = stored.crypto.as_ref().expect("crypto metadata");
        assert_eq!(crypto.version, "v1");
        assert!(crypto.node_pubkey.is_empty());
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());

        let NodeOutboundMessage::Text(frame) = rx.try_recv().expect("nudge frame") else {
            panic!("expected text outbound frame");
        };
        let frame: Value = serde_json::from_str(&frame).expect("frame json");
        assert_eq!(
            frame,
            serde_json::json!({ "type": "pending_credentials_available" })
        );
    }

    #[tokio::test]
    async fn route_get_pending_credential_pubkey_exposes_org_integrity_opt_out() {
        let db = test_db("pending_route_get_pubkey_optout").await;
        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_org_with_integrity_opt_out(&org_id),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");
        let node = test_node(&org_id, "optout-pubkey-node");
        insert_node(&db, &node).await;
        let pending = create_remote_pending_with_pubkey(&db, &admin_id, &node.id, "openclaw").await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &admin_id);
        let app = api_app(state);

        let (status, body) = route_json(
            app,
            Method::GET,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}",
                node.id, pending.id
            ),
            &token,
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["pending_id"], pending.id);
        assert_eq!(body["integrity_verification_opt_out"], true);
    }

    #[tokio::test]
    async fn route_post_fan_out_push_creates_doc_audits_and_nudges_connected_targets() {
        let (db, state, app, token, actor_id, first, second) =
            fan_out_route_fixture("pending_route_fanout_push_success").await;
        let (first_tx, mut first_rx) = mpsc::channel(2);
        let (second_tx, mut second_rx) = mpsc::channel(2);
        state
            .node_ws_manager
            .register_connection(&first.id, first_tx);
        state
            .node_ws_manager
            .register_connection(&second.id, second_tx);
        let created_audit = audit_service::notify_on_audit_write_for_user(
            "node_credential_rci_fan_out_created",
            actor_id.clone(),
        );

        let body = route_create_fan_out_pending(app, &token, &actor_id).await;

        assert_eq!(body["fan_out_revision"], 1);
        assert_eq!(body["target_count"], 2);
        assert_eq!(body["service_slug"], "openclaw");
        assert_eq!(body["injection_method"], "header");
        assert_eq!(body["field_name"], "X-API-Key");
        assert_eq!(body["target_url"], "https://gateway.example.com/v1");
        assert_eq!(body["label"], "Production");
        assert!(body.get("remote_state").is_none());
        assert_eq!(response_target(&body, &first.id)["generation"], 0);
        assert_eq!(response_target(&body, &second.id)["generation"], 0);
        assert!(
            response_target(&body, &first.id)
                .get("remote_state")
                .is_none()
        );
        assert!(
            response_target(&body, &second.id)
                .get("remote_state")
                .is_none()
        );
        assert_no_fan_out_secret_fields(&body);

        for rx in [&mut first_rx, &mut second_rx] {
            let NodeOutboundMessage::Text(frame) = rx.try_recv().expect("nudge frame") else {
                panic!("expected text outbound frame");
            };
            let frame: Value = serde_json::from_str(&frame).expect("frame json");
            assert_eq!(
                frame,
                serde_json::json!({ "type": "pending_credentials_available" })
            );
        }

        let fanout_id = body["fanout_id"].as_str().expect("fanout id");
        let stored = load_pending(&db, fanout_id).await;
        assert_eq!(stored.id, fanout_id);
        assert_eq!(stored.owner_user_id, actor_id);
        assert_eq!(stored.created_by_user_id, actor_id);
        assert_eq!(stored.node_id, first.id);
        assert!(stored.crypto.is_none());
        assert_eq!(stored.fan_out_revision, 1);
        assert_eq!(stored.fan_out_nodes.len(), 2);
        assert!(stored.remote_state.is_none());
        assert!(
            stored
                .fan_out_nodes
                .iter()
                .all(|target| target.remote_state.is_none())
        );

        let audit = load_audit_entry(&db, created_audit).await;
        assert_fan_out_aggregate_audit_row(
            &audit,
            "node_credential_rci_fan_out_created",
            &actor_id,
            &stored,
            None,
        );
    }

    #[tokio::test]
    async fn route_get_fan_out_pending_and_pubkeys_return_public_shapes() {
        let (db, _state, app, token, actor_id, first, second) =
            fan_out_route_fixture("pending_route_fanout_get_success").await;
        let created = route_create_fan_out_pending(app.clone(), &token, &actor_id).await;
        let fanout_id = created["fanout_id"].as_str().expect("fanout id");

        let (status, body) = route_json(
            app.clone(),
            Method::GET,
            format!("/api/v1/nodes/credentials/pending/{fanout_id}/fan-out"),
            &token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["fanout_id"], fanout_id);
        assert_eq!(body["fan_out_revision"], 1);
        assert_eq!(body["target_count"], 2);
        assert_eq!(body["service_slug"], "openclaw");
        assert!(body.get("remote_state").is_none());
        assert!(
            response_target(&body, &first.id)
                .get("remote_state")
                .is_none()
        );
        assert!(
            response_target(&body, &second.id)
                .get("remote_state")
                .is_none()
        );
        assert_no_fan_out_secret_fields(&body);

        let (status, body) = route_json(
            app.clone(),
            Method::GET,
            format!("/api/v1/nodes/credentials/pending/{fanout_id}/fan-out/pubkeys"),
            &token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["fanout_id"], fanout_id);
        assert_eq!(body["fan_out_revision"], 1);
        assert_eq!(body["target_count"], 2);
        assert_eq!(body["integrity_verification_opt_out"], false);
        let first_pubkey = response_target(&body, &first.id);
        let second_pubkey = response_target(&body, &second.id);
        assert_eq!(first_pubkey["version"], "v1");
        assert_eq!(second_pubkey["version"], "v1");
        assert_eq!(
            first_pubkey["error_code"],
            crate::errors::PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE
        );
        assert_eq!(
            second_pubkey["error_code"],
            crate::errors::PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE
        );
        assert!(first_pubkey.get("node_pubkey").is_none());
        assert!(second_pubkey.get("node_pubkey").is_none());
        assert_no_fan_out_secret_fields(&body);

        node_pending_credential_service::record_fan_out_pubkey(
            &db,
            &first.id,
            fanout_id,
            "v1",
            &b64url(12, 32),
        )
        .await
        .expect("record first pubkey");
        let (status, body) = route_json(
            app,
            Method::GET,
            format!("/api/v1/nodes/credentials/pending/{fanout_id}/fan-out/pubkeys"),
            &token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let first_pubkey = response_target(&body, &first.id);
        let second_pubkey = response_target(&body, &second.id);
        assert_eq!(first_pubkey["node_pubkey"], b64url(12, 32));
        assert_eq!(first_pubkey["remote_state"], "pubkey_posted");
        assert!(first_pubkey.get("error_code").is_none());
        assert_eq!(
            second_pubkey["error_code"],
            crate::errors::PENDING_CREDENTIAL_PUBKEY_AWAITING_CODE
        );
        assert!(body.to_string().contains(&b64url(12, 32)));
        assert!(!body.to_string().contains(&b64url(13, 32)));
    }

    #[tokio::test]
    async fn route_post_fan_out_ciphertexts_accepts_online_and_offline_targets() {
        let (db, state, app, token, actor_id, first, second) =
            fan_out_route_fixture("pending_route_fanout_ciphertexts_success").await;
        let created = route_create_fan_out_pending(app.clone(), &token, &actor_id).await;
        let fanout_id = created["fanout_id"]
            .as_str()
            .expect("fanout id")
            .to_string();
        let pending = record_all_fan_out_pubkeys(&db, &fanout_id, &first.id, &second.id).await;

        let (first_tx, mut first_rx) = mpsc::channel(4);
        state
            .node_ws_manager
            .register_connection(&first.id, first_tx);
        state.node_ws_manager.record_capabilities(
            &first.id,
            &NodeCapabilitiesMsg {
                remote_credential_crypto_v1: true,
                ..NodeCapabilitiesMsg::default()
            },
        );
        let received_first = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_received",
            Some(fanout_id.clone()),
        );
        let received_second = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_received",
            Some(fanout_id.clone()),
        );
        let forwarded_first = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_forwarded",
            Some(fanout_id.clone()),
        );
        let queued_second = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_queued",
            Some(fanout_id.clone()),
        );

        let (status, body) = route_json(
            app,
            Method::POST,
            format!("/api/v1/nodes/credentials/pending/{fanout_id}/fan-out/ciphertexts"),
            &token,
            Some(fan_out_ciphertext_request_for_targets(
                pending.fan_out_revision,
                &first.id,
                &second.id,
            )),
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["fanout_id"], fanout_id);
        assert_eq!(body["fan_out_revision"], pending.fan_out_revision + 1);
        assert_eq!(body["remote_state"], "ciphertext_queued");
        let first_status = response_target(&body, &first.id);
        assert_eq!(first_status["generation"], 0);
        assert_eq!(first_status["remote_state"], "ciphertext_received");
        assert_eq!(first_status["delivery_status"], "sent");
        assert!(first_status.get("error_code").is_none());
        let second_status = response_target(&body, &second.id);
        assert_eq!(second_status["generation"], 0);
        assert_eq!(second_status["remote_state"], "ciphertext_queued");
        assert_eq!(second_status["delivery_status"], "queued");
        assert_no_fan_out_secret_fields(&body);

        let NodeOutboundMessage::Text(frame) = first_rx.recv().await.expect("ciphertext frame")
        else {
            panic!("expected text outbound frame");
        };
        let frame: Value = serde_json::from_str(&frame).expect("frame json");
        assert_eq!(frame["type"], "pending_credential_ciphertext");
        assert_eq!(frame["pending_id"], fanout_id);
        assert_eq!(frame["version"], "v1");
        assert_eq!(frame["admin_pubkey"], b64url(10, 32));
        assert_eq!(frame["nonce"], b64url(11, 24));
        assert_eq!(
            frame["ciphertext"],
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3])
        );

        let stored = load_pending(&db, &fanout_id).await;
        assert_eq!(stored.fan_out_revision, pending.fan_out_revision + 1);
        assert_eq!(
            stored.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        let first_target = node_pending_credential_service::fan_out_target(&stored, &first.id)
            .expect("first target");
        assert_eq!(
            first_target.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        );
        assert!(first_target.ciphertext_queued_at.is_none());
        assert!(first_target.ciphertext_expires_at.is_none());
        assert_eq!(
            first_target.crypto.ciphertext.as_deref(),
            Some([1, 2, 3].as_slice())
        );
        let second_target = node_pending_credential_service::fan_out_target(&stored, &second.id)
            .expect("second target");
        assert_eq!(
            second_target.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        assert!(second_target.ciphertext_queued_at.is_some());
        assert!(second_target.ciphertext_expires_at.is_some());
        assert_eq!(
            second_target.crypto.ciphertext.as_deref(),
            Some([1, 2, 3, 4].as_slice())
        );

        let received = [
            load_audit_entry(&db, received_first).await,
            load_audit_entry(&db, received_second).await,
        ];
        for audit in &received {
            let node_id = audit.event_data.as_ref().unwrap()["node_id"]
                .as_str()
                .expect("audit node id");
            assert!(
                node_id == first.id || node_id == second.id,
                "unexpected node id {node_id}"
            );
            assert_fan_out_target_audit_row(
                audit,
                "node_credential_rci_ciphertext_received",
                &stored,
                node_id,
                Some("ciphertext_received"),
                &[],
            );
        }
        let forwarded = load_audit_entry(&db, forwarded_first).await;
        assert_fan_out_target_audit_row(
            &forwarded,
            "node_credential_rci_ciphertext_forwarded",
            &stored,
            &first.id,
            Some("ciphertext_received"),
            &["delivery"],
        );
        assert_eq!(
            forwarded.event_data.as_ref().unwrap()["delivery"],
            "online_forward"
        );
        let queued = load_audit_entry(&db, queued_second).await;
        assert_fan_out_target_audit_row(
            &queued,
            "node_credential_rci_ciphertext_queued",
            &stored,
            &second.id,
            Some("ciphertext_queued"),
            &[
                "ciphertext_expires_at",
                "ciphertext_queued_at",
                "delivery",
                "error_code",
                "error_kind",
            ],
        );
        let queued_data = queued.event_data.as_ref().unwrap();
        assert_eq!(queued_data["delivery"], "offline_queue");
        assert_eq!(
            queued_data["error_code"],
            PENDING_CREDENTIAL_NODE_OFFLINE_CODE
        );
        assert_eq!(queued_data["error_kind"], "pending_credential_node_offline");
    }

    #[tokio::test]
    async fn route_retry_failed_fan_out_resets_only_failed_targets_and_audits() {
        let (db, state, app, token, actor_id, first, second) =
            fan_out_route_fixture("pending_route_fanout_retry_success").await;
        let created = route_create_fan_out_pending(app.clone(), &token, &actor_id).await;
        let fanout_id = created["fanout_id"]
            .as_str()
            .expect("fanout id")
            .to_string();
        let pending = record_all_fan_out_pubkeys(&db, &fanout_id, &first.id, &second.id).await;
        node_pending_credential_service::store_fan_out_ciphertexts_revision_guard(
            &db,
            &actor_id,
            &fanout_id,
            node_pending_credential_service::StoreFanOutCiphertextsInput {
                fan_out_revision: pending.fan_out_revision,
                items: vec![
                    node_pending_credential_service::StoreFanOutCiphertextItemInput::new(
                        first.id.clone(),
                        0,
                        "v1".to_string(),
                        b64url(10, 32),
                        b64url(11, 24),
                        vec![1, 2, 3],
                    ),
                    node_pending_credential_service::StoreFanOutCiphertextItemInput::new(
                        second.id.clone(),
                        0,
                        "v1".to_string(),
                        b64url(10, 32),
                        b64url(11, 24),
                        vec![1, 2, 3, 4],
                    ),
                ],
                online_node_ids: [first.id.clone(), second.id.clone()].into_iter().collect(),
            },
            Utc::now(),
        )
        .await
        .expect("store fan-out ciphertexts");
        node_pending_credential_service::record_fan_out_decrypt_result(
            &db,
            &first.id,
            &fanout_id,
            node_pending_credential_service::PendingCredentialDecryptOutcome::Ok,
            None,
            Utc::now(),
        )
        .await
        .expect("first consumes");
        let failed = node_pending_credential_service::record_fan_out_decrypt_result(
            &db,
            &second.id,
            &fanout_id,
            node_pending_credential_service::PendingCredentialDecryptOutcome::Error,
            Some(crate::errors::PENDING_CREDENTIAL_DECRYPT_FAILED_CODE),
            Utc::now(),
        )
        .await
        .expect("second fails");
        assert_eq!(
            failed.remote_state,
            Some(RemoteCryptoState::PartialDecrypted)
        );
        let (second_tx, mut second_rx) = mpsc::channel(2);
        state
            .node_ws_manager
            .register_connection(&second.id, second_tx);
        let retry_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_fan_out_retry_started",
            Some(fanout_id.clone()),
        );

        let (status, body) = route_json(
            app,
            Method::POST,
            format!("/api/v1/nodes/credentials/pending/{fanout_id}/fan-out/retry-failed"),
            &token,
            Some(serde_json::json!({
                "fan_out_revision": failed.fan_out_revision,
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["fanout_id"], fanout_id);
        assert_eq!(body["fan_out_revision"], failed.fan_out_revision + 1);
        assert_eq!(body["remote_state"], "partial_decrypted");
        let first_status = response_target(&body, &first.id);
        assert_eq!(first_status["generation"], 0);
        assert_eq!(first_status["remote_state"], "consumed");
        let second_status = response_target(&body, &second.id);
        assert_eq!(second_status["generation"], 1);
        assert!(second_status.get("remote_state").is_none());
        assert!(second_status.get("error_code").is_none());
        assert!(second_status.get("error_kind").is_none());

        let NodeOutboundMessage::Text(frame) = second_rx.try_recv().expect("retry nudge") else {
            panic!("expected text outbound frame");
        };
        let frame: Value = serde_json::from_str(&frame).expect("frame json");
        assert_eq!(
            frame,
            serde_json::json!({ "type": "pending_credentials_available" })
        );

        let stored = load_pending(&db, &fanout_id).await;
        let first_target = node_pending_credential_service::fan_out_target(&stored, &first.id)
            .expect("first target");
        assert_eq!(first_target.generation, 0);
        assert_eq!(first_target.remote_state, Some(RemoteCryptoState::Consumed));
        let second_target = node_pending_credential_service::fan_out_target(&stored, &second.id)
            .expect("second target");
        assert_eq!(second_target.generation, 1);
        assert!(second_target.remote_state.is_none());
        assert!(second_target.crypto.node_pubkey.is_empty());
        assert!(second_target.crypto.admin_pubkey.is_none());
        assert!(second_target.crypto.nonce.is_none());
        assert!(second_target.crypto.ciphertext.is_none());

        let audit = load_audit_entry(&db, retry_audit).await;
        assert_fan_out_aggregate_audit_row(
            &audit,
            "node_credential_rci_fan_out_retry_started",
            &actor_id,
            &stored,
            Some("partial_decrypted"),
        );
    }

    #[tokio::test]
    async fn partial_fan_out_expiry_writes_expired_aggregate_audit_row() {
        let (db, _state, _app, _token, actor_id, first, second) =
            fan_out_route_fixture("pending_route_fanout_expired_audit").await;
        let pending = node_pending_credential_service::create_fan_out_pending_credential(
            &db,
            &actor_id,
            node_pending_credential_service::CreateFanOutPendingCredentialInput {
                owner_user_id: actor_id.clone(),
                service_id: "catalog-svc".to_string(),
                service_slug: "openclaw".to_string(),
                injection_method: InjectionMethod::Header,
                field_name: "X-API-Key".to_string(),
                target_url: None,
                label: None,
                ttl_secs: 86_400,
                remote_crypto: true,
            },
        )
        .await
        .expect("create fan-out pending")
        .pending;
        let pending = record_all_fan_out_pubkeys(&db, &pending.id, &first.id, &second.id).await;
        node_pending_credential_service::store_fan_out_ciphertexts_revision_guard(
            &db,
            &actor_id,
            &pending.id,
            node_pending_credential_service::StoreFanOutCiphertextsInput {
                fan_out_revision: pending.fan_out_revision,
                items: vec![
                    node_pending_credential_service::StoreFanOutCiphertextItemInput::new(
                        first.id.clone(),
                        0,
                        "v1".to_string(),
                        b64url(10, 32),
                        b64url(11, 24),
                        vec![1, 2, 3],
                    ),
                    node_pending_credential_service::StoreFanOutCiphertextItemInput::new(
                        second.id.clone(),
                        0,
                        "v1".to_string(),
                        b64url(10, 32),
                        b64url(11, 24),
                        vec![1, 2, 3, 4],
                    ),
                ],
                online_node_ids: [first.id.clone(), second.id.clone()].into_iter().collect(),
            },
            Utc::now(),
        )
        .await
        .expect("store fan-out ciphertexts");
        node_pending_credential_service::record_fan_out_decrypt_result(
            &db,
            &first.id,
            &pending.id,
            node_pending_credential_service::PendingCredentialDecryptOutcome::Ok,
            None,
            Utc::now(),
        )
        .await
        .expect("first consumes");
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending.id },
                doc! {
                    "$set": {
                        "expires_at": mongodb::bson::DateTime::from_chrono(
                            Utc::now() - chrono::Duration::seconds(1)
                        ),
                    },
                },
            )
            .await
            .expect("force top-level expiry");
        let expired_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_fan_out_expired",
            Some(pending.id.clone()),
        );

        let summaries = node_pending_credential_service::expire_queued_ciphertexts_with_summaries(
            &db,
            Utc::now(),
        )
        .await
        .expect("expire partial fan-out");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].pending_credential_id, pending.id);
        assert_eq!(summaries[0].node_id, second.id);
        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::Expired));
        let first_target = node_pending_credential_service::fan_out_target(&stored, &first.id)
            .expect("first target");
        assert_eq!(first_target.remote_state, Some(RemoteCryptoState::Consumed));
        let second_target = node_pending_credential_service::fan_out_target(&stored, &second.id)
            .expect("second target");
        assert_eq!(second_target.remote_state, Some(RemoteCryptoState::Expired));
        assert!(second_target.crypto.admin_pubkey.is_none());
        assert!(second_target.crypto.nonce.is_none());
        assert!(second_target.crypto.ciphertext.is_none());

        let audit = load_audit_entry(&db, expired_audit).await;
        assert_fan_out_aggregate_audit_row(
            &audit,
            "node_credential_rci_fan_out_expired",
            &actor_id,
            &stored,
            Some("expired"),
        );
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_sends_to_online_capable_node() {
        let (db, state, app, token, node, pending) =
            pending_route_fixture("pending_route_post_sent", "sent-node", "openclaw", true).await;
        let (tx, mut rx) = mpsc::channel(4);
        state.node_ws_manager.register_connection(&node.id, tx);
        state.node_ws_manager.record_capabilities(
            &node.id,
            &NodeCapabilitiesMsg {
                remote_credential_crypto_v1: true,
                ..NodeCapabilitiesMsg::default()
            },
        );
        let received_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_received",
            Some(pending.id.clone()),
        );
        let forwarded_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_forwarded",
            Some(pending.id.clone()),
        );
        let integrity_audit = audit_service::notify_on_audit_write(
            "node_credential_ciphertext_submitted",
            Some(pending.id.clone()),
        );

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(ciphertext_request(vec![1, 2, 3])),
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["delivery_status"], "sent");
        assert_eq!(body["remote_state"], "ciphertext_received");
        assert!(body.get("error_code").is_none());

        let NodeOutboundMessage::Text(frame) = rx.recv().await.expect("outbound frame") else {
            panic!("expected text outbound frame");
        };
        let frame: Value = serde_json::from_str(&frame).expect("frame json");
        assert_eq!(frame["type"], "pending_credential_ciphertext");
        assert_eq!(frame["pending_id"], pending.id);
        assert_eq!(frame["version"], "v1");
        assert_eq!(frame["admin_pubkey"], b64url(10, 32));
        assert_eq!(frame["nonce"], b64url(11, 24));
        assert_eq!(
            frame["ciphertext"],
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3])
        );

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(
            stored.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        );
        assert!(stored.ciphertext_queued_at.is_none());

        let received = load_audit_entry(&db, received_audit).await;
        assert_rci_audit_row(
            &received,
            "node_credential_rci_ciphertext_received",
            &stored,
            Some("ciphertext_received"),
            &[],
        );
        let forwarded = load_audit_entry(&db, forwarded_audit).await;
        assert_rci_audit_row(
            &forwarded,
            "node_credential_rci_ciphertext_forwarded",
            &stored,
            Some("ciphertext_received"),
            &["delivery"],
        );
        assert_eq!(
            forwarded.event_data.as_ref().unwrap()["delivery"],
            "online_forward"
        );
        let integrity = load_audit_entry(&db, integrity_audit).await;
        let integrity_data = integrity.event_data.as_ref().expect("integrity audit data");
        assert_eq!(integrity_data["node_id"], node.id);
        assert_eq!(integrity_data["pending_credential_id"], pending.id);
        assert_eq!(
            integrity_data["integrity_verification"]["mode"],
            "admin_verified"
        );
        assert_eq!(
            integrity_data["integrity_verification"]["fingerprint_sha384_prefix"],
            "aaaaaaaaaaaa"
        );
        let integrity_json = integrity_data.to_string();
        assert!(!integrity_json.contains(&"a".repeat(96)));
        assert!(!integrity_json.contains(&b64url(10, 32)));
        assert!(!integrity_json.contains(&b64url(11, 24)));
        assert!(
            !integrity_json
                .contains(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3]))
        );
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_fails_closed_without_manifest_for_non_opt_out_owner() {
        let (db, mut state, _app, token, node, pending) = pending_route_fixture(
            "pending_route_integrity_manifest_missing",
            "manifest-missing-node",
            "openclaw",
            true,
        )
        .await;
        state.config.release_integrity_manifest_url = None;
        let app = api_app_preserving_config(state);

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(ciphertext_request(vec![1, 2, 3])),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], 1008);
        assert_eq!(body["error"], "validation_error");
        assert!(
            body["message"]
                .as_str()
                .expect("message")
                .contains("release integrity manifest URL is not configured")
        );
        let stored = load_pending(&db, &pending.id).await;
        assert_pubkey_only_pending(&stored, &b64url(12, 32));
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_rejects_expired_integrity_metadata() {
        let (db, state, app, token, node, pending) = pending_route_fixture(
            "pending_route_integrity_expired",
            "expired-integrity-node",
            "openclaw",
            true,
        )
        .await;
        let mut body = ciphertext_request(vec![1, 2, 3]);
        body["integrity_verification"]["verified_at"] = Value::String(
            (Utc::now() - chrono::Duration::seconds(state.config.jwt_relay_reply_ttl_secs + 60))
                .to_rfc3339(),
        );

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(body),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], 1008);
        assert!(
            body["message"]
                .as_str()
                .expect("message")
                .contains("integrity verification has expired")
        );
        let stored = load_pending(&db, &pending.id).await;
        assert_pubkey_only_pending(&stored, &b64url(12, 32));
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_accepts_org_policy_opt_out_without_manifest() {
        let db = test_db("pending_route_integrity_org_optout").await;
        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_org_with_integrity_opt_out(&org_id),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");
        let node = test_node(&org_id, "org-optout-node");
        insert_node(&db, &node).await;
        let pending = create_remote_pending_with_pubkey(&db, &admin_id, &node.id, "openclaw").await;
        let mut state = test_app_state(db.clone());
        state.config.release_integrity_manifest_url = None;
        let token = access_token(&state, &admin_id);
        let app = api_app_preserving_config(state);
        let integrity_audit = audit_service::notify_on_audit_write(
            "node_credential_ciphertext_submitted",
            Some(pending.id.clone()),
        );
        let mut body = ciphertext_request(vec![4, 5, 6]);
        body["integrity_verification"] = serde_json::json!({
            "mode": "org_policy_opt_out",
            "fingerprint_sha384_hex": null,
            "verified_at": null,
            "manifest_url_configured": false,
        });

        let (status, response) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(body),
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(response["delivery_status"], "queued");
        let audit = load_audit_entry(&db, integrity_audit).await;
        let data = audit.event_data.as_ref().expect("audit data");
        assert_eq!(data["owner_user_id"], org_id);
        assert_eq!(data["integrity_verification"]["mode"], "org_policy_opt_out");
        assert!(data["integrity_verification"]["fingerprint_sha384_prefix"].is_null());
        assert!(data["integrity_verification"]["verified_at"].is_null());
        assert_eq!(
            data["integrity_verification"]["manifest_url_configured"],
            false
        );
        let audit_json = data.to_string();
        assert!(!audit_json.contains(&b64url(10, 32)));
        assert!(!audit_json.contains(&b64url(11, 24)));
        assert!(
            !audit_json
                .contains(&base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([4, 5, 6]))
        );
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_queues_for_offline_unsupported_and_send_failure() {
        let (db, state, app, token, node, offline_pending) = pending_route_fixture(
            "pending_route_post_queued",
            "queued-node",
            "offline-service",
            true,
        )
        .await;

        let cases = [
            ("offline", offline_pending.id.clone()),
            (
                "unsupported",
                create_remote_pending_with_pubkey(&db, &node.user_id, &node.id, "unsupported")
                    .await
                    .id,
            ),
            (
                "send-failure",
                create_remote_pending_with_pubkey(&db, &node.user_id, &node.id, "send-failure")
                    .await
                    .id,
            ),
        ];

        for (case, pending_id) in cases {
            match case {
                "unsupported" => {
                    let (tx, _rx) = mpsc::channel(4);
                    state.node_ws_manager.register_connection(&node.id, tx);
                }
                "send-failure" => {
                    let (tx, _rx) = mpsc::channel(1);
                    state.node_ws_manager.register_connection(&node.id, tx);
                    state.node_ws_manager.record_capabilities(
                        &node.id,
                        &NodeCapabilitiesMsg {
                            remote_credential_crypto_v1: true,
                            ..NodeCapabilitiesMsg::default()
                        },
                    );
                    state
                        .node_ws_manager
                        .send_pending_credentials_available(&node.id)
                        .expect("pre-fill writer queue");
                }
                _ => {}
            }
            let received_audit = audit_service::notify_on_audit_write(
                "node_credential_rci_ciphertext_received",
                Some(pending_id.clone()),
            );
            let queued_audit = audit_service::notify_on_audit_write(
                "node_credential_rci_ciphertext_queued",
                Some(pending_id.clone()),
            );

            let (status, body) = route_json(
                app.clone(),
                Method::POST,
                format!(
                    "/api/v1/nodes/{}/credentials/pending/{pending_id}/ciphertext",
                    node.id
                ),
                &token,
                Some(ciphertext_request(vec![case.len() as u8])),
            )
            .await;

            assert_eq!(status, StatusCode::ACCEPTED, "{case}");
            assert_eq!(body["delivery_status"], "queued", "{case}");
            assert_eq!(body["remote_state"], "ciphertext_queued", "{case}");
            assert_eq!(body["error_code"], PENDING_CREDENTIAL_NODE_OFFLINE_CODE);

            let stored = load_pending(&db, &pending_id).await;
            assert_eq!(
                stored.remote_state,
                Some(RemoteCryptoState::CiphertextQueued),
                "{case}"
            );
            assert!(stored.ciphertext_queued_at.is_some(), "{case}");
            assert!(stored.ciphertext_expires_at.is_some(), "{case}");

            let received = load_audit_entry(&db, received_audit).await;
            assert_rci_audit_row(
                &received,
                "node_credential_rci_ciphertext_received",
                &stored,
                Some("ciphertext_received"),
                &[],
            );
            let queued = load_audit_entry(&db, queued_audit).await;
            assert_rci_audit_row(
                &queued,
                "node_credential_rci_ciphertext_queued",
                &stored,
                Some("ciphertext_queued"),
                &[
                    "ciphertext_expires_at",
                    "ciphertext_queued_at",
                    "delivery",
                    "error_code",
                    "error_kind",
                ],
            );
            let queued_data = queued.event_data.as_ref().unwrap();
            assert_eq!(queued_data["delivery"], "offline_queue", "{case}");
            assert_eq!(
                queued_data["error_code"], PENDING_CREDENTIAL_NODE_OFFLINE_CODE,
                "{case}"
            );
            assert_eq!(
                queued_data["error_kind"], "pending_credential_node_offline",
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_before_pubkey_audits_awaiting() {
        let (db, _state, app, token, node, pending) = pending_route_fixture(
            "pending_route_post_before_pubkey_audit",
            "awaiting-node",
            "openclaw",
            false,
        )
        .await;
        let awaiting_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_pubkey_awaiting",
            Some(pending.id.clone()),
        );

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(ciphertext_request(vec![1, 2, 3])),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error_code"], 8009);
        assert_eq!(body["error"], "pending_credential_pubkey_awaiting");
        let stored = load_pending(&db, &pending.id).await;
        let audit = load_audit_entry(&db, awaiting_audit).await;
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_pubkey_awaiting",
            &stored,
            None,
            &["error_code", "error_kind"],
        );
        let event_data = audit.event_data.as_ref().unwrap();
        assert_eq!(event_data["error_code"], 8009);
        assert_eq!(
            event_data["error_kind"],
            "pending_credential_pubkey_awaiting"
        );
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_returns_queue_full() {
        let (db, _state, app, token, node, _pending) = pending_route_fixture(
            "pending_route_post_queue_full",
            "queue-full-node",
            "seed-service",
            true,
        )
        .await;
        let now = Utc::now();
        for index in 0..node_pending_credential_service::MAX_OFFLINE_CIPHERTEXT_QUEUE_PER_NODE {
            let pending = create_remote_pending_with_pubkey(
                &db,
                &node.user_id,
                &node.id,
                &format!("queued-{index}"),
            )
            .await;
            node_pending_credential_service::store_pending_ciphertext_first_writer_wins(
                &db,
                &node.user_id,
                &node.id,
                &pending.id,
                node_pending_credential_service::StorePendingCiphertextInput::new(
                    b64url(index as u8, 32),
                    b64url(index as u8 + 1, 24),
                    vec![index as u8],
                ),
                false,
                now,
            )
            .await
            .expect("queue seed ciphertext");
        }
        let pending =
            create_remote_pending_with_pubkey(&db, &node.user_id, &node.id, "queue-full-final")
                .await;
        let queue_full_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_queue_full",
            Some(pending.id.clone()),
        );

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(ciphertext_request(vec![42])),
        )
        .await;

        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error_code"], PENDING_CREDENTIAL_QUEUE_FULL_CODE);
        assert_eq!(body["error"], "pending_credential_queue_full");

        let stored = load_pending(&db, &pending.id).await;
        let audit = load_audit_entry(&db, queue_full_audit).await;
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_queue_full",
            &stored,
            Some("pubkey_posted"),
            &["error_code", "error_kind"],
        );
        let event_data = audit.event_data.as_ref().unwrap();
        assert_eq!(event_data["error_code"], PENDING_CREDENTIAL_QUEUE_FULL_CODE);
        assert_eq!(event_data["error_kind"], "pending_credential_queue_full");
    }

    #[tokio::test]
    async fn route_cancel_pending_credential_writes_rci_canceled_audit_row() {
        let (db, _state, app, token, node, pending) = pending_route_fixture(
            "pending_route_cancel_rci_audit",
            "cancel-rci-node",
            "cancel-rci",
            false,
        )
        .await;
        let canceled_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_canceled",
            Some(pending.id.clone()),
        );

        let (status, body) = route_json(
            app,
            Method::DELETE,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}",
                node.id, pending.id
            ),
            &token,
            None,
        )
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_null());
        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        let audit = load_audit_entry(&db, canceled_audit).await;
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_canceled",
            &stored,
            Some("canceled"),
            &[],
        );
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_rejects_non_writable_actor_without_state_change() {
        let db = test_db("pending_route_post_acl_denied").await;

        let admin_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let stranger_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&member_id, UserType::Person),
                test_user(&stranger_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_id, &member_id, OrgRole::Member, None),
            ])
            .await
            .expect("insert memberships");
        let node = test_node(&org_id, "org-node");
        insert_node(&db, &node).await;
        let pending =
            create_remote_pending_with_pubkey(&db, &admin_id, &node.id, "acl-denied").await;
        let expected_node_pubkey = b64url(12, 32);
        let before = load_pending(&db, &pending.id).await;
        assert_pubkey_only_pending(&before, &expected_node_pubkey);

        let state = test_app_state(db.clone());
        let app = api_app(state.clone());
        let uri = format!(
            "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
            node.id, pending.id
        );

        for denied_actor_id in [&member_id, &stranger_id] {
            let token = access_token(&state, denied_actor_id);
            let (status, body) = route_json(
                app.clone(),
                Method::POST,
                uri.clone(),
                &token,
                Some(ciphertext_request(vec![1, 2, 3])),
            )
            .await;

            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body["error_code"], 8000);
            assert_eq!(body["error"], "node_not_found");
            let stored = load_pending(&db, &pending.id).await;
            assert_pubkey_only_pending(&stored, &expected_node_pubkey);
        }
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_rejects_invalid_version() {
        let (_db, _state, app, token, node, pending) = pending_route_fixture(
            "pending_route_post_bad_version",
            "bad-version-node",
            "openclaw",
            true,
        )
        .await;
        let mut body = ciphertext_request(vec![1]);
        body["version"] = Value::String("v2".to_string());

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(body),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], 8007);
        assert_eq!(body["error"], "pending_credential_version_unsupported");
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_rejects_bad_base64_and_key_lengths() {
        let (_db, _state, app, token, node, pending) = pending_route_fixture(
            "pending_route_post_bad_base64",
            "bad-base64-node",
            "openclaw",
            true,
        )
        .await;
        let uri = format!(
            "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
            node.id, pending.id
        );
        let invalid_cases = [
            (
                "bad-admin-pubkey",
                "admin_pubkey",
                Value::String("%%%".to_string()),
            ),
            (
                "padded-admin-pubkey",
                "admin_pubkey",
                Value::String(format!("{}=", b64url(1, 32))),
            ),
            (
                "short-admin-pubkey",
                "admin_pubkey",
                Value::String(b64url(1, 31)),
            ),
            ("short-nonce", "nonce", Value::String(b64url(2, 23))),
            (
                "bad-ciphertext",
                "ciphertext",
                Value::String("%%%".to_string()),
            ),
            (
                "padded-ciphertext",
                "ciphertext",
                Value::String(format!("{}=", b64url(3, 8))),
            ),
        ];

        for (case, field, value) in invalid_cases {
            let mut body = ciphertext_request(vec![1, 2, 3]);
            body[field] = value;
            let (status, body) =
                route_json(app.clone(), Method::POST, uri.clone(), &token, Some(body)).await;

            assert_eq!(status, StatusCode::BAD_REQUEST, "{case}");
            assert_eq!(body["error_code"], 1008, "{case}");
            assert_eq!(body["error"], "validation_error", "{case}");
        }
    }

    #[tokio::test]
    async fn route_post_pending_ciphertext_rejects_oversized_ciphertext() {
        let (db, _state, app, token, node, pending) = pending_route_fixture(
            "pending_route_post_oversized",
            "oversized-node",
            "openclaw",
            true,
        )
        .await;
        let too_large_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_too_large",
            Some(pending.id.clone()),
        );

        let (status, body) = route_json(
            app,
            Method::POST,
            format!(
                "/api/v1/nodes/{}/credentials/pending/{}/ciphertext",
                node.id, pending.id
            ),
            &token,
            Some(ciphertext_request(vec![
                9;
                node_pending_credential_service::MAX_CIPHERTEXT_SIZE
                    + 1
            ])),
        )
        .await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            body["error_code"],
            PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE
        );
        assert_eq!(body["error"], "pending_credential_ciphertext_too_large");

        let stored = load_pending(&db, &pending.id).await;
        let audit = load_audit_entry(&db, too_large_audit).await;
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_ciphertext_too_large",
            &stored,
            Some("pubkey_posted"),
            &["error_code", "error_kind"],
        );
        let event_data = audit.event_data.as_ref().unwrap();
        assert_eq!(
            event_data["error_code"],
            PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE
        );
        assert_eq!(
            event_data["error_kind"],
            "pending_credential_ciphertext_too_large"
        );
    }

    #[tokio::test]
    async fn route_post_fan_out_ciphertexts_rejects_per_element_oversized_ciphertext() {
        let db = test_db("pending_route_fanout_per_element_413").await;
        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let app = api_app(state);

        let (status, body) = route_json(
            app,
            Method::POST,
            "/api/v1/nodes/credentials/pending/fanout-oversized/fan-out/ciphertexts".to_string(),
            &token,
            Some(fan_out_ciphertext_request(vec![
                9;
                node_pending_credential_service::MAX_CIPHERTEXT_SIZE
                    + 1
            ])),
        )
        .await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            body["error_code"],
            PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE
        );
        assert_eq!(body["error"], "pending_credential_ciphertext_too_large");
    }

    #[tokio::test]
    async fn route_post_fan_out_ciphertexts_rejects_body_limit_oversized_aggregate() {
        let db = test_db("pending_route_fanout_body_limit_413").await;
        let actor_id = Uuid::new_v4().to_string();
        insert_users(&db, vec![test_user(&actor_id, UserType::Person)]).await;
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let app = api_app(state);
        let oversized_ciphertext = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![
            1;
            node_pending_credential_service::MAX_FAN_OUT_HTTP_BODY_BYTES
        ]);
        let body = serde_json::json!({
            "fan_out_revision": 1,
            "items": [
                {
                    "node_id": "node-a",
                    "generation": 0,
                    "version": "v1",
                    "admin_pubkey": b64url(10, 32),
                    "nonce": b64url(11, 24),
                    "ciphertext": oversized_ciphertext,
                }
            ],
        })
        .to_string();

        let (status, _body) = route_raw(
            app,
            Method::POST,
            "/api/v1/nodes/credentials/pending/fanout-body/fan-out/ciphertexts".to_string(),
            &token,
            body,
        )
        .await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn create_registration_token_accepts_explicit_direct_owner_scope() {
        let Some(db) = connect_test_database("node_token_direct").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db.clone());
        let Json(response) = create_registration_token(
            State(state),
            test_auth_user(&actor_id),
            Json(CreateRegistrationTokenRequest {
                name: "direct-node".to_string(),
                owner_user_id: Some(actor_id.clone()),
            }),
        )
        .await
        .expect("explicit direct owner should be allowed");

        let stored = db
            .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
            .find_one(doc! { "_id": &response.token_id })
            .await
            .expect("query token")
            .expect("token exists");
        assert_eq!(stored.user_id, actor_id);
        assert_eq!(stored.name, "direct-node");
    }

    #[tokio::test]
    async fn create_registration_token_accepts_org_admin_owner_scope() {
        let Some(db) = connect_test_database("node_token_org_admin").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let state = test_app_state(db.clone());
        let Json(response) = create_registration_token(
            State(state),
            test_auth_user(&admin_id),
            Json(CreateRegistrationTokenRequest {
                name: "org-node".to_string(),
                owner_user_id: Some(org_id.clone()),
            }),
        )
        .await
        .expect("org admin can create owner-scoped token");

        let stored = db
            .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
            .find_one(doc! { "_id": &response.token_id })
            .await
            .expect("query token")
            .expect("token exists");
        assert_eq!(stored.user_id, org_id);
        assert_eq!(stored.name, "org-node");
    }

    #[tokio::test]
    async fn create_registration_token_rejects_non_admin_owner_scope() {
        let Some(db) = connect_test_database("node_token_non_admin").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .expect("insert membership");

        let state = test_app_state(db);
        let err = create_registration_token(
            State(state),
            test_auth_user(&member_id),
            Json(CreateRegistrationTokenRequest {
                name: "org-node".to_string(),
                owner_user_id: Some(org_id),
            }),
        )
        .await
        .expect_err("org member cannot create owner-scoped token");

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn create_registration_token_counts_nodes_against_requested_owner_not_actor() {
        let Some(db) = connect_test_database("node_token_owner_cap").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let actor_nodes: Vec<Node> = (0..10)
            .map(|idx| test_node(&admin_id, &format!("actor-node-{idx}")))
            .collect();
        db.collection::<Node>(NODES)
            .insert_many(actor_nodes)
            .await
            .expect("insert actor nodes at personal cap");

        let state = test_app_state(db.clone());
        let Json(response) = create_registration_token(
            State(state),
            test_auth_user(&admin_id),
            Json(CreateRegistrationTokenRequest {
                name: "org-node".to_string(),
                owner_user_id: Some(org_id.clone()),
            }),
        )
        .await
        .expect("actor personal cap should not block org-owned token");

        let stored = db
            .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
            .find_one(doc! { "_id": &response.token_id })
            .await
            .expect("query token")
            .expect("token exists");
        assert_eq!(stored.user_id, org_id);
    }

    #[tokio::test]
    async fn list_nodes_returns_owner_metadata_for_personal_and_org_nodes() {
        let Some(db) = connect_test_database("node_list_owner_metadata").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let personal_node = test_node(&actor_id, "personal-node");
        let org_node = test_node(&org_id, "org-node");
        db.collection::<Node>(NODES)
            .insert_many([personal_node.clone(), org_node.clone()])
            .await
            .expect("insert nodes");

        let state = test_app_state(db);
        let Json(response) = list_nodes(State(state), test_auth_user(&actor_id))
            .await
            .expect("list nodes");

        let personal = response
            .nodes
            .iter()
            .find(|node| node.id == personal_node.id)
            .expect("personal node listed");
        assert_eq!(personal.owner.kind, node_service::NodeOwnerKind::User);
        assert_eq!(personal.owner.id, actor_id);

        let org = response
            .nodes
            .iter()
            .find(|node| node.id == org_node.id)
            .expect("org node listed");
        assert_eq!(org.owner.kind, node_service::NodeOwnerKind::Org);
        assert_eq!(org.owner.id, org_id);
        assert_eq!(org.owner.display_name, "Test Org");
    }

    #[tokio::test]
    async fn get_node_returns_owner_metadata_for_org_member() {
        let Some(db) = connect_test_database("node_get_owner_metadata").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .expect("insert membership");
        let org_node = test_node(&org_id, "org-node");
        db.collection::<Node>(NODES)
            .insert_one(org_node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = get_node(
            State(state),
            test_auth_user(&member_id),
            Path(org_node.id.clone()),
        )
        .await
        .expect("get org node");

        assert_eq!(response.id, org_node.id);
        assert_eq!(response.owner.kind, node_service::NodeOwnerKind::Org);
        assert_eq!(response.owner.id, org_id);
    }

    #[tokio::test]
    async fn transfer_personal_node_to_admin_org_succeeds_and_detaches_old_routes() {
        let Some(db) = connect_test_database("node_transfer_personal_to_org").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let node = test_node(&admin_id, "edge-node");
        let binding = test_binding(&admin_id, &node.id, "svc-old");
        let old_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &admin_id,
            "old-service",
            &Uuid::new_v4().to_string(),
            Some("svc-old"),
            Some(&node.id),
        );
        let new_owner_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "org-service",
            &Uuid::new_v4().to_string(),
            Some("svc-org"),
            Some(&node.id),
        );
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(binding.clone())
            .await
            .expect("insert binding");
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([old_service.clone(), new_owner_service.clone()])
            .await
            .expect("insert user services");

        let state = test_app_state(db.clone());
        let Json(response) = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id.clone(),
            }),
        )
        .await
        .expect("transfer succeeds");

        assert_eq!(response.node_id, node.id);
        assert_eq!(response.previous_owner.id, admin_id);
        assert_eq!(response.new_owner.id, org_id);
        assert_eq!(response.deactivated_bindings_count, 1);
        assert_eq!(response.cleared_user_service_count, 1);

        let audit_payload = transfer_audit_event_data(
            &admin_id,
            &node_service::TransferNodeResult {
                node_id: node.id.clone(),
                previous_owner_user_id: admin_id.clone(),
                new_owner_user_id: org_id.clone(),
                deactivated_bindings_count: response.deactivated_bindings_count,
                cleared_user_service_count: response.cleared_user_service_count,
                deactivated_pending_credentials_count: 0,
            },
        );
        assert_eq!(
            audit_payload
                .get("previous_owner_user_id")
                .and_then(|value| value.as_str()),
            Some(admin_id.as_str())
        );
        assert_eq!(
            audit_payload
                .get("new_owner_user_id")
                .and_then(|value| value.as_str()),
            Some(org_id.as_str())
        );
        assert_eq!(
            audit_payload
                .get("deactivated_bindings_count")
                .and_then(|value| value.as_u64()),
            Some(1)
        );
        assert_eq!(
            audit_payload
                .get("cleared_user_service_count")
                .and_then(|value| value.as_u64()),
            Some(1)
        );

        let transferred = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("query node")
            .expect("node exists");
        assert_eq!(transferred.user_id, org_id);

        let updated_binding = db
            .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .find_one(doc! { "_id": &binding.id })
            .await
            .expect("query binding")
            .expect("binding exists");
        assert!(!updated_binding.is_active);

        let old_service_after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &old_service.id })
            .await
            .expect("query old service")
            .expect("old service exists");
        assert_eq!(old_service_after.node_id, None);

        let new_owner_service_after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &new_owner_service.id })
            .await
            .expect("query new owner service")
            .expect("new owner service exists");
        assert_eq!(
            new_owner_service_after.node_id.as_deref(),
            Some(node.id.as_str())
        );
    }

    #[tokio::test]
    async fn transfer_org_node_between_administered_orgs_succeeds() {
        let Some(db) = connect_test_database("node_transfer_org_to_org").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_a_id = Uuid::new_v4().to_string();
        let org_b_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_a_id, UserType::Org),
                test_user(&org_b_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_a_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_b_id, &admin_id, OrgRole::Admin, None),
            ])
            .await
            .expect("insert memberships");
        let node = test_node(&org_a_id, "shared-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());
        let _ = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_b_id.clone(),
            }),
        )
        .await
        .expect("transfer succeeds");

        let transferred = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("query node")
            .expect("node exists");
        assert_eq!(transferred.user_id, org_b_id);
    }

    #[tokio::test]
    async fn transfer_org_node_by_member_returns_not_found() {
        let Some(db) = connect_test_database("node_transfer_member_denied").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .expect("insert membership");
        let node = test_node(&org_id, "member-denied-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&member_id),
            Path(node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: member_id,
            }),
        )
        .await
        .expect_err("member cannot transfer org node");

        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn transfer_to_same_owner_returns_bad_request() {
        let Some(db) = connect_test_database("node_transfer_same_owner").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert owner");
        let node = test_node(&owner_id, "same-owner-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&owner_id),
            Path(node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: owner_id,
            }),
        )
        .await
        .expect_err("same-owner transfer rejected");

        assert!(
            matches!(err, AppError::BadRequest(message) if message == "node already belongs to that owner")
        );
    }

    #[tokio::test]
    async fn transfer_name_collision_returns_explicit_bad_request() {
        let Some(db) = connect_test_database("node_transfer_name_collision").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");
        let source_node = test_node(&admin_id, "duplicate-node");
        let colliding_node = test_node(&org_id, "duplicate-node");
        db.collection::<Node>(NODES)
            .insert_many([source_node.clone(), colliding_node])
            .await
            .expect("insert nodes");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(source_node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id,
            }),
        )
        .await
        .expect_err("name collision rejected");

        assert!(
            matches!(err, AppError::BadRequest(message) if message.contains("An active node named 'duplicate-node' already exists for the destination owner"))
        );
    }

    #[tokio::test]
    async fn transfer_destination_at_node_cap_returns_bad_request() {
        let Some(db) = connect_test_database("node_transfer_cap").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let source_node = test_node(&admin_id, "source-node");
        let mut nodes = vec![source_node.clone()];
        nodes.extend((0..10).map(|idx| test_node(&org_id, &format!("org-node-{idx}"))));
        db.collection::<Node>(NODES)
            .insert_many(nodes)
            .await
            .expect("insert nodes");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(source_node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id,
            }),
        )
        .await
        .expect_err("cap rejected");

        assert!(
            matches!(err, AppError::BadRequest(message) if message == "Maximum of 10 nodes per user reached")
        );
    }

    #[tokio::test]
    async fn transfer_updates_list_visibility_for_previous_and_new_owner_members() {
        let Some(db) = connect_test_database("node_transfer_list_visibility").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_a_member_id = Uuid::new_v4().to_string();
        let org_b_member_id = Uuid::new_v4().to_string();
        let org_a_id = Uuid::new_v4().to_string();
        let org_b_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_a_member_id, UserType::Person),
                test_user(&org_b_member_id, UserType::Person),
                test_user(&org_a_id, UserType::Org),
                test_user(&org_b_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_a_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_b_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_a_id, &org_a_member_id, OrgRole::Member, None),
                test_membership(&org_b_id, &org_b_member_id, OrgRole::Member, None),
            ])
            .await
            .expect("insert memberships");
        let node = test_node(&org_a_id, "moving-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());
        let _ = transfer_node(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_b_id,
            }),
        )
        .await
        .expect("transfer succeeds");

        let Json(previous_response) =
            list_nodes(State(state.clone()), test_auth_user(&org_a_member_id))
                .await
                .expect("previous owner member can list nodes");
        assert!(
            !previous_response
                .nodes
                .iter()
                .any(|item| item.id == node.id)
        );

        let Json(new_response) = list_nodes(State(state), test_auth_user(&org_b_member_id))
            .await
            .expect("new owner member can list nodes");
        assert!(new_response.nodes.iter().any(|item| item.id == node.id));
    }

    #[tokio::test]
    async fn transfer_orders_cleanup_before_ownership_flip() {
        let Some(db) = connect_test_database("node_transfer_cleanup_order").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let other_old_owner_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&actor_id, UserType::Person),
                test_user(&other_old_owner_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let node = test_node(&actor_id, "edge-node");
        let binding = test_binding(&actor_id, &node.id, "svc-old");
        let actor_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &actor_id,
            "actor-service",
            &Uuid::new_v4().to_string(),
            Some("svc-old"),
            Some(&node.id),
        );
        let orphaned_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &other_old_owner_id,
            "orphaned-service",
            &Uuid::new_v4().to_string(),
            Some("svc-orphaned"),
            Some(&node.id),
        );
        let destination_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "destination-service",
            &Uuid::new_v4().to_string(),
            Some("svc-destination"),
            Some(&node.id),
        );
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(binding.clone())
            .await
            .expect("insert binding");
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([actor_service, orphaned_service, destination_service])
            .await
            .expect("insert user services");

        let state = test_app_state(db.clone());
        let Json(response) = transfer_node(
            State(state),
            test_auth_user(&actor_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id.clone(),
            }),
        )
        .await
        .expect("transfer succeeds");

        assert_eq!(response.deactivated_bindings_count, 1);
        let audit_payload = transfer_audit_event_data(
            &actor_id,
            &node_service::TransferNodeResult {
                node_id: node.id.clone(),
                previous_owner_user_id: actor_id.clone(),
                new_owner_user_id: org_id.clone(),
                deactivated_bindings_count: response.deactivated_bindings_count,
                cleared_user_service_count: response.cleared_user_service_count,
                deactivated_pending_credentials_count: 0,
            },
        );
        assert_eq!(
            audit_payload
                .get("actor_user_id")
                .and_then(|value| value.as_str()),
            Some(actor_id.as_str())
        );
        assert_eq!(
            audit_payload
                .get("owner_user_id")
                .and_then(|value| value.as_str()),
            Some(org_id.as_str())
        );
        assert_eq!(
            audit_payload
                .get("deactivated_bindings_count")
                .and_then(|value| value.as_u64()),
            Some(1)
        );

        let cross_owner_routes = db
            .collection::<UserService>(USER_SERVICES)
            .count_documents(doc! {
                "node_id": &node.id,
                "user_id": { "$ne": &org_id },
                "is_active": true,
            })
            .await
            .expect("count cross-owner routes");
        assert_eq!(cross_owner_routes, 0);

        let active_bindings = db
            .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .count_documents(doc! { "node_id": &node.id, "is_active": true })
            .await
            .expect("count active bindings");
        assert_eq!(active_bindings, 0);

        let transferred = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("query node")
            .expect("node exists");
        assert_eq!(transferred.user_id, org_id);
    }

    #[tokio::test]
    async fn list_admins_returns_personal_owner() {
        let Some(db) = connect_test_database("node_admins_personal").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert owner");
        let node = test_node(&owner_id, "personal-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = list_admins(
            State(state),
            test_auth_user(&owner_id),
            Path(node.id.clone()),
        )
        .await
        .expect("list admins");

        assert_eq!(response.admins.len(), 1);
        assert_eq!(response.admins[0].user_id, owner_id);
        assert_eq!(response.admins[0].role, node_service::NodeAdminRole::Owner);
    }

    #[tokio::test]
    async fn list_admins_returns_org_admins_for_readable_org_node() {
        let Some(db) = connect_test_database("node_admins_org").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_a_id = Uuid::new_v4().to_string();
        let admin_b_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&admin_a_id, UserType::Person),
                test_user(&admin_b_id, UserType::Person),
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_id, &admin_a_id, OrgRole::Admin, None),
                test_membership(&org_id, &admin_b_id, OrgRole::Admin, None),
                test_membership(&org_id, &member_id, OrgRole::Member, None),
            ])
            .await
            .expect("insert memberships");
        let node = test_node(&org_id, "org-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = list_admins(
            State(state),
            test_auth_user(&member_id),
            Path(node.id.clone()),
        )
        .await
        .expect("member can list node admins");

        let mut admin_ids: Vec<&str> = response
            .admins
            .iter()
            .map(|admin| admin.user_id.as_str())
            .collect();
        admin_ids.sort_unstable();
        let mut expected = vec![admin_a_id.as_str(), admin_b_id.as_str()];
        expected.sort_unstable();
        assert_eq!(admin_ids, expected);
        assert!(
            response
                .admins
                .iter()
                .all(|admin| admin.role == node_service::NodeAdminRole::Admin)
        );
    }

    #[tokio::test]
    async fn list_nodes_returns_empty_for_user_with_no_nodes() {
        let Some(db) = connect_test_database("node_ext_list_empty").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db);
        let Json(response) = list_nodes(State(state), test_auth_user(&actor_id))
            .await
            .expect("list nodes");

        assert!(response.nodes.is_empty());
    }

    #[tokio::test]
    async fn get_node_returns_not_found_for_nonexistent() {
        let Some(db) = connect_test_database("node_ext_get_notfound").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db);
        let err = get_node(
            State(state),
            test_auth_user(&actor_id),
            Path("nonexistent-node-id".to_string()),
        )
        .await
        .expect_err("should return not found");

        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn delete_node_returns_not_found_for_nonexistent() {
        let Some(db) = connect_test_database("node_ext_del_notfound").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db);
        let result = delete_node(
            State(state),
            test_auth_user(&actor_id),
            crate::telemetry::TelemetryContext::default(),
            Path("nonexistent-node-id".to_string()),
        )
        .await;

        let err = result
            .err()
            .expect("should return error for nonexistent node");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn delete_node_removes_node_and_makes_it_unfindable() {
        let Some(db) = connect_test_database("node_ext_delete_ok").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert user");
        let node = test_node(&owner_id, "deletable-node");
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());
        let result = delete_node(
            State(state.clone()),
            test_auth_user(&owner_id),
            crate::telemetry::TelemetryContext::default(),
            Path(node_id.clone()),
        )
        .await;
        assert!(result.is_ok());

        let err = get_node(State(state), test_auth_user(&owner_id), Path(node_id))
            .await
            .expect_err("node should be gone");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn list_bindings_returns_empty_for_node_with_no_bindings() {
        let Some(db) = connect_test_database("node_ext_bindings_empty").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert user");
        let node = test_node(&owner_id, "empty-bindings-node");
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = list_bindings(State(state), test_auth_user(&owner_id), Path(node_id))
            .await
            .expect("list bindings");

        assert!(response.bindings.is_empty());
    }

    #[tokio::test]
    async fn rotate_token_returns_new_credentials() {
        let Some(db) = connect_test_database("node_ext_rotate_token").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert user");
        let node = test_node(&owner_id, "rotate-node");
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = rotate_token(State(state), test_auth_user(&owner_id), Path(node_id))
            .await
            .expect("rotate token");

        assert!(!response.auth_token.is_empty());
        assert!(!response.signing_secret.is_empty());
        assert!(response.message.contains("rotated"));
    }

    #[test]
    fn audit_event_data_with_owner_adds_owner_only_when_shared() {
        let personal =
            audit_event_data_with_owner("user-1", "user-1", serde_json::json!({ "node_id": "n1" }));
        assert!(personal.get("owner_user_id").is_none());

        let shared =
            audit_event_data_with_owner("user-1", "org-1", serde_json::json!({ "node_id": "n1" }));
        assert_eq!(
            shared.get("owner_user_id").and_then(|v| v.as_str()),
            Some("org-1")
        );
    }

    // --- Pure function tests: build_metrics_info ---

    #[test]
    fn build_metrics_info_zero_requests_yields_zero_success_rate() {
        let metrics = NodeMetrics::default();
        let info = build_metrics_info(&metrics);

        assert_eq!(info.total_requests, 0);
        assert_eq!(info.success_count, 0);
        assert_eq!(info.error_count, 0);
        assert!((info.success_rate - 0.0).abs() < f64::EPSILON);
        assert!((info.avg_latency_ms - 0.0).abs() < f64::EPSILON);
        assert!(info.last_error.is_none());
        assert!(info.last_error_at.is_none());
        assert!(info.last_success_at.is_none());
    }

    #[test]
    fn build_metrics_info_computes_success_rate_correctly() {
        let metrics = NodeMetrics {
            total_requests: 200,
            success_count: 150,
            error_count: 50,
            avg_latency_ms: 42.5,
            last_error: Some("timeout".to_string()),
            last_error_at: Some(Utc::now()),
            last_success_at: Some(Utc::now()),
        };
        let info = build_metrics_info(&metrics);

        assert_eq!(info.total_requests, 200);
        assert_eq!(info.success_count, 150);
        assert_eq!(info.error_count, 50);
        assert!((info.success_rate - 0.75).abs() < f64::EPSILON);
        assert!((info.avg_latency_ms - 42.5).abs() < f64::EPSILON);
        assert_eq!(info.last_error.as_deref(), Some("timeout"));
        assert!(info.last_error_at.is_some());
        assert!(info.last_success_at.is_some());
    }

    #[test]
    fn build_metrics_info_all_successes_yields_one() {
        let metrics = NodeMetrics {
            total_requests: 1000,
            success_count: 1000,
            error_count: 0,
            avg_latency_ms: 10.0,
            last_error: None,
            last_error_at: None,
            last_success_at: Some(Utc::now()),
        };
        let info = build_metrics_info(&metrics);
        assert!((info.success_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn build_metrics_info_all_errors_yields_zero_rate() {
        let metrics = NodeMetrics {
            total_requests: 50,
            success_count: 0,
            error_count: 50,
            avg_latency_ms: 500.0,
            last_error: Some("connection refused".to_string()),
            last_error_at: Some(Utc::now()),
            last_success_at: None,
        };
        let info = build_metrics_info(&metrics);
        assert!((info.success_rate - 0.0).abs() < f64::EPSILON);
        assert!(info.last_success_at.is_none());
    }

    // --- Serialization tests: NodeMetricsInfo ---

    #[test]
    fn node_metrics_info_serialization_skips_none_fields() {
        let info = NodeMetricsInfo {
            total_requests: 10,
            success_count: 8,
            error_count: 2,
            success_rate: 0.8,
            avg_latency_ms: 25.0,
            last_error: None,
            last_error_at: None,
            last_success_at: None,
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["total_requests"], 10);
        assert_eq!(json["success_count"], 8);
        assert_eq!(json["error_count"], 2);
        assert!((json["success_rate"].as_f64().unwrap() - 0.8).abs() < f64::EPSILON);
        assert!((json["avg_latency_ms"].as_f64().unwrap() - 25.0).abs() < f64::EPSILON);
        // skip_serializing_if fields should be absent
        assert!(json.get("last_error").is_none());
        assert!(json.get("last_error_at").is_none());
        assert!(json.get("last_success_at").is_none());
    }

    #[test]
    fn node_metrics_info_serialization_includes_present_fields() {
        let info = NodeMetricsInfo {
            total_requests: 5,
            success_count: 3,
            error_count: 2,
            success_rate: 0.6,
            avg_latency_ms: 100.0,
            last_error: Some("bad gateway".to_string()),
            last_error_at: Some("2025-01-15T10:00:00+00:00".to_string()),
            last_success_at: Some("2025-01-15T09:00:00+00:00".to_string()),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["last_error"], "bad gateway");
        assert_eq!(json["last_error_at"], "2025-01-15T10:00:00+00:00");
        assert_eq!(json["last_success_at"], "2025-01-15T09:00:00+00:00");
    }

    // --- Serialization tests: NodeInfo ---

    #[test]
    fn node_info_serialization_skips_none_optional_fields() {
        let info = NodeInfo {
            id: "node-1".to_string(),
            name: "test-node".to_string(),
            owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::User,
                id: "user-1".to_string(),
                display_name: "Test User".to_string(),
            },
            status: "online".to_string(),
            is_connected: true,
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: None,
            binding_count: 3,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["id"], "node-1");
        assert_eq!(json["name"], "test-node");
        assert_eq!(json["status"], "online");
        assert_eq!(json["is_connected"], true);
        assert_eq!(json["binding_count"], 3);
        assert_eq!(json["created_at"], "2025-01-01T00:00:00+00:00");
        assert_eq!(json["owner"]["kind"], "user");
        assert_eq!(json["owner"]["id"], "user-1");
        assert_eq!(json["owner"]["display_name"], "Test User");
        // Optional fields absent
        assert!(json.get("last_heartbeat_at").is_none());
        assert!(json.get("connected_at").is_none());
        assert!(json.get("metadata").is_none());
        assert!(json.get("metrics").is_none());
    }

    #[test]
    fn node_info_serialization_includes_all_fields_when_present() {
        let info = NodeInfo {
            id: "node-2".to_string(),
            name: "prod-node".to_string(),
            owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::Org,
                id: "org-1".to_string(),
                display_name: "Acme Corp".to_string(),
            },
            status: "draining".to_string(),
            is_connected: false,
            last_heartbeat_at: Some("2025-06-01T12:00:00+00:00".to_string()),
            connected_at: Some("2025-06-01T10:00:00+00:00".to_string()),
            metadata: Some(NodeMetadata {
                agent_version: Some("1.2.3".to_string()),
                os: Some("linux".to_string()),
                arch: Some("x86_64".to_string()),
                ip_address: Some("10.0.0.1".to_string()),
                provisioning_source: None,
            }),
            metrics: Some(NodeMetricsInfo {
                total_requests: 100,
                success_count: 90,
                error_count: 10,
                success_rate: 0.9,
                avg_latency_ms: 50.0,
                last_error: None,
                last_error_at: None,
                last_success_at: None,
            }),
            binding_count: 5,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["owner"]["kind"], "org");
        assert_eq!(json["last_heartbeat_at"], "2025-06-01T12:00:00+00:00");
        assert_eq!(json["connected_at"], "2025-06-01T10:00:00+00:00");
        assert_eq!(json["metadata"]["agent_version"], "1.2.3");
        assert_eq!(json["metadata"]["os"], "linux");
        assert_eq!(json["metadata"]["arch"], "x86_64");
        assert_eq!(json["metadata"]["ip_address"], "10.0.0.1");
        assert_eq!(json["metrics"]["total_requests"], 100);
    }

    // --- Serialization tests: BindingInfo ---

    #[test]
    fn binding_info_serialization() {
        let info = BindingInfo {
            id: "binding-1".to_string(),
            service_id: "svc-1".to_string(),
            service_name: "OpenAI".to_string(),
            service_slug: "openai".to_string(),
            is_active: true,
            priority: 10,
            created_at: "2025-03-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["id"], "binding-1");
        assert_eq!(json["service_id"], "svc-1");
        assert_eq!(json["service_name"], "OpenAI");
        assert_eq!(json["service_slug"], "openai");
        assert_eq!(json["is_active"], true);
        assert_eq!(json["priority"], 10);
        assert_eq!(json["created_at"], "2025-03-01T00:00:00+00:00");
    }

    // --- Serialization tests: CreateRegistrationTokenResponse ---

    #[test]
    fn create_registration_token_response_serialization() {
        let resp = CreateRegistrationTokenResponse {
            token_id: "tid-1".to_string(),
            token: "nyx_nreg_abc123".to_string(),
            name: "my-node".to_string(),
            expires_at: "2025-06-01T12:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["token_id"], "tid-1");
        assert_eq!(json["token"], "nyx_nreg_abc123");
        assert_eq!(json["name"], "my-node");
        assert_eq!(json["expires_at"], "2025-06-01T12:00:00+00:00");
    }

    // --- Serialization tests: RotateTokenResponse ---

    #[test]
    fn rotate_token_response_serialization() {
        let resp = RotateTokenResponse {
            auth_token: "new-token".to_string(),
            signing_secret: "new-secret".to_string(),
            message: "Rotated".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["auth_token"], "new-token");
        assert_eq!(json["signing_secret"], "new-secret");
        assert_eq!(json["message"], "Rotated");
    }

    // --- Serialization tests: TransferNodeResponse ---

    #[test]
    fn transfer_node_response_serialization() {
        let resp = TransferNodeResponse {
            node_id: "node-1".to_string(),
            previous_owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::User,
                id: "user-1".to_string(),
                display_name: "Alice".to_string(),
            },
            new_owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::Org,
                id: "org-1".to_string(),
                display_name: "Acme".to_string(),
            },
            deactivated_bindings_count: 2,
            cleared_user_service_count: 1,
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["node_id"], "node-1");
        assert_eq!(json["previous_owner"]["kind"], "user");
        assert_eq!(json["previous_owner"]["id"], "user-1");
        assert_eq!(json["new_owner"]["kind"], "org");
        assert_eq!(json["new_owner"]["id"], "org-1");
        assert_eq!(json["deactivated_bindings_count"], 2);
        assert_eq!(json["cleared_user_service_count"], 1);
    }

    #[test]
    fn pending_ciphertext_request_validates_v1_base64url_and_size() {
        let request = PendingCredentialCiphertextRequest {
            version: "v1".to_string(),
            admin_pubkey: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1_u8; 32]),
            nonce: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2_u8; 24]),
            ciphertext: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3_u8; 48]),
            integrity_verification: None,
        };

        let decoded = validate_pending_ciphertext_request(&request).expect("valid request");
        assert_eq!(decoded, PendingCiphertextValidation::Valid(vec![3_u8; 48]));
    }

    #[test]
    fn pending_ciphertext_request_rejects_bad_version_padding_lengths_and_oversize() {
        let valid_admin = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1_u8; 32]);
        let valid_nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2_u8; 24]);
        let valid_ciphertext = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3_u8; 48]);

        let bad_version = PendingCredentialCiphertextRequest {
            version: "v0".to_string(),
            admin_pubkey: valid_admin.clone(),
            nonce: valid_nonce.clone(),
            ciphertext: valid_ciphertext.clone(),
            integrity_verification: None,
        };
        assert!(matches!(
            validate_pending_ciphertext_request(&bad_version),
            Err(AppError::PendingCredentialVersionUnsupported(version)) if version == "v0"
        ));

        let padded_admin = PendingCredentialCiphertextRequest {
            version: "v1".to_string(),
            admin_pubkey: format!("{valid_admin}="),
            nonce: valid_nonce.clone(),
            ciphertext: valid_ciphertext.clone(),
            integrity_verification: None,
        };
        assert!(matches!(
            validate_pending_ciphertext_request(&padded_admin),
            Err(AppError::ValidationError(message)) if message.contains("without padding")
        ));

        let short_nonce = PendingCredentialCiphertextRequest {
            version: "v1".to_string(),
            admin_pubkey: valid_admin,
            nonce: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2_u8; 23]),
            ciphertext: valid_ciphertext,
            integrity_verification: None,
        };
        assert!(matches!(
            validate_pending_ciphertext_request(&short_nonce),
            Err(AppError::ValidationError(message)) if message.contains("24 bytes")
        ));

        let oversized = PendingCredentialCiphertextRequest {
            version: "v1".to_string(),
            admin_pubkey: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1_u8; 32]),
            nonce: valid_nonce,
            ciphertext: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![
                9_u8;
                node_pending_credential_service::MAX_CIPHERTEXT_SIZE
                    + 1
            ]),
            integrity_verification: None,
        };
        assert!(matches!(
            validate_pending_ciphertext_request(&oversized),
            Ok(PendingCiphertextValidation::TooLarge)
        ));
    }

    #[test]
    fn pending_pubkey_response_omits_admin_ciphertext_material() {
        let now = Utc::now();
        let pending = NodePendingCredential {
            id: "pending-1".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openclaw".to_string(),
            injection_method: crate::models::node_pending_credential::InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(5),
            consumed_at: None,
            declined_at: None,
            crypto: Some(crate::models::node_pending_credential::CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: "node-pubkey".to_string(),
                admin_pubkey: Some("admin-pubkey".to_string()),
                nonce: Some("nonce".to_string()),
                ciphertext: Some(vec![1, 2, 3]),
            }),
            remote_state: Some(RemoteCryptoState::PubkeyPosted),
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };

        let response = pending_pubkey_response(pending, false).expect("pubkey response");
        let json = serde_json::to_value(&response).expect("serialize");

        assert_eq!(json["pending_id"], "pending-1");
        assert_eq!(json["node_pubkey"], "node-pubkey");
        assert_eq!(json["remote_state"], "pubkey_posted");
        assert!(json.get("admin_pubkey").is_none());
        assert!(json.get("nonce").is_none());
        assert!(json.get("ciphertext").is_none());
    }

    #[test]
    fn pending_pubkey_response_returns_awaiting_until_pubkey_exists() {
        let now = Utc::now();
        let pending = NodePendingCredential {
            id: "pending-awaiting".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openclaw".to_string(),
            injection_method: crate::models::node_pending_credential::InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(5),
            consumed_at: None,
            declined_at: None,
            crypto: Some(crate::models::node_pending_credential::CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: String::new(),
                admin_pubkey: None,
                nonce: None,
                ciphertext: None,
            }),
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };

        assert!(matches!(
            pending_pubkey_response(pending, false),
            Err(AppError::PendingCredentialPubkeyAwaiting(id)) if id == "pending-awaiting"
        ));
    }

    // --- Serialization tests: PendingCredentialInfo ---

    #[test]
    fn pending_credential_info_serialization_skips_none_fields() {
        let info = PendingCredentialInfo {
            id: "pc-1".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: "header".to_string(),
            field_name: "Authorization".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            expires_at: "2025-01-01T01:00:00+00:00".to_string(),
            consumed_at: None,
            declined_at: None,
            remote_state: None,
            is_active: true,
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["id"], "pc-1");
        assert_eq!(json["injection_method"], "header");
        assert_eq!(json["field_name"], "Authorization");
        assert_eq!(json["is_active"], true);
        // skip_serializing_if fields absent
        assert!(json.get("target_url").is_none());
        assert!(json.get("label").is_none());
        assert!(json.get("consumed_at").is_none());
        assert!(json.get("declined_at").is_none());
    }

    #[test]
    fn pending_credential_info_serialization_includes_optional_fields() {
        let info = PendingCredentialInfo {
            id: "pc-2".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "anthropic".to_string(),
            injection_method: "query-param".to_string(),
            field_name: "api_key".to_string(),
            target_url: Some("https://api.anthropic.com".to_string()),
            label: Some("Production".to_string()),
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "org-1".to_string(),
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            expires_at: "2025-01-01T01:00:00+00:00".to_string(),
            consumed_at: Some("2025-01-01T00:30:00+00:00".to_string()),
            declined_at: None,
            remote_state: Some("consumed".to_string()),
            is_active: false,
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["target_url"], "https://api.anthropic.com");
        assert_eq!(json["label"], "Production");
        assert_eq!(json["consumed_at"], "2025-01-01T00:30:00+00:00");
        assert_eq!(json["remote_state"], "consumed");
        assert_eq!(json["is_active"], false);
        assert!(json.get("declined_at").is_none());
    }

    // --- Pure function tests: pending_credential_info mapping ---

    #[test]
    fn pending_credential_info_maps_model_fields_correctly() {
        let now = Utc::now();
        let expires = now + chrono::Duration::hours(1);
        let model = crate::models::node_pending_credential::NodePendingCredential {
            id: "pc-map-1".to_string(),
            node_id: "node-map-1".to_string(),
            service_slug: "github".to_string(),
            injection_method: crate::models::node_pending_credential::InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.github.com".to_string()),
            label: Some("GH Token".to_string()),
            created_by_user_id: "creator-1".to_string(),
            owner_user_id: "owner-1".to_string(),
            created_at: now,
            expires_at: expires,
            consumed_at: None,
            declined_at: Some(now),
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: false,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };
        let info = pending_credential_info(model.clone());

        assert_eq!(info.id, "pc-map-1");
        assert_eq!(info.node_id, "node-map-1");
        assert_eq!(info.service_slug, "github");
        assert_eq!(info.injection_method, "header");
        assert_eq!(info.field_name, "Authorization");
        assert_eq!(info.target_url.as_deref(), Some("https://api.github.com"));
        assert_eq!(info.label.as_deref(), Some("GH Token"));
        assert_eq!(info.created_by_user_id, "creator-1");
        assert_eq!(info.owner_user_id, "owner-1");
        assert_eq!(info.created_at, now.to_rfc3339());
        assert_eq!(info.expires_at, expires.to_rfc3339());
        assert!(info.consumed_at.is_none());
        assert!(info.declined_at.is_some());
        assert!(!info.is_active);
    }

    #[test]
    fn pending_credential_info_includes_remote_state_metadata_only() {
        let now = Utc::now();
        let model = crate::models::node_pending_credential::NodePendingCredential {
            id: "pc-map-state".to_string(),
            node_id: "node-map-state".to_string(),
            service_slug: "openclaw".to_string(),
            injection_method: crate::models::node_pending_credential::InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "creator-1".to_string(),
            owner_user_id: "owner-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: Some(crate::models::node_pending_credential::CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: "raw-node-pubkey".to_string(),
                admin_pubkey: Some("raw-admin-pubkey".to_string()),
                nonce: Some("raw-nonce".to_string()),
                ciphertext: Some(vec![1, 2, 3]),
            }),
            remote_state: Some(RemoteCryptoState::PubkeyAwaiting),
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };

        let json = serde_json::to_value(pending_credential_info(model)).unwrap();

        assert_eq!(json["remote_state"], "pubkey_awaiting");
        let body = json.to_string();
        assert!(!body.contains("raw-node-pubkey"));
        assert!(!body.contains("raw-admin-pubkey"));
        assert!(!body.contains("raw-nonce"));
    }

    // --- Serialization tests: MyBoundServicesResponse ---

    #[test]
    fn my_bound_services_response_serialization() {
        let resp = MyBoundServicesResponse {
            service_ids: vec!["svc-1".to_string(), "svc-2".to_string()],
        };
        let json = serde_json::to_value(&resp).unwrap();
        let ids = json["service_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "svc-1");
        assert_eq!(ids[1], "svc-2");
    }

    #[test]
    fn my_bound_services_response_empty_serialization() {
        let resp = MyBoundServicesResponse {
            service_ids: vec![],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["service_ids"].as_array().unwrap().is_empty());
    }

    // --- Serialization tests: CreateBindingResponse ---

    #[test]
    fn create_binding_response_serialization() {
        let resp = CreateBindingResponse {
            id: "bind-1".to_string(),
            service_id: "svc-1".to_string(),
            service_name: "Anthropic".to_string(),
            message: "Service binding created".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["id"], "bind-1");
        assert_eq!(json["service_id"], "svc-1");
        assert_eq!(json["service_name"], "Anthropic");
        assert_eq!(json["message"], "Service binding created");
    }

    // --- Serialization tests: NodeListResponse ---

    #[test]
    fn node_list_response_serialization_empty() {
        let resp = NodeListResponse { nodes: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["nodes"].as_array().unwrap().is_empty());
    }

    // --- Serialization tests: BindingListResponse ---

    #[test]
    fn binding_list_response_serialization_empty() {
        let resp = BindingListResponse { bindings: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["bindings"].as_array().unwrap().is_empty());
    }

    // --- Serialization tests: NodeAdminsResponse ---

    #[test]
    fn node_admins_response_serialization_empty() {
        let resp = NodeAdminsResponse { admins: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["admins"].as_array().unwrap().is_empty());
    }

    // --- Pure function tests: audit_event_data_with_owner edge cases ---

    #[test]
    fn audit_event_data_with_owner_preserves_existing_fields() {
        let result = audit_event_data_with_owner(
            "actor-1",
            "org-1",
            serde_json::json!({ "node_id": "n1", "extra": true }),
        );
        assert_eq!(result.get("node_id").and_then(|v| v.as_str()), Some("n1"));
        assert_eq!(result.get("extra").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            result.get("owner_user_id").and_then(|v| v.as_str()),
            Some("org-1")
        );
    }

    #[test]
    fn audit_event_data_with_owner_non_object_value_unchanged() {
        let result = audit_event_data_with_owner("actor-1", "org-1", serde_json::json!("scalar"));
        // Non-object values should be returned unchanged (no owner_user_id insertion possible)
        assert_eq!(result, serde_json::json!("scalar"));
    }
}
