use axum::{
    extract::State,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::AppState;
use crate::models::node::{NodeMetadata, NodeStatus};
use crate::services::{
    audit_service, node_service,
    node_ws_manager::{
        NodeCapabilitiesMsg, NodeOutboundMessage, NodeProxyResponse, NodeSshExecResult,
        NodeWsManager, WsFrameInjectedInbound, WsProxyBinaryInbound, WsProxyClosedInbound,
        WsProxyErrorInbound, WsProxyOpenedInbound, WsProxyResponseChunkMsg, WsProxyResponseEndMsg,
        WsProxyResponseStartMsg, WsProxyTextInbound, WsSshExecResultMsg, WsSshNodeExecCloseMsg,
        WsSshNodeExecDataMsg, WsSshNodeExecErrorMsg, WsSshTunnelClosedMsg, WsSshTunnelDataMsg,
        WsSshTunnelOpenedMsg, WsWebTerminalClosedMsg, WsWebTerminalDataMsg,
        WsWebTerminalStartedMsg,
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

/// GET /api/v1/nodes/ws
///
/// WebSocket upgrade handler for node agent connections.
/// Authentication happens in the first message (register or auth).
/// If no valid auth message within 10 seconds, connection is closed.
///
/// Security: The global rate limiter applies to the HTTP upgrade request.
/// Additionally, a max concurrent connections limit is enforced here.
/// Auth tokens should only be transmitted over TLS/WSS in production.
pub async fn ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
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

    ws.on_upgrade(|socket| handle_node_connection(state, socket, guard))
        .into_response()
}

async fn handle_node_connection(state: AppState, socket: WebSocket, _guard: PendingAuthGuard) {
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
                        None,
                        None,
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
                                None,
                                None,
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
                                None,
                                None,
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
                                None,
                                None,
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
                        None,
                        None,
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
                if let Some(caps) = capabilities {
                    ws_manager.record_capabilities(&node_id_reader, &caps);
                }
                // Always mark capability state "resolved" on any
                // status_update, regardless of whether `capabilities`
                // was present. This releases strict-push waiters
                // parked in `await_capability_resolution` — the
                // flag's value (present vs. absent) is what they
                // want to observe, and it's now final for this
                // connection (twenty-ninth-round Codex P2).
                ws_manager.mark_status_update_received(&node_id_reader);
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
    use super::{decode_base64_payload, decode_binary_stream_frame, handle_proxy_response_chunk};
    use base64::Engine;
    use serde_json::Value;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    use crate::services::node_ws_manager::{
        NodeOutboundMessage, NodeProxyRequest, NodeWsManager, ProxyResponseType, StreamChunk,
        WsProxyResponseChunkMsg,
    };

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
}
