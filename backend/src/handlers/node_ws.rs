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
        NodeOutboundMessage, NodeProxyResponse, NodeWsManager, WsProxyResponseChunkMsg,
        WsProxyResponseEndMsg, WsProxyResponseStartMsg, WsSshTunnelClosedMsg, WsSshTunnelDataMsg,
        WsSshTunnelOpenedMsg,
    },
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
    },
    #[serde(rename = "ssh_tunnel_opened")]
    SshTunnelOpened(WsSshTunnelOpenedMsg),
    #[serde(rename = "ssh_tunnel_data")]
    SshTunnelData(WsSshTunnelDataMsg),
    #[serde(rename = "ssh_tunnel_closed")]
    SshTunnelClosed(WsSshTunnelClosedMsg),
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
                            return Some(node.id);
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
                            });
                            let _ = ws_sink.send(Message::Text(ok_msg.to_string().into())).await;
                            return Some(node.id);
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

    let node_id = match auth_result {
        Ok(Some(id)) => id,
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

    while let Some(msg) = ws_stream.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => continue,
            Ok(_) => continue,
            Err(e) => {
                tracing::debug!(node_id = %node_id_reader, error = %e, "WebSocket read error");
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
                let body = resp.body.as_deref().map_or_else(Vec::new, |b| {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD
                        .decode(b)
                        .unwrap_or_else(|_| b.as_bytes().to_vec())
                });

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
                if let Some(data) = decode_base64_payload(
                    chunk.data.as_deref(),
                    "proxy_response_chunk",
                    &chunk.request_id,
                ) {
                    ws_manager.deliver_stream_chunk(&node_id_reader, &chunk.request_id, data);
                }
            }
            NodeMessage::ProxyResponseEnd(end) => {
                ws_manager.deliver_stream_end(&node_id_reader, &end.request_id);
            }
            NodeMessage::StatusUpdate { .. } => {
                // Future: update node metadata / ready services
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
