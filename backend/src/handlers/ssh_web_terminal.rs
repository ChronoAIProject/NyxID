use std::time::{Duration, Instant};

use axum::{
    extract::{
        ConnectInfo, Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use serde::Deserialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, node_routing_service, node_service, ssh_service};

use super::ssh_tunnel::authorize_ssh_access;

#[derive(Debug, Deserialize)]
pub struct WebTerminalQuery {
    pub principal: String,
    #[serde(default = "default_cols")]
    pub cols: u32,
    #[serde(default = "default_rows")]
    pub rows: u32,
}

fn default_cols() -> u32 {
    80
}
fn default_rows() -> u32 {
    24
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientControl {
    #[serde(rename = "resize")]
    Resize { cols: u32, rows: u32 },
}

fn server_connected_msg(cols: u32, rows: u32) -> String {
    serde_json::json!({ "type": "connected", "cols": cols, "rows": rows }).to_string()
}

fn server_error_msg(message: &str) -> String {
    serde_json::json!({ "type": "error", "message": message }).to_string()
}

const DEFAULT_WEB_TERMINAL_IDLE_TIMEOUT_SECS: u64 = 1800;

pub async fn ssh_web_terminal(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Query(query): Query<WebTerminalQuery>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    authorize_ssh_access(&state, &auth_user, &service_id).await?;
    let ssh_svc = ssh_service::get_ssh_service(&state.db, &service_id).await?;

    if !ssh_svc.certificate_auth_enabled {
        return Err(AppError::BadRequest(
            "Web terminal requires SSH certificate auth to be enabled".to_string(),
        ));
    }
    if ssh_svc.allowed_principals.is_empty() {
        return Err(AppError::BadRequest(
            "Web terminal requires at least one allowed principal".to_string(),
        ));
    }

    let principal = query.principal.trim().to_string();
    ssh_service::validate_principal(&principal)?;
    if !ssh_svc.allowed_principals.iter().any(|p| p == &principal) {
        return Err(AppError::Forbidden(
            "Requested SSH principal is not allowed for this service".to_string(),
        ));
    }

    let session_guard = state
        .ssh_session_manager
        .try_acquire(&auth_user.user_id.to_string())?;

    let client_meta = (
        Some(addr.ip().to_string()),
        headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
    );

    let cols = query.cols.clamp(10, 500);
    let rows = query.rows.clamp(2, 200);

    Ok(ws
        .on_upgrade(move |socket| async move {
            handle_web_terminal(
                state,
                auth_user,
                service_id,
                ssh_svc,
                principal,
                cols,
                rows,
                socket,
                session_guard,
                client_meta,
            )
            .await;
        })
        .into_response())
}

#[allow(clippy::too_many_arguments)]
async fn handle_web_terminal(
    state: AppState,
    auth_user: AuthUser,
    service_id: String,
    ssh_svc: crate::models::downstream_service::SshServiceConfig,
    principal: String,
    cols: u32,
    rows: u32,
    mut socket: WebSocket,
    session_guard: ssh_service::SshSessionGuard,
    client_meta: (Option<String>, Option<String>),
) {
    let _ = &session_guard;
    let user_id = auth_user.user_id.to_string();
    let session_id = uuid::Uuid::new_v4().to_string();
    let started_at = Instant::now();
    let (ip_address, user_agent) = client_meta;

    // Guard against React Strict Mode double-mount
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if socket.send(Message::Ping(vec![].into())).await.is_err() {
            return;
        }
    }

    // ----- Generate ephemeral SSH credentials -----
    let ephemeral = match generate_ephemeral_credentials(
        &state,
        &ssh_svc,
        &service_id,
        &user_id,
        &principal,
    )
    .await
    {
        Ok(creds) => creds,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "Web terminal: credential gen failed");
            send_error_and_close(&mut socket, "Failed to generate SSH credentials").await;
            return;
        }
    };

    // ----- Check for node route -----
    let node_route = node_routing_service::resolve_node_route(
        &state.db,
        &user_id,
        &service_id,
        &state.node_ws_manager,
    )
    .await
    .ok()
    .flatten();

    if let Some(node_route) = node_route {
        handle_node_web_terminal(
            state, service_id, ssh_svc, principal, cols, rows, socket, user_id, session_id,
            started_at, ip_address, user_agent, node_route, ephemeral,
        )
        .await;
    } else {
        tracing::warn!(
            service_id = %service_id,
            "Web terminal: no node agent bound to this service"
        );
        send_error_and_close(
            &mut socket,
            "No node agent is bound to this SSH service. \
             Deploy a NyxID node agent on the target's network and bind it to this service.",
        )
        .await;
    }
}

/// Ephemeral SSH credentials (key + certificate) for node-routed SSH sessions.
/// Sent to the node agent as strings -- no files written to the NyxID server.
pub(crate) struct EphemeralSshCredentials {
    pub(crate) private_key_pem: String,
    pub(crate) certificate_openssh: String,
}

// ---- Node-routed web terminal ----

#[allow(clippy::too_many_arguments)]
async fn handle_node_web_terminal(
    state: AppState,
    service_id: String,
    ssh_svc: crate::models::downstream_service::SshServiceConfig,
    principal: String,
    cols: u32,
    rows: u32,
    mut socket: WebSocket,
    user_id: String,
    session_id: String,
    started_at: Instant,
    ip_address: Option<String>,
    user_agent: Option<String>,
    node_route: node_routing_service::NodeRoute,
    ephemeral: EphemeralSshCredentials,
) {
    let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
        .chain(node_route.fallback_node_ids.iter().map(|id| id.as_str()))
        .collect();

    let mut terminal_rx = None;
    let mut selected_node_id = None;

    for node_id in all_node_ids {
        let signing_secret = if state.config.node_hmac_signing_enabled {
            match node_service::get_node_signing_secret(
                &state.db,
                state.encryption_keys.as_ref(),
                node_id,
            )
            .await
            {
                Ok(secret) => Some(secret),
                Err(error) => {
                    tracing::warn!(
                        service_id = %service_id,
                        node_id = %node_id,
                        error = %error,
                        "Web terminal node signing secret resolution failed"
                    );
                    continue;
                }
            }
        } else {
            None
        };

        match state
            .node_ws_manager
            .open_web_terminal(
                node_id,
                crate::services::node_ws_manager::NodeWebTerminalRequest {
                    session_id: session_id.clone(),
                    service_id: service_id.clone(),
                    host: ssh_svc.host.clone(),
                    port: ssh_svc.port,
                    principal: principal.clone(),
                    private_key_pem: ephemeral.private_key_pem.clone(),
                    certificate_openssh: ephemeral.certificate_openssh.clone(),
                    cols,
                    rows,
                },
                signing_secret.as_ref().map(|s| s.as_slice()),
            )
            .await
        {
            Ok(rx) => {
                terminal_rx = Some(rx);
                selected_node_id = Some(node_id.to_string());
                break;
            }
            Err(error) => {
                tracing::warn!(
                    service_id = %service_id,
                    node_id = %node_id,
                    error = %error,
                    "Web terminal node open failed, trying next"
                );
            }
        }
    }

    let Some(mut terminal_rx) = terminal_rx else {
        tracing::warn!(
            service_id = %service_id,
            session_id = %session_id,
            "Web terminal: all node attempts failed"
        );
        send_error_and_close(
            &mut socket,
            "Failed to connect through the node agent. Ensure the node agent is running and bound to this service.",
        )
        .await;
        return;
    };

    let node_id = selected_node_id.expect("selected_node_id is set when terminal_rx is Some");
    tracing::info!(
        service_id = %service_id,
        node_id = %node_id,
        session_id = %session_id,
        "Web terminal: session opened via node"
    );

    // Send connected message to browser
    if socket
        .send(Message::Text(server_connected_msg(cols, rows).into()))
        .await
        .is_err()
    {
        close_node_web_terminal(&state, &node_id, &session_id, "browser_send_failed");
        return;
    }

    audit_service::log_async(
        state.db.clone(),
        Some(user_id.clone()),
        "ssh_web_terminal_connected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "principal": principal,
            "target_host": ssh_svc.host,
            "target_port": ssh_svc.port,
            "routed_via": "node",
            "node_id": node_id,
        })),
        ip_address.clone(),
        user_agent.clone(),
    );

    // ----- Bridge loop: browser WebSocket <-> node agent -----
    let idle_timeout = Duration::from_secs(DEFAULT_WEB_TERMINAL_IDLE_TIMEOUT_SECS);
    let max_duration = Duration::from_secs(state.config.ssh_max_tunnel_duration_secs);
    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;

    let max_timer = tokio::time::sleep(max_duration);
    tokio::pin!(max_timer);
    let idle_timer = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_timer);

    let disconnect_reason = loop {
        tokio::select! {
            _ = &mut max_timer => {
                let _ = socket.send(Message::Text(server_error_msg("Session reached maximum duration").into())).await;
                break "max_duration_exceeded";
            }
            _ = &mut idle_timer => {
                let _ = socket.send(Message::Text(server_error_msg("Session timed out due to inactivity").into())).await;
                break "idle_timeout";
            }
            ws_msg = socket.next() => {
                idle_timer.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                match ws_msg {
                    Some(Ok(Message::Binary(data))) => {
                        from_client_bytes += data.len() as u64;
                        if state.node_ws_manager.send_web_terminal_data(&node_id, &session_id, &data).is_err() {
                            break "node_terminal_send_failed";
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ClientControl::Resize { cols: c, rows: r }) =
                            serde_json::from_str::<ClientControl>(&text)
                        {
                            let _ = state.node_ws_manager.send_web_terminal_resize(
                                &node_id,
                                &session_id,
                                c.clamp(10, 500),
                                r.clamp(2, 200),
                            );
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break "client_write_failed";
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break "client_closed",
                    Some(Err(_)) => break "client_error",
                }
            }
            terminal_msg = terminal_rx.recv() => {
                idle_timer.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                match terminal_msg {
                    Some(crate::services::node_ws_manager::WebTerminalChunk::Data(bytes)) => {
                        to_client_bytes += bytes.len() as u64;
                        if socket.send(Message::Binary(bytes.into())).await.is_err() {
                            break "client_write_failed";
                        }
                    }
                    Some(crate::services::node_ws_manager::WebTerminalChunk::Closed(error)) => {
                        break if error.is_some() {
                            "node_terminal_closed_with_error"
                        } else {
                            "node_terminal_closed"
                        };
                    }
                    None => break "node_terminal_channel_closed",
                }
            }
        }
    };

    close_node_web_terminal(&state, &node_id, &session_id, "session_cleanup");
    let _ = socket.close().await;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_web_terminal_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "principal": principal,
            "routed_via": "node",
            "node_id": node_id,
            "duration_ms": started_at.elapsed().as_millis() as u64,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
            "disconnect_reason": disconnect_reason,
        })),
        ip_address,
        user_agent,
    );
}

fn close_node_web_terminal(state: &AppState, node_id: &str, session_id: &str, reason: &str) {
    if let Err(error) = state
        .node_ws_manager
        .close_web_terminal(node_id, session_id)
    {
        tracing::warn!(
            node_id,
            session_id,
            reason,
            error = %error,
            "Failed to close node-routed web terminal"
        );
    }
}

// ---- Helpers ----

/// Generate an ephemeral SSH key pair and certificate for a node-routed SSH session.
/// Returns the PEM-encoded private key and OpenSSH certificate strings.
pub(crate) async fn generate_ephemeral_credentials(
    state: &AppState,
    ssh_svc: &crate::models::downstream_service::SshServiceConfig,
    service_id: &str,
    user_id: &str,
    principal: &str,
) -> AppResult<EphemeralSshCredentials> {
    let mut rng = rand::rngs::OsRng;
    let ephemeral_key = ssh_key::PrivateKey::random(&mut rng, ssh_key::Algorithm::Ed25519)
        .map_err(|e| AppError::Internal(format!("Failed to generate ephemeral key: {e}")))?;

    let public_key_openssh = ephemeral_key
        .public_key()
        .to_openssh()
        .map_err(|e| AppError::Internal(format!("Failed to encode public key: {e}")))?;

    let private_key_openssh = ephemeral_key
        .to_openssh(ssh_key::LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("Failed to encode private key: {e}")))?;

    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let issued = ssh_service::issue_certificate(
        &state.encryption_keys,
        ssh_svc,
        service_id,
        user_id,
        &user.email,
        &public_key_openssh,
        principal,
    )
    .await?;

    Ok(EphemeralSshCredentials {
        private_key_pem: private_key_openssh.to_string(),
        certificate_openssh: issued.certificate,
    })
}

async fn send_error_and_close(socket: &mut WebSocket, message: &str) {
    let _ = socket
        .send(Message::Text(server_error_msg(message).into()))
        .await;
    let _ = socket.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_connected_msg_is_valid_json() {
        let msg = server_connected_msg(120, 40);
        let parsed: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
        assert_eq!(parsed["type"], "connected");
    }

    #[test]
    fn server_error_msg_is_valid_json() {
        let msg = server_error_msg("broke");
        let parsed: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
        assert_eq!(parsed["type"], "error");
    }

    #[test]
    fn client_resize_deserializes() {
        let json = r#"{"type":"resize","cols":120,"rows":40}"#;
        let msg: ClientControl = serde_json::from_str(json).expect("valid resize");
        match msg {
            ClientControl::Resize { cols, rows } => {
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
            }
        }
    }
}
