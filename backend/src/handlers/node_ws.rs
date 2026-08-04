use axum::{
    extract::ConnectInfo,
    extract::State,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::node::{NodeMetadata, NodeStatus};
use crate::models::node_pending_credential::NodePendingCredential;
use crate::services::{
    audit_service, node_pending_credential_service, node_service,
    node_ws_manager::{
        NodeCapabilitiesMsg, NodeOutboundMessage, NodeProxyResponse, NodeSshExecResult,
        NodeWsManager, PendingCredentialCiphertextParams, WsFrameInjectedInbound,
        WsProxyBinaryInbound, WsProxyClosedInbound, WsProxyErrorInbound, WsProxyOpenedInbound,
        WsProxyResponseChunkMsg, WsProxyResponseEndMsg, WsProxyResponseStartMsg,
        WsProxyTextInbound, WsSshExecResultMsg, WsSshNodeExecCloseMsg, WsSshNodeExecDataMsg,
        WsSshNodeExecErrorMsg, WsSshTunnelClosedMsg, WsSshTunnelDataMsg, WsSshTunnelOpenedMsg,
        WsWebTerminalClosedMsg, WsWebTerminalDataMsg, WsWebTerminalStartedMsg,
    },
    rci_audit_service::{
        self, RciAuditDelivery, RciAuditErrorKind, RciAuditEventKind, RciAuditSubject,
    },
};
use crate::telemetry::{
    context::{TelemetryContext, emit_event},
    sampling::hash_short_id,
    schema::TelemetryEvent,
};

/// RAII guard that decrements the pending auth counter on drop.
/// Prevents counter leaks if the WS handler future is cancelled (H3).
struct PendingAuthGuard {
    manager: Arc<NodeWsManager>,
}

impl Drop for PendingAuthGuard {
    fn drop(&mut self) {
        self.manager.decrement_pending_auth();
    }
}

/// Size of the bounded channel for WS writer messages (H4).
const WS_WRITER_CHANNEL_SIZE: usize = 256;

/// JSON messages from node -> NyxID (first message must be register or auth).
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum NodeMessage {
    #[serde(rename = "register")]
    Register {
        token: String,
        #[serde(default)]
        metadata: Option<NodeMetadata>,
    },
    #[serde(rename = "auth")]
    Auth { node_id: String, token: String },
    #[serde(rename = "heartbeat_pong")]
    HeartbeatPong {
        #[allow(dead_code)]
        timestamp: Option<String>,
    },
    #[serde(rename = "proxy_response")]
    ProxyResponse(crate::services::node_ws_manager::WsProxyResponseMsg),
    #[serde(rename = "proxy_error")]
    ProxyError(crate::services::node_ws_manager::WsProxyErrorMsg),
    #[serde(rename = "proxy_response_start")]
    ProxyResponseStart(WsProxyResponseStartMsg),
    #[serde(rename = "proxy_response_chunk")]
    ProxyResponseChunk(WsProxyResponseChunkMsg),
    #[serde(rename = "proxy_response_end")]
    ProxyResponseEnd(WsProxyResponseEndMsg),
    #[serde(rename = "status_update")]
    StatusUpdate {
        #[allow(dead_code)]
        agent_version: Option<String>,
        #[allow(dead_code)]
        services_ready: Option<Vec<String>>,
        /// Capabilities this node agent supports. When present and true,
        /// the backend enables features that require the node to
        /// cooperate with a new protocol contract. Old agents omit the
        /// field and default to `None` → backend treats the capability
        /// as unsupported and falls back to the legacy behavior
        /// (twenty-seventh-round Codex P2: capability negotiation so
        /// backend + node don't need a lockstep upgrade).
        #[serde(default)]
        capabilities: Option<NodeCapabilitiesMsg>,
    },
    #[serde(rename = "ssh_tunnel_opened")]
    SshTunnelOpened(WsSshTunnelOpenedMsg),
    #[serde(rename = "ssh_tunnel_data")]
    SshTunnelData(WsSshTunnelDataMsg),
    #[serde(rename = "ssh_tunnel_closed")]
    SshTunnelClosed(WsSshTunnelClosedMsg),
    #[serde(rename = "web_terminal_started")]
    WebTerminalStarted(WsWebTerminalStartedMsg),
    #[serde(rename = "web_terminal_data")]
    WebTerminalData(WsWebTerminalDataMsg),
    #[serde(rename = "web_terminal_closed")]
    WebTerminalClosed(WsWebTerminalClosedMsg),
    #[serde(rename = "ssh_exec_result")]
    SshExecResult(WsSshExecResultMsg),
    #[serde(rename = "ssh_node_exec_data")]
    SshNodeExecData(WsSshNodeExecDataMsg),
    #[serde(rename = "ssh_node_exec_close")]
    SshNodeExecClose(WsSshNodeExecCloseMsg),
    #[serde(rename = "ssh_node_exec_error")]
    SshNodeExecError(WsSshNodeExecErrorMsg),
    // Placed before CredentialUpdateAck for serde ordering stability. Actual
    // capability type is shared with `node_ws_manager`.
    #[serde(rename = "credential_update_ack")]
    CredentialUpdateAck {
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        service_slug: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    #[serde(rename = "pending_credential_pubkey")]
    PendingCredentialPubkey {
        pending_id: String,
        version: String,
        node_pubkey: String,
    },
    #[serde(rename = "pending_credential_decrypt_result")]
    PendingCredentialDecryptResult {
        pending_id: String,
        status: String,
        #[serde(default)]
        error_code: Option<serde_json::Value>,
    },
    #[serde(rename = "ws_proxy_opened")]
    WsProxyOpened(WsProxyOpenedInbound),
    #[serde(rename = "ws_proxy_text")]
    WsProxyText(WsProxyTextInbound),
    #[serde(rename = "ws_proxy_binary")]
    WsProxyBinary(WsProxyBinaryInbound),
    #[serde(rename = "ws_proxy_closed")]
    WsProxyClosed(WsProxyClosedInbound),
    #[serde(rename = "ws_proxy_error")]
    WsProxyError(WsProxyErrorInbound),
    #[serde(rename = "ws_frame_injected")]
    WsFrameInjected(WsFrameInjectedInbound),
}

fn decode_base64_payload(
    payload: Option<&str>,
    message_type: &str,
    request_id: &str,
) -> Option<Vec<u8>> {
    let Some(payload) = payload else {
        return Some(Vec::new());
    };

    use base64::Engine;
    match base64::engine::general_purpose::STANDARD.decode(payload) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::warn!(
                msg_type = message_type,
                request_id = request_id,
                error = %error,
                "Dropping invalid base64 payload from node"
            );
            None
        }
    }
}

fn handle_proxy_response_chunk(
    ws_manager: &NodeWsManager,
    node_id: &str,
    chunk: WsProxyResponseChunkMsg,
) {
    if let Some(data) = decode_base64_payload(
        chunk.data.as_deref(),
        "proxy_response_chunk",
        &chunk.request_id,
    ) {
        ws_manager.deliver_stream_chunk(node_id, &chunk.request_id, data);
    } else {
        ws_manager.deliver_proxy_error(
            node_id,
            &chunk.request_id,
            "invalid_base64_payload",
            502,
            false,
            None,
        );
    }
}

fn decode_binary_stream_frame(data: &[u8]) -> Result<(&str, &[u8]), &'static str> {
    const REQUEST_ID_LEN: usize = 36;

    if data.len() < REQUEST_ID_LEN {
        return Err("binary frame too short for request_id prefix");
    }

    let request_id = std::str::from_utf8(&data[..REQUEST_ID_LEN])
        .map_err(|_| "binary frame has invalid UTF-8 request_id prefix")?;

    Ok((request_id, &data[REQUEST_ID_LEN..]))
}

fn validate_base64url_no_pad_exact(
    value: &str,
    field: &str,
    expected_len: usize,
) -> Result<(), String> {
    if value.contains('=') {
        return Err(format!("{field} must be base64url without padding"));
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .map_err(|_| format!("{field} must be valid base64url"))?;
    if decoded.len() != expected_len {
        return Err(format!("{field} must decode to {expected_len} bytes"));
    }
    Ok(())
}

fn send_pending_ciphertext_to_node(
    ws_manager: &NodeWsManager,
    node_id: &str,
    pending: &NodePendingCredential,
) -> AppResult<()> {
    let crypto = match pending.crypto.as_ref() {
        Some(crypto) => crypto,
        None => node_pending_credential_service::fan_out_target(pending, node_id)
            .map(|target| &target.crypto)
            .ok_or_else(|| {
                AppError::Internal(
                    "pending credential ciphertext missing crypto bundle".to_string(),
                )
            })?,
    };
    let admin_pubkey = crypto.admin_pubkey.as_deref().ok_or_else(|| {
        AppError::Internal("pending credential ciphertext missing admin_pubkey".to_string())
    })?;
    let nonce = crypto.nonce.as_deref().ok_or_else(|| {
        AppError::Internal("pending credential ciphertext missing nonce".to_string())
    })?;
    let ciphertext = crypto.ciphertext.as_ref().ok_or_else(|| {
        AppError::Internal("pending credential ciphertext missing ciphertext".to_string())
    })?;
    if ciphertext.len() > node_pending_credential_service::MAX_CIPHERTEXT_SIZE {
        return Err(AppError::PendingCredentialCiphertextTooLarge(
            ciphertext.len(),
        ));
    }
    let ciphertext_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ciphertext.as_slice());
    let params = PendingCredentialCiphertextParams {
        pending_id: &pending.id,
        version: &crypto.version,
        admin_pubkey,
        nonce,
        ciphertext: &ciphertext_b64,
    };
    ws_manager.send_pending_credential_ciphertext(node_id, &params)
}

fn log_rci_for_node_pending(
    db: mongodb::Database,
    ip_address: Option<String>,
    user_agent: Option<String>,
    pending: &NodePendingCredential,
    kind: RciAuditEventKind,
) {
    let subject = RciAuditSubject::from_pending(pending);
    rci_audit_service::log_rci_for_node(
        db,
        &pending.owner_user_id,
        ip_address,
        user_agent,
        &subject,
        kind,
    );
}

fn log_rci_for_node_fan_out_target(
    db: mongodb::Database,
    ip_address: Option<String>,
    user_agent: Option<String>,
    pending: &NodePendingCredential,
    node_id: &str,
    kind: RciAuditEventKind,
) {
    if let Some(target) = node_pending_credential_service::fan_out_target(pending, node_id) {
        let subject = RciAuditSubject::from_fan_out_target(pending, target);
        rci_audit_service::log_rci_for_node(
            db,
            &pending.owner_user_id,
            ip_address,
            user_agent,
            &subject,
            kind,
        );
    }
}

fn log_rci_for_node_summary(
    db: mongodb::Database,
    ip_address: Option<String>,
    user_agent: Option<String>,
    summary: &node_pending_credential_service::PendingCredentialAuditSummary,
    kind: RciAuditEventKind,
) {
    let subject = RciAuditSubject::from_summary(summary);
    rci_audit_service::log_rci_for_node(
        db,
        &summary.owner_user_id,
        ip_address,
        user_agent,
        &subject,
        kind,
    );
}

async fn drain_queued_pending_ciphertexts(state: &AppState, node_id: &str) {
    const DRAIN_LIMIT: i64 = 50;
    let now = chrono::Utc::now();
    match node_pending_credential_service::expire_queued_ciphertexts_with_summaries(&state.db, now)
        .await
    {
        Ok(summaries) => {
            for summary in summaries {
                log_rci_for_node_summary(
                    state.db.clone(),
                    None,
                    None,
                    &summary,
                    RciAuditEventKind::Expired,
                );
            }
        }
        Err(err) => {
            tracing::warn!(
                node_id = %node_id,
                error = %err,
                "Failed to expire queued pending credential ciphertexts"
            );
        }
    }
    let pending =
        match node_pending_credential_service::list_deliverable_queued_ciphertexts_for_node(
            &state.db,
            node_id,
            DRAIN_LIMIT,
            now,
        )
        .await
        {
            Ok(pending) => pending,
            Err(err) => {
                tracing::warn!(
                    node_id = %node_id,
                    error = %err,
                    "Failed to list queued pending credential ciphertexts"
                );
                return;
            }
        };

    for pending in pending {
        if let Err(err) =
            send_pending_ciphertext_to_node(state.node_ws_manager.as_ref(), node_id, &pending)
        {
            if matches!(err, AppError::PendingCredentialCiphertextTooLarge(_)) {
                match node_pending_credential_service::mark_queued_ciphertext_too_large_after_replay(
                    &state.db,
                    node_id,
                    &pending.id,
                    chrono::Utc::now(),
                )
                .await
                {
                    Ok(updated) => {
                        if updated.fan_out_nodes.is_empty() {
                            log_rci_for_node_pending(
                                state.db.clone(),
                                None,
                                None,
                                &updated,
                                RciAuditEventKind::CiphertextTooLarge,
                            );
                        } else {
                            log_rci_for_node_fan_out_target(
                                state.db.clone(),
                                None,
                                None,
                                &updated,
                                node_id,
                                RciAuditEventKind::CiphertextTooLarge,
                            );
                        }
                    }
                    Err(mark_err) => {
                        tracing::warn!(
                            node_id = %node_id,
                            pending_id = %pending.id,
                            error = %mark_err,
                            "Failed to mark oversized queued pending credential ciphertext"
                        );
                    }
                }
                continue;
            }
            tracing::warn!(
                node_id = %node_id,
                pending_id = %pending.id,
                error = %err,
                "Stopping pending credential ciphertext drain after send failure"
            );
            break;
        }
        if let Err(err) = node_pending_credential_service::mark_queued_ciphertext_sent(
            &state.db,
            node_id,
            &pending.id,
            chrono::Utc::now(),
        )
        .await
        {
            tracing::warn!(
                node_id = %node_id,
                pending_id = %pending.id,
                error = %err,
                "Failed to mark queued pending credential ciphertext sent"
            );
        } else {
            if pending.fan_out_nodes.is_empty() {
                log_rci_for_node_pending(
                    state.db.clone(),
                    None,
                    None,
                    &pending,
                    RciAuditEventKind::CiphertextReplayed {
                        delivery: RciAuditDelivery::QueuedReplay,
                    },
                );
            } else {
                log_rci_for_node_fan_out_target(
                    state.db.clone(),
                    None,
                    None,
                    &pending,
                    node_id,
                    RciAuditEventKind::CiphertextReplayed {
                        delivery: RciAuditDelivery::QueuedReplay,
                    },
                );
            }
        }
    }
}

async fn apply_status_update_capabilities(
    state: &AppState,
    node_id: &str,
    capabilities: Option<NodeCapabilitiesMsg>,
) {
    if let Some(caps) = capabilities {
        state.node_ws_manager.record_capabilities(node_id, &caps);
    }
    state.node_ws_manager.mark_status_update_received(node_id);
    if state
        .node_ws_manager
        .supports_remote_credential_crypto(node_id)
    {
        drain_queued_pending_ciphertexts(state, node_id).await;
    }
}

async fn record_pending_credential_pubkey_frame(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: String,
    version: String,
    node_pubkey: String,
) -> Option<NodePendingCredential> {
    if version != "v1" {
        tracing::warn!(
            node_id = %node_id,
            pending_id = %pending_id,
            "Ignoring pending credential pubkey with unsupported version"
        );
        return None;
    }
    if validate_base64url_no_pad_exact(&node_pubkey, "node_pubkey", 32).is_err() {
        tracing::warn!(
            node_id = %node_id,
            pending_id = %pending_id,
            error_kind = "invalid_pending_credential_pubkey",
            "Ignoring invalid pending credential pubkey"
        );
        return None;
    }
    match node_pending_credential_service::record_pending_credential_pubkey(
        db,
        node_id,
        &pending_id,
        &version,
        &node_pubkey,
    )
    .await
    {
        Ok(pending) => Some(pending),
        Err(err) => {
            if matches!(err, AppError::NotFound(_)) {
                match node_pending_credential_service::record_fan_out_pubkey(
                    db,
                    node_id,
                    &pending_id,
                    &version,
                    &node_pubkey,
                )
                .await
                {
                    Ok(pending) => Some(pending),
                    Err(err) => {
                        tracing::warn!(
                            node_id = %node_id,
                            pending_id = %pending_id,
                            error = %err,
                            "Failed to record fan-out pending credential pubkey"
                        );
                        None
                    }
                }
            } else {
                tracing::warn!(
                    node_id = %node_id,
                    pending_id = %pending_id,
                    error = %err,
                    "Failed to record pending credential pubkey"
                );
                None
            }
        }
    }
}

async fn handle_pending_credential_pubkey_message(
    db: &mongodb::Database,
    node_id: &str,
    ip_address: Option<String>,
    user_agent: Option<String>,
    pending_id: String,
    version: String,
    node_pubkey: String,
) {
    if version != "v1"
        && let Ok(summary) =
            node_pending_credential_service::get_pending_credential_audit_summary_for_node(
                db,
                node_id,
                &pending_id,
            )
            .await
    {
        log_rci_for_node_summary(
            db.clone(),
            ip_address.clone(),
            user_agent.clone(),
            &summary,
            RciAuditEventKind::VersionUnsupported,
        );
    }
    if let Some(pending) =
        record_pending_credential_pubkey_frame(db, node_id, pending_id, version, node_pubkey).await
    {
        if pending.fan_out_nodes.is_empty() {
            log_rci_for_node_pending(
                db.clone(),
                ip_address,
                user_agent,
                &pending,
                RciAuditEventKind::PubkeyPosted,
            );
        } else {
            log_rci_for_node_fan_out_target(
                db.clone(),
                ip_address,
                user_agent,
                &pending,
                node_id,
                RciAuditEventKind::PubkeyPosted,
            );
        }
    }
}

async fn record_pending_credential_decrypt_result_frame(
    db: &mongodb::Database,
    node_id: &str,
    pending_id: String,
    status: String,
    error_code: Option<serde_json::Value>,
) -> Option<(NodePendingCredential, RciAuditEventKind)> {
    let outcome = match status.as_str() {
        "ok" => node_pending_credential_service::PendingCredentialDecryptOutcome::Ok,
        "error" => node_pending_credential_service::PendingCredentialDecryptOutcome::Error,
        _other => {
            tracing::warn!(
                node_id = %node_id,
                pending_id = %pending_id,
                "Ignoring pending credential decrypt_result with unsupported status"
            );
            return None;
        }
    };
    match node_pending_credential_service::record_pending_credential_decrypt_result(
        db,
        node_id,
        &pending_id,
        outcome,
        chrono::Utc::now(),
    )
    .await
    {
        Ok(pending) => {
            let event_kind = match outcome {
                node_pending_credential_service::PendingCredentialDecryptOutcome::Ok => {
                    RciAuditEventKind::DecryptSucceeded
                }
                node_pending_credential_service::PendingCredentialDecryptOutcome::Error => {
                    match error_code
                        .as_ref()
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|code| RciAuditErrorKind::from_code(code as u32))
                    {
                        Some(RciAuditErrorKind::VersionUnsupported) => {
                            RciAuditEventKind::from_error_kind(
                                RciAuditErrorKind::VersionUnsupported,
                            )
                        }
                        _ => RciAuditEventKind::from_error_kind(RciAuditErrorKind::DecryptFailed),
                    }
                }
            };
            Some((pending, event_kind))
        }
        Err(err) => {
            if matches!(err, AppError::NotFound(_)) {
                let parsed_error_code = error_code
                    .as_ref()
                    .and_then(serde_json::Value::as_u64)
                    .map(|code| code as u32);
                match node_pending_credential_service::record_fan_out_decrypt_result(
                    db,
                    node_id,
                    &pending_id,
                    outcome,
                    parsed_error_code,
                    chrono::Utc::now(),
                )
                .await
                {
                    Ok(pending) => {
                        let event_kind = match outcome {
                            node_pending_credential_service::PendingCredentialDecryptOutcome::Ok => {
                                RciAuditEventKind::DecryptSucceeded
                            }
                            node_pending_credential_service::PendingCredentialDecryptOutcome::Error => {
                                match parsed_error_code
                                    .and_then(RciAuditErrorKind::from_code)
                                {
                                    Some(RciAuditErrorKind::VersionUnsupported) => {
                                        RciAuditEventKind::from_error_kind(
                                            RciAuditErrorKind::VersionUnsupported,
                                        )
                                    }
                                    _ => RciAuditEventKind::from_error_kind(
                                        RciAuditErrorKind::DecryptFailed,
                                    ),
                                }
                            }
                        };
                        Some((pending, event_kind))
                    }
                    Err(err) => {
                        tracing::warn!(
                            node_id = %node_id,
                            pending_id = %pending_id,
                            error = %err,
                            "Failed to record fan-out pending credential decrypt_result"
                        );
                        None
                    }
                }
            } else {
                tracing::warn!(
                    node_id = %node_id,
                    pending_id = %pending_id,
                    error = %err,
                    "Failed to record pending credential decrypt_result"
                );
                None
            }
        }
    }
}

async fn handle_pending_credential_decrypt_result_message(
    db: &mongodb::Database,
    node_id: &str,
    ip_address: Option<String>,
    user_agent: Option<String>,
    pending_id: String,
    status: String,
    error_code: Option<serde_json::Value>,
) {
    if let Some((pending, event_kind)) =
        record_pending_credential_decrypt_result_frame(db, node_id, pending_id, status, error_code)
            .await
    {
        if pending.fan_out_nodes.is_empty() {
            log_rci_for_node_pending(db.clone(), ip_address, user_agent, &pending, event_kind);
        } else {
            log_rci_for_node_fan_out_target(
                db.clone(),
                ip_address.clone(),
                user_agent.clone(),
                &pending,
                node_id,
                event_kind,
            );
            let aggregate_subject =
                rci_audit_service::RciFanOutAuditSubject::from_pending(&pending);
            match pending.remote_state {
                Some(crate::models::node_pending_credential::RemoteCryptoState::Consumed) => {
                    rci_audit_service::log_rci_fan_out_for_node(
                        db.clone(),
                        &pending.owner_user_id,
                        ip_address,
                        user_agent,
                        &aggregate_subject,
                        rci_audit_service::RciFanOutAuditEventKind::Completed,
                    );
                }
                Some(
                    crate::models::node_pending_credential::RemoteCryptoState::PartialDecrypted,
                ) => {
                    rci_audit_service::log_rci_fan_out_for_node(
                        db.clone(),
                        &pending.owner_user_id,
                        ip_address,
                        user_agent,
                        &aggregate_subject,
                        rci_audit_service::RciFanOutAuditEventKind::Partial,
                    );
                }
                _ => {}
            }
        }
    }
}

/// GET /api/v1/nodes/ws
///
/// WebSocket upgrade handler for node agent connections.
/// Authentication happens in the first message (register or auth).
/// If no valid auth message within 10 seconds, connection is closed.
///
/// Security: The global rate limiter applies to the HTTP upgrade request.
/// Additionally, a max concurrent connections limit is enforced here.
/// Auth tokens should only be transmitted over TLS/WSS in production.
pub async fn ws_handler(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // Enforce max concurrent WebSocket connections (includes pending auth).
    // M6: This check + increment is not atomic (TOCTOU). Concurrent upgrade
    // requests could slightly exceed the limit (by 1-2 connections). This is
    // acceptable since the limit is a soft cap and the race window is narrow.
    if state.node_ws_manager.total_connection_count() >= state.node_ws_manager.max_connections() {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "Maximum node connections reached",
        )
            .into_response();
    }

    state.node_ws_manager.increment_pending_auth();

    // H3: Create RAII guard so the pending auth counter is decremented
    // even if the upgrade future is cancelled or the task is aborted.
    let guard = PendingAuthGuard {
        manager: state.node_ws_manager.clone(),
    };

    let ip = ws_extract_ip(&headers, Some(peer));
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    ws.on_upgrade(move |socket| handle_node_connection(state, socket, guard, ip, ua))
        .into_response()
}

fn ws_extract_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(forwarded);
    }
    if let Some(real_ip) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(real_ip);
    }
    peer.map(|addr| addr.ip().to_string())
}

async fn handle_node_connection(
    state: AppState,
    socket: WebSocket,
    _guard: PendingAuthGuard,
    ip_address: Option<String>,
    user_agent: Option<String>,
) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Wait for auth/register message with 10s timeout
    let auth_result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(msg) = ws_stream.next().await {
            let msg = match msg {
                Ok(Message::Text(text)) => text,
                Ok(Message::Close(_)) => return None,
                Ok(_) => continue,
                Err(_) => return None,
            };

            let parsed: NodeMessage = match serde_json::from_str(&msg) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "Invalid WebSocket auth message");
                    let err_msg = serde_json::json!({
                        "type": "auth_error",
                        "message": "Invalid message format"
                    });
                    let _ = ws_sink
                        .send(Message::Text(err_msg.to_string().into()))
                        .await;
                    // M5: Audit log failed auth (invalid message format)
                    audit_service::log_async(
                        state.db.clone(),
                        None,
                        "node_ws_auth_failed".to_string(),
                        Some(serde_json::json!({ "reason": "invalid_message_format" })),
                        ip_address.clone(),
                        user_agent.clone(),
                        None,
                        None,
                    );
                    return None;
                }
            };

            match parsed {
                NodeMessage::Register { token, metadata } => {
                    match node_service::register_node(
                        &state.db,
                        &state.encryption_keys,
                        &token,
                        metadata,
                    )
                    .await
                    {
                        Ok((node, raw_auth_token, raw_signing_secret)) => {
                            let ok_msg = serde_json::json!({
                                "type": "register_ok",
                                "node_id": &node.id,
                                "auth_token": raw_auth_token,
                                "signing_secret": raw_signing_secret,
                            });
                            let _ = ws_sink.send(Message::Text(ok_msg.to_string().into())).await;

                            // Telemetry: node.registered. `profile` is unknown server-side
                            // (the CLI-side profile name is never sent over the wire).
                            // TODO(telemetry): see TELEMETRY.md §6.5 degraded emissions -- profile unknown server-side
                            let node_platform = node
                                .metadata
                                .as_ref()
                                .and_then(|m| m.os.clone())
                                .unwrap_or_else(|| "unknown".to_string());
                            let ctx = TelemetryContext::default();
                            emit_event(
                                state.telemetry.as_deref(),
                                &node.user_id,
                                None,
                                &ctx,
                                TelemetryEvent::NodeRegistered {
                                    node_platform,
                                    profile: "unknown".to_string(),
                                },
                            );

                            return Some((node.id, node.user_id));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Node registration failed");
                            let err_msg = serde_json::json!({
                                "type": "auth_error",
                                "message": "Registration failed"
                            });
                            let _ = ws_sink
                                .send(Message::Text(err_msg.to_string().into()))
                                .await;
                            // M5: Audit log failed registration
                            audit_service::log_async(
                                state.db.clone(),
                                None,
                                "node_ws_auth_failed".to_string(),
                                Some(serde_json::json!({ "reason": "registration_failed" })),
                        ip_address.clone(),
                        user_agent.clone(),
                        None,
                        None,
                            );
                            return None;
                        }
                    }
                }
                NodeMessage::Auth { node_id, token } => {
                    match node_service::validate_auth_token(&state.db, &token).await {
                        Ok(node) if node.id == node_id => {
                            let ok_msg = serde_json::json!({
                                "type": "auth_ok",
                                "node_id": &node.id,
                                "heartbeat_interval_secs": state.config.node_heartbeat_interval_secs,
                                "capabilities": {
                                    "proxy_binary_chunks": true
                                }
                            });
                            let _ = ws_sink.send(Message::Text(ok_msg.to_string().into())).await;
                            return Some((node.id, node.user_id));
                        }
                        Ok(_) => {
                            let err_msg = serde_json::json!({
                                "type": "auth_error",
                                "message": "Token does not match node_id"
                            });
                            let _ = ws_sink
                                .send(Message::Text(err_msg.to_string().into()))
                                .await;
                            // M5: Audit log node_id mismatch
                            audit_service::log_async(
                                state.db.clone(),
                                None,
                                "node_ws_auth_failed".to_string(),
                                Some(serde_json::json!({
                                    "reason": "node_id_mismatch",
                                    "claimed_node_id": &node_id,
                                })),
                        ip_address.clone(),
                        user_agent.clone(),
                        None,
                        None,
                            );
                            return None;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Node auth token validation failed");
                            let err_msg = serde_json::json!({
                                "type": "auth_error",
                                "message": "Authentication failed"
                            });
                            let _ = ws_sink
                                .send(Message::Text(err_msg.to_string().into()))
                                .await;
                            // M5: Audit log invalid auth token
                            audit_service::log_async(
                                state.db.clone(),
                                None,
                                "node_ws_auth_failed".to_string(),
                                Some(serde_json::json!({ "reason": "invalid_auth_token" })),
                        ip_address.clone(),
                        user_agent.clone(),
                        None,
                        None,
                            );
                            return None;
                        }
                    }
                }
                _ => {
                    let err_msg = serde_json::json!({
                        "type": "auth_error",
                        "message": "First message must be 'register' or 'auth'"
                    });
                    let _ = ws_sink
                        .send(Message::Text(err_msg.to_string().into()))
                        .await;
                    // M5: Audit log unexpected first message
                    audit_service::log_async(
                        state.db.clone(),
                        None,
                        "node_ws_auth_failed".to_string(),
                        Some(serde_json::json!({ "reason": "unexpected_first_message" })),
                        ip_address.clone(),
                        user_agent.clone(),
                        None,
                        None,
                    );
                    return None;
                }
            }
        }
        None
    })
    .await;

    // H3: The RAII guard (_guard) decrements pending_auth on drop.
    // Drop it explicitly here since auth phase is complete.
    drop(_guard);

    let (node_id, owner_user_id) = match auth_result {
        Ok(Some(pair)) => pair,
        _ => {
            // Timeout or auth failure -- close connection
            let _ = ws_sink
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 4001,
                    reason: "Authentication timeout or failure".into(),
                })))
                .await;
            return;
        }
    };

    tracing::info!(node_id = %node_id, "Node connected via WebSocket");

    // H4: Use bounded channel to prevent memory exhaustion from slow/malicious nodes
    let (tx, mut rx) = mpsc::channel::<NodeOutboundMessage>(WS_WRITER_CHANNEL_SIZE);
    state.node_ws_manager.register_connection(&node_id, tx);

    // Telemetry: node.connected. Emitted once after WS auth + registration
    // in the manager. `profile` is unknown server-side -- only the CLI knows
    // the profile name.
    // TODO(telemetry): see TELEMETRY.md §6.5 degraded emissions -- profile unknown server-side
    {
        let ctx = TelemetryContext::default();
        emit_event(
            state.telemetry.as_deref(),
            &owner_user_id,
            None,
            &ctx,
            TelemetryEvent::NodeConnected {
                // Raw node_id is a UUID and would be redacted by
                // `telemetry::scrub` to `[UUID_REDACTED]`, collapsing
                // every node onto the same property value. Hash keeps
                // per-node granularity without leaking the UUID.
                node_id: hash_short_id(&node_id),
                profile: "unknown".to_string(),
            },
        );
    }

    // Mark node online
    if let Err(e) = node_service::set_node_status(&state.db, &node_id, NodeStatus::Online).await {
        tracing::warn!(node_id = %node_id, error = %e, "Failed to set node status to online");
    }

    // Spawn writer task: forwards messages from the channel to the WS sink
    let node_id_writer = node_id.clone();
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                NodeOutboundMessage::Text(text) => {
                    if ws_sink.send(Message::Text(text.into())).await.is_err() {
                        tracing::debug!(node_id = %node_id_writer, "WebSocket send failed, closing writer");
                        break;
                    }
                }
                NodeOutboundMessage::Close { code, reason } => {
                    let _ = ws_sink
                        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code,
                            reason: reason.into(),
                        })))
                        .await;
                    break;
                }
            }
        }
    });

    // Reader loop: process incoming messages from the node
    let node_id_reader = node_id.clone();
    let db = state.db.clone();
    let ws_manager = state.node_ws_manager.clone();

    // Track teardown reason for telemetry. Default assumes a clean client
    // close; any read error flips it to "error" before the loop breaks.
    let mut reason: &'static str = "client_close";

    while let Some(msg) = ws_stream.next().await {
        // Binary frames carry streaming proxy data chunks:
        //   [36 bytes: request_id as ASCII UUID][remaining: raw data]
        if let Ok(Message::Binary(data)) = &msg {
            match decode_binary_stream_frame(data) {
                Ok((request_id, chunk)) => {
                    ws_manager.deliver_stream_chunk(&node_id_reader, request_id, chunk.to_vec());
                }
                Err(message) => {
                    tracing::warn!(node_id = %node_id_reader, len = data.len(), "{message}");
                }
            }
            continue;
        }

        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => {
                reason = "client_close";
                break;
            }
            Ok(Message::Ping(_)) => continue,
            Ok(_) => continue,
            Err(e) => {
                tracing::debug!(node_id = %node_id_reader, error = %e, "WebSocket read error");
                reason = "error";
                break;
            }
        };

        let parsed: NodeMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(node_id = %node_id_reader, error = %e, "Invalid message from node");
                continue;
            }
        };

        match parsed {
            NodeMessage::HeartbeatPong { .. } => {
                if let Err(e) = node_service::update_heartbeat(&db, &node_id_reader, None).await {
                    tracing::warn!(
                        node_id = %node_id_reader,
                        error = %e,
                        "Failed to update heartbeat"
                    );
                }
            }
            NodeMessage::ProxyResponse(resp) => {
                let Some(body) =
                    decode_base64_payload(resp.body.as_deref(), "proxy_response", &resp.request_id)
                else {
                    ws_manager.deliver_proxy_error(
                        &node_id_reader,
                        &resp.request_id,
                        "invalid_base64_payload",
                        502,
                        false,
                        None,
                    );
                    continue;
                };

                let headers: Vec<(String, String)> = resp
                    .headers
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();

                ws_manager.deliver_proxy_response(
                    &node_id_reader,
                    NodeProxyResponse {
                        request_id: resp.request_id,
                        status: resp.status,
                        headers,
                        body,
                    },
                );
            }
            NodeMessage::ProxyError(err) => {
                let status = err.status.unwrap_or(502);
                ws_manager.deliver_proxy_error(
                    &node_id_reader,
                    &err.request_id,
                    &err.error,
                    status,
                    err.retryable,
                    err.reason.as_deref(),
                );
            }
            NodeMessage::ProxyResponseStart(start) => {
                let headers: Vec<(String, String)> = start
                    .headers
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();

                // Upgrade from an awaiting correlated response to a streaming
                // receiver consumed by the proxy handler.
                if !ws_manager.deliver_stream_start(
                    &node_id_reader,
                    &start.request_id,
                    start.status,
                    headers,
                ) {
                    tracing::debug!(
                        node_id = %node_id_reader,
                        request_id = %start.request_id,
                        "Dropped stream start for unknown or expired request"
                    );
                }
            }
            NodeMessage::ProxyResponseChunk(chunk) => {
                handle_proxy_response_chunk(&ws_manager, &node_id_reader, chunk);
            }
            NodeMessage::ProxyResponseEnd(end) => {
                ws_manager.deliver_stream_end(&node_id_reader, &end.request_id);
            }
            NodeMessage::StatusUpdate { capabilities, .. } => {
                // Record capability flags so `push_credential_to_node_strict`
                // can decide between strict ack-wait and legacy fire-and-
                // forget delivery. Old agents omit the field → `caps` is
                // `None` → flags default to all-false → strict mode stays
                // off for them (twenty-seventh-round Codex P2).
                // Always mark capability state "resolved" on any
                // status_update, regardless of whether `capabilities`
                // was present. This releases strict-push waiters
                // parked in `await_capability_resolution` — the
                // flag's value (present vs. absent) is what they
                // want to observe, and it's now final for this
                // connection (twenty-ninth-round Codex P2).
                apply_status_update_capabilities(&state, &node_id_reader, capabilities).await;
                tracing::debug!(node_id = %node_id_reader, "Received status_update");
            }
            NodeMessage::SshTunnelOpened(opened) => {
                if !ws_manager.deliver_ssh_tunnel_opened(&node_id_reader, &opened.session_id) {
                    tracing::debug!(
                        node_id = %node_id_reader,
                        session_id = %opened.session_id,
                        "Dropped SSH tunnel opened event for unknown session"
                    );
                }
            }
            NodeMessage::SshTunnelData(data) => {
                if let Some(bytes) =
                    decode_base64_payload(data.data.as_deref(), "ssh_tunnel_data", &data.session_id)
                {
                    ws_manager.deliver_ssh_tunnel_data(&node_id_reader, &data.session_id, bytes);
                } else {
                    ws_manager.deliver_ssh_tunnel_closed(
                        &node_id_reader,
                        &data.session_id,
                        Some("invalid_base64_payload".to_string()),
                    );
                }
            }
            NodeMessage::SshTunnelClosed(closed) => {
                ws_manager.deliver_ssh_tunnel_closed(
                    &node_id_reader,
                    &closed.session_id,
                    closed.error,
                );
            }
            NodeMessage::WebTerminalStarted(started) => {
                if !ws_manager.deliver_web_terminal_started(&node_id_reader, &started.session_id) {
                    tracing::debug!(
                        node_id = %node_id_reader,
                        session_id = %started.session_id,
                        "Dropped web terminal started event for unknown session"
                    );
                }
            }
            NodeMessage::WebTerminalData(data) => {
                if let Some(bytes) = decode_base64_payload(
                    data.data.as_deref(),
                    "web_terminal_data",
                    &data.session_id,
                ) {
                    ws_manager.deliver_web_terminal_data(&node_id_reader, &data.session_id, bytes);
                } else {
                    ws_manager.deliver_web_terminal_closed(
                        &node_id_reader,
                        &data.session_id,
                        Some("invalid_base64_payload".to_string()),
                    );
                }
            }
            NodeMessage::WebTerminalClosed(closed) => {
                ws_manager.deliver_web_terminal_closed(
                    &node_id_reader,
                    &closed.session_id,
                    closed.error,
                );
            }
            NodeMessage::SshExecResult(result) => {
                let stdout = decode_base64_payload(
                    result.stdout.as_deref(),
                    "ssh_exec_result",
                    &result.request_id,
                )
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();
                let stderr = decode_base64_payload(
                    result.stderr.as_deref(),
                    "ssh_exec_result",
                    &result.request_id,
                )
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();

                ws_manager.deliver_ssh_exec_result(
                    &node_id_reader,
                    NodeSshExecResult {
                        request_id: result.request_id,
                        exit_code: result.exit_code,
                        stdout,
                        stderr,
                        duration_ms: result.duration_ms,
                        timed_out: result.timed_out,
                        error: result.error,
                        error_code: result.error_code,
                    },
                );
            }
            NodeMessage::SshNodeExecData(data) => {
                if let Some(bytes) = decode_base64_payload(
                    data.data.as_deref(),
                    "ssh_node_exec_data",
                    &data.request_id,
                ) {
                    ws_manager.deliver_ssh_node_exec_data(
                        &node_id_reader,
                        &data.request_id,
                        data.stream.as_deref(),
                        bytes,
                    );
                } else {
                    ws_manager.deliver_ssh_node_exec_error(
                        &node_id_reader,
                        data.request_id,
                        "invalid_base64_payload".to_string(),
                        Some(1013),
                        0,
                    );
                }
            }
            NodeMessage::SshNodeExecClose(closed) => {
                ws_manager.deliver_ssh_node_exec_close(
                    &node_id_reader,
                    closed.request_id,
                    closed.exit_code,
                    closed.duration_ms,
                    closed.timed_out,
                );
            }
            NodeMessage::SshNodeExecError(error) => {
                ws_manager.deliver_ssh_node_exec_error(
                    &node_id_reader,
                    error.request_id,
                    error.error,
                    error.error_code,
                    error.duration_ms,
                );
            }
            NodeMessage::CredentialUpdateAck {
                request_id,
                service_slug,
                status,
                error,
            } => {
                let slug = service_slug.as_deref().unwrap_or("unknown");
                let st = status.as_deref().unwrap_or("unknown");
                if st == "ok" {
                    tracing::info!(
                        node_id = %node_id_reader,
                        slug = %slug,
                        "Node acknowledged credential update"
                    );
                } else {
                    let err = error.as_deref().unwrap_or("unknown");
                    tracing::warn!(
                        node_id = %node_id_reader,
                        slug = %slug,
                        error = %err,
                        "Node failed to apply credential update"
                    );
                }
                // Resolve any strict-push waiter registered for this
                // `request_id`. Legacy node agents don't echo
                // `request_id`; the ack is still logged but no waiter
                // is woken up. New CLIs echo it back so the backend
                // knows whether the node-side apply actually landed.
                if let Some(req_id) = request_id {
                    use crate::services::node_ws_manager::CredentialAckOutcome;
                    let outcome = if st == "ok" {
                        CredentialAckOutcome::Ok
                    } else {
                        CredentialAckOutcome::Err(
                            error.unwrap_or_else(|| "unknown node error".to_string()),
                        )
                    };
                    ws_manager.deliver_credential_ack(&node_id_reader, &req_id, outcome);
                }
            }
            NodeMessage::PendingCredentialPubkey {
                pending_id,
                version,
                node_pubkey,
            } => {
                handle_pending_credential_pubkey_message(
                    &db,
                    &node_id_reader,
                    ip_address.clone(),
                    user_agent.clone(),
                    pending_id,
                    version,
                    node_pubkey,
                )
                .await;
            }
            NodeMessage::PendingCredentialDecryptResult {
                pending_id,
                status,
                error_code,
            } => {
                handle_pending_credential_decrypt_result_message(
                    &db,
                    &node_id_reader,
                    ip_address.clone(),
                    user_agent.clone(),
                    pending_id,
                    status,
                    error_code,
                )
                .await;
            }
            NodeMessage::WsProxyOpened(msg) => {
                if !ws_manager.deliver_ws_proxy_opened(
                    &node_id_reader,
                    &msg.session_id,
                    msg.selected_protocol,
                ) {
                    tracing::debug!(
                        node_id = %node_id_reader,
                        session_id = %msg.session_id,
                        "ws_proxy_opened for unknown session"
                    );
                }
            }
            NodeMessage::WsProxyText(msg) => {
                ws_manager.deliver_ws_proxy_text(&node_id_reader, &msg.session_id, msg.data);
            }
            NodeMessage::WsProxyBinary(msg) => {
                if let Some(bytes) =
                    decode_base64_payload(Some(&msg.data), "ws_proxy_binary", &msg.session_id)
                {
                    ws_manager.deliver_ws_proxy_binary(&node_id_reader, &msg.session_id, bytes);
                } else {
                    ws_manager.deliver_ws_proxy_closed(
                        &node_id_reader,
                        &msg.session_id,
                        None,
                        Some("Invalid base64 in ws_proxy_binary".to_string()),
                    );
                }
            }
            NodeMessage::WsProxyClosed(msg) => {
                ws_manager.deliver_ws_proxy_closed(
                    &node_id_reader,
                    &msg.session_id,
                    msg.code,
                    msg.reason,
                );
            }
            NodeMessage::WsProxyError(msg) => {
                ws_manager.deliver_ws_proxy_error(&node_id_reader, &msg.session_id, &msg.error);
            }
            NodeMessage::WsFrameInjected(msg) => {
                ws_manager.deliver_ws_frame_injected(
                    &node_id_reader,
                    &msg.session_id,
                    msg.trigger_kind,
                    msg.frame_index,
                );
            }
            NodeMessage::Register { .. } | NodeMessage::Auth { .. } => {
                // Already authenticated, ignore duplicate auth messages
                tracing::warn!(
                    node_id = %node_id_reader,
                    "Received auth message on already-authenticated connection"
                );
            }
        }
    }

    // Cleanup on disconnect
    tracing::info!(node_id = %node_id, "Node disconnected");
    writer_task.abort();
    ws_manager.unregister_connection(&node_id);

    if let Err(e) = node_service::set_node_status(&state.db, &node_id, NodeStatus::Offline).await {
        tracing::warn!(node_id = %node_id, error = %e, "Failed to set node status to offline");
    }

    // Telemetry: node.disconnected. Single-owner rule -- only the reader
    // teardown emits this event. Admin-initiated disconnects + heartbeat
    // sweep disconnects are tracked via their own events (or are §6.5
    // leftovers today); they will route back here when the WS reader
    // observes the close frame.
    {
        let ctx = TelemetryContext::default();
        emit_event(
            state.telemetry.as_deref(),
            &owner_user_id,
            None,
            &ctx,
            TelemetryEvent::NodeDisconnected {
                // See note on NodeConnected: raw UUIDs are redacted by the
                // scrubber, so hash for telemetry.
                node_id: hash_short_id(&node_id),
                reason: reason.to_string(),
            },
        );
    }
}

/// Heartbeat sweep: check timeouts first, then send pings to surviving nodes.
/// Called periodically from the background task in main.rs.
///
/// The order matters: we check whether the *previous* ping was answered before
/// sending the next one.  This avoids a race where we send a ping and
/// immediately check the (not-yet-updated) `last_heartbeat_at`.
pub async fn node_ws_manager_heartbeat_sweep(
    db: &mongodb::Database,
    ws_manager: &Arc<crate::services::node_ws_manager::NodeWsManager>,
    timeout_secs: u64,
) {
    let node_ids = ws_manager.connected_node_ids();

    for node_id in &node_ids {
        // 1. Check if the previous heartbeat was answered in time.
        //    Skip for nodes with no last_heartbeat_at (newly connected).
        let timed_out = match node_service::get_node_by_id(db, node_id).await {
            Ok(Some(node)) => {
                if let Some(last_hb) = node.last_heartbeat_at {
                    let elapsed = chrono::Utc::now()
                        .signed_duration_since(last_hb)
                        .num_seconds();
                    if elapsed > timeout_secs as i64 {
                        tracing::info!(
                            node_id = %node_id,
                            elapsed_secs = elapsed,
                            "Node heartbeat timeout, disconnecting"
                        );
                        ws_manager
                            .disconnect_connection(node_id, 4005, "heartbeat timeout")
                            .await;
                        if let Err(e) =
                            node_service::set_node_status(db, node_id, NodeStatus::Offline).await
                        {
                            tracing::warn!(
                                node_id = %node_id,
                                error = %e,
                                "Failed to set node offline after timeout"
                            );
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Ok(None) => {
                // Node was deleted, disconnect
                ws_manager
                    .disconnect_connection(node_id, 4006, "node deleted")
                    .await;
                true
            }
            Err(e) => {
                tracing::warn!(
                    node_id = %node_id,
                    error = %e,
                    "Failed to check node heartbeat"
                );
                false
            }
        };

        if timed_out {
            continue;
        }

        // 2. Send the next heartbeat ping (node will respond with pong,
        //    which updates last_heartbeat_at before the next sweep).
        if let Err(e) = ws_manager.send_heartbeat_ping(node_id) {
            tracing::debug!(node_id = %node_id, error = %e, "Failed to send heartbeat ping");
            ws_manager
                .disconnect_connection(node_id, 4004, "heartbeat ping failed")
                .await;
            if let Err(e) = node_service::set_node_status(db, node_id, NodeStatus::Offline).await {
                tracing::warn!(node_id = %node_id, error = %e, "Failed to set node offline after ping failure");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_status_update_capabilities, decode_base64_payload, decode_binary_stream_frame,
        drain_queued_pending_ciphertexts, handle_pending_credential_decrypt_result_message,
        handle_pending_credential_pubkey_message, handle_proxy_response_chunk,
        record_pending_credential_decrypt_result_frame, record_pending_credential_pubkey_frame,
        validate_base64url_no_pad_exact,
    };
    use base64::Engine;
    use chrono::{Duration, Utc};
    use mongodb::bson::{self, doc};
    use serde_json::Value;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    use crate::crypto::token::hash_token;
    use crate::errors::{
        PENDING_CREDENTIAL_CIPHERTEXT_TOO_LARGE_CODE, PENDING_CREDENTIAL_DECRYPT_FAILED_CODE,
    };
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_pending_credential::{
        COLLECTION_NAME as NODE_PENDING_CREDENTIALS, CryptoBundle, FanOutDecryptOutcome,
        FanOutNodeState, InjectionMethod, NodePendingCredential, RemoteCryptoState,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::node_ws_manager::{
        NodeCapabilitiesMsg, NodeOutboundMessage, NodeProxyRequest, NodeWsManager,
        ProxyResponseType, StreamChunk, WsProxyResponseChunkMsg,
    };
    use crate::services::{
        audit_service, node_pending_credential_service,
        rci_audit_service::{self, RciAuditEventKind, RciAuditSubject},
    };
    use crate::test_utils::{
        assert_rci_audit_row, connect_test_database, test_app_state, test_user,
    };

    async fn test_db(prefix: &str) -> mongodb::Database {
        connect_test_database(prefix)
            .await
            .expect("local MongoDB required for node WS behavior tests")
    }

    fn b64url(byte: u8, len: usize) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![byte; len])
    }

    fn test_node(owner_id: &str, name: &str, raw_auth_token: &str) -> Node {
        let now = Utc::now();
        Node {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: hash_token(raw_auth_token),
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

    async fn insert_user_and_node(db: &mongodb::Database, owner_id: &str, node: &Node) {
        db.collection(USERS)
            .insert_one(test_user(owner_id, UserType::Person))
            .await
            .expect("insert user");
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");
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
            &b64url(5, 32),
        )
        .await
        .expect("record node pubkey");
        pending
    }

    async fn store_ciphertext(
        db: &mongodb::Database,
        actor_id: &str,
        node_id: &str,
        pending_id: &str,
        node_connected: bool,
    ) {
        node_pending_credential_service::store_pending_ciphertext_first_writer_wins(
            db,
            actor_id,
            node_id,
            pending_id,
            node_pending_credential_service::StorePendingCiphertextInput::new(
                b64url(6, 32),
                b64url(7, 24),
                vec![1, 2, 3, 4],
            ),
            node_connected,
            Utc::now(),
        )
        .await
        .expect("store pending ciphertext");
    }

    async fn load_pending(db: &mongodb::Database, pending_id: &str) -> NodePendingCredential {
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": pending_id })
            .await
            .expect("query pending credential")
            .expect("pending credential exists")
    }

    fn fan_out_target_state(
        node_id: &str,
        remote_state: RemoteCryptoState,
        now: chrono::DateTime<Utc>,
        ciphertext_byte: u8,
    ) -> FanOutNodeState {
        let consumed = matches!(remote_state, RemoteCryptoState::Consumed);
        let declined = matches!(remote_state, RemoteCryptoState::Declined);
        let decrypt_failed = matches!(remote_state, RemoteCryptoState::DecryptFailed);
        let completed = matches!(
            remote_state,
            RemoteCryptoState::Consumed
                | RemoteCryptoState::Declined
                | RemoteCryptoState::DecryptFailed
                | RemoteCryptoState::Expired
        );
        let decrypt_outcome = if consumed {
            Some(FanOutDecryptOutcome::Ok)
        } else if declined || decrypt_failed {
            Some(FanOutDecryptOutcome::Error)
        } else {
            None
        };
        FanOutNodeState {
            node_id: node_id.to_string(),
            generation: 0,
            crypto: CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: b64url(ciphertext_byte, 32),
                admin_pubkey: (!completed).then(|| b64url(20 + ciphertext_byte, 32)),
                nonce: (!completed).then(|| b64url(40 + ciphertext_byte, 24)),
                ciphertext: (!completed).then(|| vec![ciphertext_byte; 4]),
            },
            remote_state: Some(remote_state),
            decrypt_outcome,
            error_code: None,
            error_kind: None,
            pubkey_posted_at: Some(now),
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            consumed_at: consumed.then_some(now),
            declined_at: declined.then_some(now),
            updated_at: now,
        }
    }

    async fn insert_primary_fan_out_pending(
        db: &mongodb::Database,
        owner_id: &str,
        primary_node_id: &str,
        other_node_id: &str,
        service_slug: &str,
        other_state: RemoteCryptoState,
        top_state: RemoteCryptoState,
    ) -> NodePendingCredential {
        let now = Utc::now();
        let pending = NodePendingCredential {
            id: uuid::Uuid::new_v4().to_string(),
            node_id: primary_node_id.to_string(),
            service_slug: service_slug.to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: Some("Production".to_string()),
            created_by_user_id: owner_id.to_string(),
            owner_user_id: owner_id.to_string(),
            created_at: now,
            expires_at: now + Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: Some(top_state),
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: vec![
                fan_out_target_state(
                    primary_node_id,
                    RemoteCryptoState::CiphertextReceived,
                    now,
                    1,
                ),
                fan_out_target_state(other_node_id, other_state, now, 2),
            ],
            fan_out_revision: 1,
        };
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .insert_one(&pending)
            .await
            .expect("insert fan-out pending credential");
        pending
    }

    fn fan_out_target<'a>(
        pending: &'a NodePendingCredential,
        node_id: &str,
    ) -> &'a FanOutNodeState {
        node_pending_credential_service::fan_out_target(pending, node_id)
            .expect("fan-out target exists")
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

    #[tokio::test]
    async fn ws_pending_credential_pubkey_records_valid_and_ignores_invalid() {
        let db = test_db("ws_pending_pubkey_behavior").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_pubkey";
        let node = test_node(&owner_id, "ws-pubkey-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let valid_pending = create_remote_pending(&db, &owner_id, &node.id, "valid-pubkey").await;
        let invalid_pending =
            create_remote_pending(&db, &owner_id, &node.id, "invalid-pubkey").await;

        let invalid_result = record_pending_credential_pubkey_frame(
            &db,
            &node.id,
            invalid_pending.id.clone(),
            "v1".to_string(),
            b64url(9, 31),
        )
        .await;
        assert!(invalid_result.is_none());

        let valid = record_pending_credential_pubkey_frame(
            &db,
            &node.id,
            valid_pending.id.clone(),
            "v1".to_string(),
            b64url(8, 32),
        )
        .await
        .expect("valid pubkey recorded");
        assert_eq!(valid.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert_eq!(
            valid
                .crypto
                .as_ref()
                .map(|crypto| crypto.node_pubkey.as_str()),
            Some(b64url(8, 32).as_str())
        );

        let invalid = load_pending(&db, &invalid_pending.id).await;
        assert!(
            invalid
                .crypto
                .as_ref()
                .is_some_and(|crypto| crypto.node_pubkey.is_empty())
        );
        assert!(invalid.remote_state.is_none());
    }

    #[tokio::test]
    async fn ws_pending_credential_pubkey_writes_metadata_audit_row() {
        let db = test_db("ws_pending_pubkey_audit").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_pubkey_audit";
        let node = test_node(&owner_id, "ws-pubkey-audit-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let pending = create_remote_pending(&db, &owner_id, &node.id, "pubkey-audit").await;
        let audit_rx = audit_service::notify_on_audit_write(
            "node_credential_rci_pubkey_posted",
            Some(pending.id.clone()),
        );

        handle_pending_credential_pubkey_message(
            &db,
            &node.id,
            Some("203.0.113.20".to_string()),
            Some("nyxid-node-test".to_string()),
            pending.id.clone(),
            "v1".to_string(),
            b64url(8, 32),
        )
        .await;

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::PubkeyPosted));
        assert_eq!(
            stored
                .crypto
                .as_ref()
                .map(|crypto| crypto.node_pubkey.as_str()),
            Some(b64url(8, 32).as_str())
        );
        let audit = load_audit_entry(&db, audit_rx).await;
        assert_eq!(audit.ip_address.as_deref(), Some("203.0.113.20"));
        assert_eq!(audit.user_agent.as_deref(), Some("nyxid-node-test"));
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_pubkey_posted",
            &stored,
            Some("pubkey_posted"),
            &[],
        );
    }

    #[tokio::test]
    async fn ws_pending_credential_pubkey_unsupported_version_writes_metadata_audit_row() {
        let db = test_db("ws_pending_pubkey_bad_version_audit").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_pubkey_bad_version";
        let node = test_node(&owner_id, "ws-pubkey-bad-version-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let pending = create_remote_pending(&db, &owner_id, &node.id, "pubkey-bad-version").await;
        let before = load_pending(&db, &pending.id).await;
        assert!(before.remote_state.is_none());
        assert!(
            before
                .crypto
                .as_ref()
                .is_some_and(|crypto| crypto.node_pubkey.is_empty())
        );
        let audit_rx = audit_service::notify_on_audit_write(
            "node_credential_rci_version_unsupported",
            Some(pending.id.clone()),
        );

        handle_pending_credential_pubkey_message(
            &db,
            &node.id,
            Some("203.0.113.21".to_string()),
            Some("nyxid-node-test".to_string()),
            pending.id.clone(),
            "v2".to_string(),
            b64url(8, 32),
        )
        .await;

        let stored = load_pending(&db, &pending.id).await;
        assert!(stored.remote_state.is_none());
        assert!(
            stored
                .crypto
                .as_ref()
                .is_some_and(|crypto| crypto.node_pubkey.is_empty())
        );
        let audit = load_audit_entry(&db, audit_rx).await;
        assert_eq!(audit.ip_address.as_deref(), Some("203.0.113.21"));
        assert_eq!(audit.user_agent.as_deref(), Some("nyxid-node-test"));
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_version_unsupported",
            &stored,
            Some("decrypt_failed"),
            &["error_code", "error_kind"],
        );
        let event_data = audit.event_data.as_ref().unwrap();
        assert_eq!(event_data["error_code"], 8007);
        assert_eq!(
            event_data["error_kind"],
            "pending_credential_version_unsupported"
        );
        assert!(!event_data.to_string().contains("v2"));
    }

    #[tokio::test]
    async fn ws_pending_credential_decrypt_result_records_ok_and_error_audit_metadata() {
        let db = test_db("ws_pending_decrypt_result_behavior").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_decrypt";
        let node = test_node(&owner_id, "ws-decrypt-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let ok_pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "decrypt-ok").await;
        let error_pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "decrypt-error").await;
        store_ciphertext(&db, &owner_id, &node.id, &ok_pending.id, true).await;
        store_ciphertext(&db, &owner_id, &node.id, &error_pending.id, true).await;

        let (ok, ok_event_kind) = record_pending_credential_decrypt_result_frame(
            &db,
            &node.id,
            ok_pending.id.clone(),
            "ok".to_string(),
            None,
        )
        .await
        .expect("ok decrypt result recorded");
        assert_eq!(ok.remote_state, Some(RemoteCryptoState::Consumed));
        assert!(!ok.is_active);
        assert!(ok.consumed_at.is_some());
        assert_eq!(ok_event_kind, RciAuditEventKind::DecryptSucceeded);

        let (failed, event_kind) = record_pending_credential_decrypt_result_frame(
            &db,
            &node.id,
            error_pending.id.clone(),
            "error".to_string(),
            Some(serde_json::json!(8006)),
        )
        .await
        .expect("error decrypt result recorded");
        assert_eq!(failed.remote_state, Some(RemoteCryptoState::DecryptFailed));
        assert!(!failed.is_active);
        assert!(failed.consumed_at.is_none());
        assert_eq!(event_kind, RciAuditEventKind::DecryptFailed);

        let raw = db
            .collection::<bson::Document>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": &error_pending.id })
            .await
            .expect("query raw pending")
            .expect("pending exists");
        let forbidden_field = ["remote", "error", "code"].join("_");
        assert!(raw.get(&forbidden_field).is_none());
        let crypto = raw.get_document("crypto").expect("crypto document");
        assert!(crypto.get("admin_pubkey").is_none());
        assert!(crypto.get("nonce").is_none());
        assert!(crypto.get("ciphertext").is_none());

        let event_data = rci_audit_service::rci_event_data(
            &RciAuditSubject::from_pending(&failed),
            event_kind,
            Utc::now(),
        );
        assert_eq!(event_data["node_id"], node.id);
        assert_eq!(event_data["pending_credential_id"], error_pending.id);
        assert_eq!(event_data["service_slug"], "decrypt-error");
        assert_eq!(event_data["owner_user_id"], owner_id);
        assert_eq!(event_data["error_code"], 8006);
        assert_eq!(
            event_data["error_kind"],
            "pending_credential_decrypt_failed"
        );
        let audit_json = event_data.to_string();
        assert!(!audit_json.contains("admin_pubkey"));
        assert!(!audit_json.contains("nonce"));
        assert!(!audit_json.contains("ciphertext"));
    }

    #[tokio::test]
    async fn ws_decrypt_result_for_primary_fan_out_target_uses_embedded_state_machine() {
        let db = test_db("ws_primary_fanout_decrypt_result").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let primary_node_id = uuid::Uuid::new_v4().to_string();
        let other_node_id = uuid::Uuid::new_v4().to_string();
        let pending = insert_primary_fan_out_pending(
            &db,
            &owner_id,
            &primary_node_id,
            &other_node_id,
            "fanout-primary-ws",
            RemoteCryptoState::CiphertextReceived,
            RemoteCryptoState::CiphertextReceived,
        )
        .await;
        let before = load_pending(&db, &pending.id).await;
        assert_eq!(before.node_id, primary_node_id);
        assert_eq!(before.fan_out_nodes[0].node_id, primary_node_id);
        let other_before = fan_out_target(&before, &other_node_id).clone();

        let (updated, event_kind) = record_pending_credential_decrypt_result_frame(
            &db,
            &primary_node_id,
            pending.id.clone(),
            "ok".to_string(),
            None,
        )
        .await
        .expect("primary fan-out decrypt result recorded");
        assert_eq!(event_kind, RciAuditEventKind::DecryptSucceeded);
        assert_eq!(updated.fan_out_revision, before.fan_out_revision + 1);
        assert_eq!(
            updated.remote_state,
            Some(RemoteCryptoState::PartialDecrypted)
        );
        assert!(updated.is_active);
        assert!(updated.consumed_at.is_none());
        assert!(updated.crypto.is_none());

        let primary_after = fan_out_target(&updated, &primary_node_id);
        assert_eq!(
            primary_after.remote_state,
            Some(RemoteCryptoState::Consumed)
        );
        assert_eq!(
            primary_after.decrypt_outcome.as_ref(),
            Some(&FanOutDecryptOutcome::Ok)
        );
        assert!(primary_after.consumed_at.is_some());
        assert!(primary_after.crypto.admin_pubkey.is_none());
        assert!(primary_after.crypto.nonce.is_none());
        assert!(primary_after.crypto.ciphertext.is_none());
        assert_eq!(fan_out_target(&updated, &other_node_id), &other_before);

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.fan_out_revision, before.fan_out_revision + 1);
        assert!(stored.is_active);
        assert_eq!(
            stored.remote_state,
            Some(RemoteCryptoState::PartialDecrypted)
        );
        assert_eq!(fan_out_target(&stored, &other_node_id), &other_before);
    }

    #[tokio::test]
    async fn ws_decrypt_result_version_unsupported_writes_metadata_audit_row() {
        let db = test_db("ws_pending_decrypt_8007_audit").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_decrypt_8007";
        let node = test_node(&owner_id, "ws-decrypt-8007-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "decrypt-8007").await;
        store_ciphertext(&db, &owner_id, &node.id, &pending.id, true).await;
        let audit_rx = audit_service::notify_on_audit_write(
            "node_credential_rci_version_unsupported",
            Some(pending.id.clone()),
        );

        handle_pending_credential_decrypt_result_message(
            &db,
            &node.id,
            Some("203.0.113.10".to_string()),
            Some("nyxid-node-test".to_string()),
            pending.id.clone(),
            "error".to_string(),
            Some(serde_json::json!(8007)),
        )
        .await;

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::DecryptFailed));
        assert!(!stored.is_active);
        let audit = load_audit_entry(&db, audit_rx).await;
        assert_eq!(audit.ip_address.as_deref(), Some("203.0.113.10"));
        assert_eq!(audit.user_agent.as_deref(), Some("nyxid-node-test"));
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_version_unsupported",
            &stored,
            Some("decrypt_failed"),
            &["error_code", "error_kind"],
        );
        let event_data = audit.event_data.as_ref().unwrap();
        assert_eq!(event_data["error_code"], 8007);
        assert_eq!(
            event_data["error_kind"],
            "pending_credential_version_unsupported"
        );
    }

    #[tokio::test]
    async fn ws_decrypt_result_success_and_default_failure_write_metadata_audit_rows() {
        let db = test_db("ws_pending_decrypt_success_failure_audit").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_decrypt_success_failure";
        let node = test_node(&owner_id, "ws-decrypt-success-failure-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let ok_pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "decrypt-success").await;
        let failed_pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "decrypt-default-failed")
                .await;
        store_ciphertext(&db, &owner_id, &node.id, &ok_pending.id, true).await;
        store_ciphertext(&db, &owner_id, &node.id, &failed_pending.id, true).await;
        let success_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_decrypt_succeeded",
            Some(ok_pending.id.clone()),
        );
        let failed_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_decrypt_failed",
            Some(failed_pending.id.clone()),
        );

        handle_pending_credential_decrypt_result_message(
            &db,
            &node.id,
            Some("203.0.113.30".to_string()),
            Some("nyxid-node-test".to_string()),
            ok_pending.id.clone(),
            "ok".to_string(),
            None,
        )
        .await;
        handle_pending_credential_decrypt_result_message(
            &db,
            &node.id,
            Some("203.0.113.31".to_string()),
            Some("nyxid-node-test".to_string()),
            failed_pending.id.clone(),
            "error".to_string(),
            Some(serde_json::json!("raw-node-error-fixture")),
        )
        .await;

        let ok = load_pending(&db, &ok_pending.id).await;
        assert_eq!(ok.remote_state, Some(RemoteCryptoState::Consumed));
        assert!(!ok.is_active);
        assert!(ok.consumed_at.is_some());
        let ok_audit = load_audit_entry(&db, success_audit).await;
        assert_eq!(ok_audit.ip_address.as_deref(), Some("203.0.113.30"));
        assert_eq!(ok_audit.user_agent.as_deref(), Some("nyxid-node-test"));
        assert_rci_audit_row(
            &ok_audit,
            "node_credential_rci_decrypt_succeeded",
            &ok,
            Some("consumed"),
            &[],
        );

        let failed = load_pending(&db, &failed_pending.id).await;
        assert_eq!(failed.remote_state, Some(RemoteCryptoState::DecryptFailed));
        assert!(!failed.is_active);
        assert!(failed.consumed_at.is_none());
        let failed_audit = load_audit_entry(&db, failed_audit).await;
        assert_eq!(failed_audit.ip_address.as_deref(), Some("203.0.113.31"));
        assert_eq!(failed_audit.user_agent.as_deref(), Some("nyxid-node-test"));
        assert_rci_audit_row(
            &failed_audit,
            "node_credential_rci_decrypt_failed",
            &failed,
            Some("decrypt_failed"),
            &["error_code", "error_kind"],
        );
        let event_data = failed_audit.event_data.as_ref().unwrap();
        assert_eq!(
            event_data["error_code"],
            PENDING_CREDENTIAL_DECRYPT_FAILED_CODE
        );
        assert_eq!(
            event_data["error_kind"],
            "pending_credential_decrypt_failed"
        );
    }

    #[tokio::test]
    async fn ws_status_update_drains_queued_ciphertext_and_marks_sent() {
        let db = test_db("ws_pending_drain_sent").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_drain";
        let node = test_node(&owner_id, "ws-drain-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "queued-drain").await;
        store_ciphertext(&db, &owner_id, &node.id, &pending.id, false).await;
        let queued = load_pending(&db, &pending.id).await;
        assert_eq!(
            queued.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        let replay_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_replayed",
            Some(pending.id.clone()),
        );

        let state = test_app_state(db.clone());
        let (tx, mut rx) = mpsc::channel(1);
        state.node_ws_manager.register_connection(&node.id, tx);

        apply_status_update_capabilities(
            &state,
            &node.id,
            Some(NodeCapabilitiesMsg {
                remote_credential_crypto_v1: true,
                ..NodeCapabilitiesMsg::default()
            }),
        )
        .await;

        let NodeOutboundMessage::Text(json) = rx.try_recv().expect("ciphertext frame queued")
        else {
            panic!("expected text ciphertext frame");
        };
        let frame: Value = serde_json::from_str(&json).expect("ciphertext frame json");
        assert_eq!(frame["type"], "pending_credential_ciphertext");
        assert_eq!(frame["pending_id"], pending.id);
        assert_eq!(frame["version"], "v1");
        assert_eq!(frame["admin_pubkey"], b64url(6, 32));
        assert_eq!(frame["nonce"], b64url(7, 24));
        assert_eq!(
            frame["ciphertext"],
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3, 4])
        );

        let sent = load_pending(&db, &pending.id).await;
        assert_eq!(
            sent.remote_state,
            Some(RemoteCryptoState::CiphertextReceived)
        );
        assert!(sent.ciphertext_queued_at.is_none());
        assert!(sent.ciphertext_expires_at.is_none());

        let audit = load_audit_entry(&db, replay_audit).await;
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_ciphertext_replayed",
            &queued,
            Some("ciphertext_received"),
            &["ciphertext_expires_at", "ciphertext_queued_at", "delivery"],
        );
        assert_eq!(
            audit.event_data.as_ref().unwrap()["delivery"],
            "queued_replay"
        );
    }

    #[tokio::test]
    async fn ws_drain_marks_oversized_stored_queued_ciphertext_failed_and_audits() {
        let db = test_db("ws_pending_drain_oversized_audit").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_drain_oversized";
        let node = test_node(&owner_id, "ws-drain-oversized-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "queued-oversized").await;
        store_ciphertext(&db, &owner_id, &node.id, &pending.id, false).await;
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending.id },
                doc! {
                    "$set": {
                        "crypto.ciphertext": bson::Binary {
                            subtype: bson::spec::BinarySubtype::Generic,
                            bytes: vec![
                                9;
                                node_pending_credential_service::MAX_CIPHERTEXT_SIZE + 1
                            ],
                        },
                    },
                },
            )
            .await
            .expect("force oversized stored ciphertext");
        let too_large_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_ciphertext_too_large",
            Some(pending.id.clone()),
        );

        let state = test_app_state(db.clone());
        let (tx, mut rx) = mpsc::channel(1);
        state.node_ws_manager.register_connection(&node.id, tx);
        apply_status_update_capabilities(
            &state,
            &node.id,
            Some(NodeCapabilitiesMsg {
                remote_credential_crypto_v1: true,
                ..NodeCapabilitiesMsg::default()
            }),
        )
        .await;

        assert!(rx.try_recv().is_err());
        let stored = load_pending(&db, &pending.id).await;
        assert!(!stored.is_active);
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::DecryptFailed));
        let crypto = stored.crypto.as_ref().expect("crypto metadata remains");
        assert!(crypto.admin_pubkey.is_none());
        assert!(crypto.nonce.is_none());
        assert!(crypto.ciphertext.is_none());
        assert!(stored.ciphertext_queued_at.is_none());
        assert!(stored.ciphertext_expires_at.is_none());

        let audit = load_audit_entry(&db, too_large_audit).await;
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_ciphertext_too_large",
            &stored,
            Some("decrypt_failed"),
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
    async fn ws_drain_expires_stale_queued_ciphertext_and_writes_metadata_audit_row() {
        let db = test_db("ws_pending_drain_expired_audit").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_drain_expired";
        let node = test_node(&owner_id, "ws-drain-expired-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "queued-expired").await;
        store_ciphertext(&db, &owner_id, &node.id, &pending.id, false).await;
        let expired_at = Utc::now() - Duration::seconds(1);
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .update_one(
                doc! { "_id": &pending.id },
                doc! {
                    "$set": {
                        "ciphertext_expires_at": bson::DateTime::from_chrono(expired_at),
                    },
                },
            )
            .await
            .expect("force ciphertext expiry");
        let queued = load_pending(&db, &pending.id).await;
        assert_eq!(
            queued.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        assert!(queued.ciphertext_queued_at.is_some());
        assert!(queued.ciphertext_expires_at.is_some());
        let expired_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_expired",
            Some(pending.id.clone()),
        );

        let state = test_app_state(db.clone());
        drain_queued_pending_ciphertexts(&state, &node.id).await;

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(stored.remote_state, Some(RemoteCryptoState::Expired));
        assert!(!stored.is_active);
        assert!(stored.ciphertext_queued_at.is_none());
        assert!(stored.ciphertext_expires_at.is_none());

        let audit = load_audit_entry(&db, expired_audit).await;
        assert_rci_audit_row(
            &audit,
            "node_credential_rci_expired",
            &queued,
            Some("expired"),
            &["ciphertext_expires_at", "ciphertext_queued_at"],
        );
    }

    #[tokio::test]
    async fn drain_queued_pending_ciphertext_send_failure_leaves_row_queued() {
        let db = test_db("ws_pending_drain_send_failure").await;
        let owner_id = uuid::Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_test_drain_failure";
        let node = test_node(&owner_id, "ws-drain-fail-node", raw_auth_token);
        insert_user_and_node(&db, &owner_id, &node).await;
        let pending =
            create_remote_pending_with_pubkey(&db, &owner_id, &node.id, "queued-failure").await;
        store_ciphertext(&db, &owner_id, &node.id, &pending.id, false).await;
        let state = test_app_state(db.clone());
        let (tx, _rx) = mpsc::channel(1);
        state.node_ws_manager.register_connection(&node.id, tx);
        state
            .node_ws_manager
            .send_pending_credentials_available(&node.id)
            .expect("pre-fill writer queue");

        drain_queued_pending_ciphertexts(&state, &node.id).await;

        let stored = load_pending(&db, &pending.id).await;
        assert_eq!(
            stored.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        assert!(stored.ciphertext_queued_at.is_some());
        assert!(stored.ciphertext_expires_at.is_some());
    }

    #[test]
    fn decode_base64_payload_decodes_valid_body() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"hello");
        assert_eq!(
            decode_base64_payload(Some(&encoded), "proxy_response", "req-1"),
            Some(b"hello".to_vec())
        );
    }

    #[test]
    fn decode_base64_payload_rejects_invalid_body() {
        assert_eq!(
            decode_base64_payload(Some("%%%not-base64%%%"), "proxy_response", "req-1"),
            None
        );
    }

    #[test]
    fn decode_binary_stream_frame_extracts_request_id_and_payload() {
        let mut frame = b"123e4567-e89b-12d3-a456-426614174000".to_vec();
        frame.extend_from_slice(b"hello");

        let (request_id, payload) =
            decode_binary_stream_frame(&frame).expect("valid binary stream frame");

        assert_eq!(request_id, "123e4567-e89b-12d3-a456-426614174000");
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn decode_binary_stream_frame_rejects_short_prefix() {
        assert_eq!(
            decode_binary_stream_frame(b"short").unwrap_err(),
            "binary frame too short for request_id prefix"
        );
    }

    #[tokio::test]
    async fn invalid_proxy_response_chunk_closes_stream_with_error() {
        let manager = Arc::new(NodeWsManager::new(30, 100));
        let (tx, mut rx) = mpsc::channel(256);
        manager.register_connection("node-1", tx);

        let manager_clone = manager.clone();
        let responder = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected outbound proxy request");
            };
            let parsed: Value = serde_json::from_str(&msg).expect("valid json");
            let request_id = parsed["request_id"].as_str().expect("request id");

            assert!(manager_clone.deliver_stream_start(
                "node-1",
                request_id,
                200,
                vec![("content-type".to_string(), "text/event-stream".to_string())],
            ));
            handle_proxy_response_chunk(
                &manager_clone,
                "node-1",
                WsProxyResponseChunkMsg {
                    request_id: request_id.to_string(),
                    data: Some("%%%not-base64%%%".to_string()),
                },
            );
        });

        let response = manager
            .send_proxy_request(
                "node-1",
                NodeProxyRequest {
                    request_id: "req-stream-invalid".to_string(),
                    service_id: "svc-1".to_string(),
                    service_slug: "demo".to_string(),
                    base_url: "https://api.example.com".to_string(),
                    method: "GET".to_string(),
                    path: "/stream".to_string(),
                    query: None,
                    headers: vec![],
                    body: None,
                },
                None,
            )
            .await
            .expect("streaming response");

        match response {
            ProxyResponseType::Streaming(mut stream) => {
                assert!(matches!(
                    stream.recv().await,
                    Some(StreamChunk::Start { status: 200, .. })
                ));
                match stream.recv().await {
                    Some(StreamChunk::Error(error)) => {
                        assert_eq!(error, "invalid_base64_payload")
                    }
                    other => panic!("expected stream error, got {other:?}"),
                }
                assert!(stream.recv().await.is_none());
            }
            ProxyResponseType::Complete(_) => panic!("expected streaming response"),
        }

        responder.await.expect("responder task");
    }

    // -----------------------------------------------------------------------
    // decode_base64_payload extended tests
    // -----------------------------------------------------------------------

    #[test]
    fn decode_base64_payload_none_returns_empty_vec() {
        let result = decode_base64_payload(None, "test_type", "req-1");
        assert_eq!(result, Some(Vec::new()));
    }

    #[test]
    fn decode_base64_payload_empty_string() {
        let result = decode_base64_payload(Some(""), "test_type", "req-1");
        assert_eq!(result, Some(Vec::new()));
    }

    #[test]
    fn decode_base64_payload_binary_data() {
        let binary_data: Vec<u8> = vec![0x00, 0x01, 0x02, 0xFF, 0xFE];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&binary_data);
        let result = decode_base64_payload(Some(&encoded), "proxy_response", "req-2");
        assert_eq!(result, Some(binary_data));
    }

    #[test]
    fn decode_base64_payload_large_payload() {
        let data = vec![0xAB_u8; 10_000];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let result = decode_base64_payload(Some(&encoded), "proxy_response", "req-3");
        assert_eq!(result.as_ref().map(|v| v.len()), Some(10_000));
    }

    #[test]
    fn decode_base64_payload_whitespace_is_invalid() {
        // base64 with embedded spaces is technically invalid for STANDARD engine
        let result = decode_base64_payload(Some("aGVs bG8="), "test", "req-4");
        assert_eq!(result, None);
    }

    // -----------------------------------------------------------------------
    // decode_binary_stream_frame extended tests
    // -----------------------------------------------------------------------

    #[test]
    fn decode_binary_stream_frame_empty_payload_after_id() {
        let frame = b"123e4567-e89b-12d3-a456-426614174000";
        let (request_id, payload) = decode_binary_stream_frame(frame).expect("exact 36 bytes");
        assert_eq!(request_id, "123e4567-e89b-12d3-a456-426614174000");
        assert!(payload.is_empty());
    }

    #[test]
    fn decode_binary_stream_frame_empty_input() {
        assert_eq!(
            decode_binary_stream_frame(b"").unwrap_err(),
            "binary frame too short for request_id prefix"
        );
    }

    #[test]
    fn decode_binary_stream_frame_exactly_35_bytes() {
        let data = b"12345678901234567890123456789012345";
        assert_eq!(data.len(), 35);
        assert_eq!(
            decode_binary_stream_frame(data).unwrap_err(),
            "binary frame too short for request_id prefix"
        );
    }

    #[test]
    fn decode_binary_stream_frame_exactly_36_bytes() {
        let data = b"123456789012345678901234567890123456";
        assert_eq!(data.len(), 36);
        let (request_id, payload) = decode_binary_stream_frame(data).unwrap();
        assert_eq!(request_id, "123456789012345678901234567890123456");
        assert!(payload.is_empty());
    }

    #[test]
    fn decode_binary_stream_frame_invalid_utf8_prefix() {
        let mut data = vec![0xFF; 36];
        data.extend_from_slice(b"payload");
        assert_eq!(
            decode_binary_stream_frame(&data).unwrap_err(),
            "binary frame has invalid UTF-8 request_id prefix"
        );
    }

    #[test]
    fn decode_binary_stream_frame_large_payload() {
        let mut frame = b"abcdefgh-ijkl-mnop-qrst-uvwxyz012345".to_vec();
        assert_eq!(frame.len(), 36);
        let payload_data = vec![0xDE_u8; 100_000];
        frame.extend_from_slice(&payload_data);

        let (request_id, payload) = decode_binary_stream_frame(&frame).unwrap();
        assert_eq!(request_id, "abcdefgh-ijkl-mnop-qrst-uvwxyz012345");
        assert_eq!(payload.len(), 100_000);
        assert!(payload.iter().all(|&b| b == 0xDE));
    }

    #[test]
    fn validate_base64url_no_pad_exact_accepts_expected_length() {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8; 32]);
        validate_base64url_no_pad_exact(&encoded, "node_pubkey", 32).expect("valid pubkey");
    }

    #[test]
    fn validate_base64url_no_pad_exact_rejects_padding_and_wrong_length() {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8; 32]);
        assert!(
            validate_base64url_no_pad_exact(&format!("{encoded}="), "node_pubkey", 32)
                .expect_err("padding rejected")
                .contains("without padding")
        );

        let short = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8; 31]);
        assert!(
            validate_base64url_no_pad_exact(&short, "node_pubkey", 32)
                .expect_err("wrong length rejected")
                .contains("32 bytes")
        );
    }

    // -----------------------------------------------------------------------
    // ws_extract_ip tests
    // -----------------------------------------------------------------------

    use super::ws_extract_ip;
    use axum::http::HeaderMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    #[test]
    fn ws_extract_ip_prefers_forwarded_for_over_real_ip_and_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        headers.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 8080);
        assert_eq!(
            ws_extract_ip(&headers, Some(peer)).as_deref(),
            Some("1.2.3.4")
        );
    }

    #[test]
    fn ws_extract_ip_falls_back_to_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "192.168.1.1".parse().unwrap());
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 8080);
        assert_eq!(
            ws_extract_ip(&headers, Some(peer)).as_deref(),
            Some("192.168.1.1")
        );
    }

    #[test]
    fn ws_extract_ip_falls_back_to_peer_addr() {
        let headers = HeaderMap::new();
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 5)), 9090);
        assert_eq!(
            ws_extract_ip(&headers, Some(peer)).as_deref(),
            Some("172.16.0.5")
        );
    }

    #[test]
    fn ws_extract_ip_returns_none_without_anything() {
        let headers = HeaderMap::new();
        assert!(ws_extract_ip(&headers, None).is_none());
    }

    #[test]
    fn ws_extract_ip_ignores_empty_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "".parse().unwrap());
        headers.insert("x-real-ip", "8.8.8.8".parse().unwrap());
        assert_eq!(ws_extract_ip(&headers, None).as_deref(), Some("8.8.8.8"));
    }

    #[test]
    fn ws_extract_ip_ignores_empty_real_ip_falls_to_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "  ".parse().unwrap());
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 3000);
        assert_eq!(
            ws_extract_ip(&headers, Some(peer)).as_deref(),
            Some("127.0.0.1")
        );
    }

    #[test]
    fn ws_extract_ip_with_ipv6_peer() {
        let headers = HeaderMap::new();
        let peer = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 443);
        assert_eq!(ws_extract_ip(&headers, Some(peer)).as_deref(), Some("::1"));
    }

    #[test]
    fn ws_extract_ip_single_forwarded_for_entry() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert_eq!(
            ws_extract_ip(&headers, None).as_deref(),
            Some("203.0.113.50")
        );
    }

    #[test]
    fn ws_extract_ip_trims_real_ip_whitespace() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "  10.0.0.1  ".parse().unwrap());
        assert_eq!(ws_extract_ip(&headers, None).as_deref(), Some("10.0.0.1"));
    }

    // -----------------------------------------------------------------------
    // NodeMessage deserialization tests
    // -----------------------------------------------------------------------

    use super::NodeMessage;

    #[test]
    fn node_message_deserialize_heartbeat_pong() {
        let json = r#"{"type": "heartbeat_pong", "timestamp": "2025-01-01T00:00:00Z"}"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, NodeMessage::HeartbeatPong { .. }));
    }

    #[test]
    fn node_message_deserialize_heartbeat_pong_without_timestamp() {
        let json = r#"{"type": "heartbeat_pong"}"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(
            msg,
            NodeMessage::HeartbeatPong { timestamp: None }
        ));
    }

    #[test]
    fn node_message_deserialize_register() {
        let json = r#"{"type": "register", "token": "nyx_nreg_abc123"}"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        match msg {
            NodeMessage::Register { token, metadata } => {
                assert_eq!(token, "nyx_nreg_abc123");
                assert!(metadata.is_none());
            }
            _ => panic!("expected Register variant"),
        }
    }

    #[test]
    fn node_message_deserialize_auth() {
        let json = r#"{"type": "auth", "node_id": "n-1", "token": "nyx_nauth_xyz"}"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        match msg {
            NodeMessage::Auth { node_id, token } => {
                assert_eq!(node_id, "n-1");
                assert_eq!(token, "nyx_nauth_xyz");
            }
            _ => panic!("expected Auth variant"),
        }
    }

    #[test]
    fn node_message_deserialize_status_update_without_capabilities() {
        let json = r#"{"type": "status_update"}"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        match msg {
            NodeMessage::StatusUpdate { capabilities, .. } => {
                assert!(capabilities.is_none());
            }
            _ => panic!("expected StatusUpdate variant"),
        }
    }

    #[test]
    fn node_message_deserialize_status_update_with_rci_capability() {
        let json = r#"{
            "type": "status_update",
            "capabilities": {
                "credential_ack_correlation": true,
                "remote_credential_crypto_v1": true
            }
        }"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        match msg {
            NodeMessage::StatusUpdate {
                capabilities: Some(capabilities),
                ..
            } => {
                assert!(capabilities.credential_ack_correlation);
                assert!(capabilities.remote_credential_crypto_v1);
            }
            _ => panic!("expected StatusUpdate with capabilities"),
        }
    }

    #[test]
    fn node_message_deserialize_credential_update_ack_minimal() {
        let json = r#"{"type": "credential_update_ack"}"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        match msg {
            NodeMessage::CredentialUpdateAck {
                request_id,
                service_slug,
                status,
                error,
            } => {
                assert!(request_id.is_none());
                assert!(service_slug.is_none());
                assert!(status.is_none());
                assert!(error.is_none());
            }
            _ => panic!("expected CredentialUpdateAck"),
        }
    }

    #[test]
    fn node_message_deserialize_credential_update_ack_full() {
        let json = r#"{
            "type": "credential_update_ack",
            "request_id": "req-42",
            "service_slug": "openai",
            "status": "ok",
            "error": null
        }"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        match msg {
            NodeMessage::CredentialUpdateAck {
                request_id,
                service_slug,
                status,
                error,
            } => {
                assert_eq!(request_id.as_deref(), Some("req-42"));
                assert_eq!(service_slug.as_deref(), Some("openai"));
                assert_eq!(status.as_deref(), Some("ok"));
                assert!(error.is_none());
            }
            _ => panic!("expected CredentialUpdateAck"),
        }
    }

    #[test]
    fn node_message_deserialize_pending_credential_pubkey() {
        let node_pubkey = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1_u8; 32]);
        let json = serde_json::json!({
            "type": "pending_credential_pubkey",
            "pending_id": "pending-1",
            "version": "v1",
            "node_pubkey": node_pubkey,
        });
        let msg: NodeMessage = serde_json::from_value(json).unwrap();
        match msg {
            NodeMessage::PendingCredentialPubkey {
                pending_id,
                version,
                node_pubkey: parsed,
            } => {
                assert_eq!(pending_id, "pending-1");
                assert_eq!(version, "v1");
                assert_eq!(parsed, node_pubkey);
            }
            _ => panic!("expected PendingCredentialPubkey"),
        }
    }

    #[test]
    fn node_message_deserialize_pending_credential_decrypt_result() {
        let json = r#"{
            "type": "pending_credential_decrypt_result",
            "pending_id": "pending-1",
            "status": "error",
            "error_code": "decrypt_failed"
        }"#;
        let msg: NodeMessage = serde_json::from_str(json).unwrap();
        match msg {
            NodeMessage::PendingCredentialDecryptResult {
                pending_id,
                status,
                error_code,
            } => {
                assert_eq!(pending_id, "pending-1");
                assert_eq!(status, "error");
                assert_eq!(error_code, Some(serde_json::json!("decrypt_failed")));
            }
            _ => panic!("expected PendingCredentialDecryptResult"),
        }
    }

    #[test]
    fn node_message_deserialize_unknown_type_fails() {
        let json = r#"{"type": "nonexistent_type"}"#;
        let result = serde_json::from_str::<NodeMessage>(json);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // WS_WRITER_CHANNEL_SIZE constant test
    // -----------------------------------------------------------------------

    use super::WS_WRITER_CHANNEL_SIZE;

    #[test]
    fn ws_writer_channel_size_is_256() {
        assert_eq!(WS_WRITER_CHANNEL_SIZE, 256);
    }
}
