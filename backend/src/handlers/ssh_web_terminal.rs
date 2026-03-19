use std::sync::Arc;
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
use crate::services::{audit_service, ssh_service};

use super::ssh_tunnel::authorize_ssh_access;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Control messages (JSON over text frames)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// russh client handler
// ---------------------------------------------------------------------------

struct SshClientHandler;

impl russh::client::Handler for SshClientHandler {
    type Error = russh::Error;

    #[allow(clippy::manual_async_fn)]
    fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        // TODO(TOFU): verify against known_hosts / per-service fingerprint
        async { Ok(true) }
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_WEB_TERMINAL_IDLE_TIMEOUT_SECS: u64 = 1800;

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

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
            "Web terminal requires SSH certificate auth to be enabled for this service".to_string(),
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

    ssh_service::validate_resolved_ssh_target(&ssh_svc.host, ssh_svc.port).await?;

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

// ---------------------------------------------------------------------------
// Session loop
// ---------------------------------------------------------------------------

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

    // ----- Generate ephemeral key + certificate -----
    let (ephemeral_key_pem, cert_openssh) =
        match generate_ephemeral_cert(&state, &ssh_svc, &service_id, &user_id, &principal).await {
            Ok(pair) => pair,
            Err(error) => {
                tracing::warn!(
                    service_id = %service_id,
                    error = %error,
                    "Web terminal: failed to generate ephemeral credentials"
                );
                send_error_and_close(&mut socket, "Failed to generate SSH credentials").await;
                return;
            }
        };

    // Parse into russh types (russh uses its own forked ssh_key internally)
    let russh_key = match russh::keys::PrivateKey::from_openssh(&ephemeral_key_pem) {
        Ok(key) => Arc::new(key),
        Err(error) => {
            tracing::warn!(error = %error, "Web terminal: failed to parse ephemeral key for russh");
            send_error_and_close(&mut socket, "Internal SSH key error").await;
            return;
        }
    };
    let russh_cert = match russh::keys::Certificate::from_openssh(&cert_openssh) {
        Ok(cert) => cert,
        Err(error) => {
            tracing::warn!(error = %error, "Web terminal: failed to parse certificate for russh");
            send_error_and_close(&mut socket, "Internal SSH certificate error").await;
            return;
        }
    };

    // ----- Connect to SSH target -----
    let ssh_config = Arc::new(russh::client::Config {
        inactivity_timeout: Some(Duration::from_secs(DEFAULT_WEB_TERMINAL_IDLE_TIMEOUT_SECS)),
        ..Default::default()
    });

    let connect_target = (ssh_svc.host.as_str(), ssh_svc.port);
    let mut ssh_handle = match tokio::time::timeout(
        Duration::from_secs(state.config.ssh_connect_timeout_secs),
        russh::client::connect(ssh_config, connect_target, SshClientHandler),
    )
    .await
    {
        Ok(Ok(handle)) => handle,
        Ok(Err(error)) => {
            tracing::warn!(service_id = %service_id, error = %error, "Web terminal: SSH connect failed");
            send_error_and_close(&mut socket, "Failed to connect to SSH target").await;
            log_failed(
                &state,
                &user_id,
                &service_id,
                &session_id,
                &ssh_svc,
                &error.to_string(),
                &ip_address,
                &user_agent,
            );
            return;
        }
        Err(_) => {
            tracing::warn!(service_id = %service_id, "Web terminal: SSH connect timed out");
            send_error_and_close(&mut socket, "SSH target connect timed out").await;
            log_failed(
                &state,
                &user_id,
                &service_id,
                &session_id,
                &ssh_svc,
                "connect_timeout",
                &ip_address,
                &user_agent,
            );
            return;
        }
    };

    // ----- Authenticate with certificate -----
    match ssh_handle
        .authenticate_openssh_cert(&principal, russh_key, russh_cert)
        .await
    {
        Ok(result) if result.success() => {}
        Ok(_) => {
            tracing::warn!(service_id = %service_id, "Web terminal: cert auth rejected");
            send_error_and_close(
                &mut socket,
                "SSH authentication failed -- ensure the target trusts the NyxID CA",
            )
            .await;
            log_failed(
                &state,
                &user_id,
                &service_id,
                &session_id,
                &ssh_svc,
                "auth_rejected",
                &ip_address,
                &user_agent,
            );
            return;
        }
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "Web terminal: cert auth error");
            send_error_and_close(&mut socket, "SSH authentication failed").await;
            log_failed(
                &state,
                &user_id,
                &service_id,
                &session_id,
                &ssh_svc,
                &error.to_string(),
                &ip_address,
                &user_agent,
            );
            return;
        }
    }

    // ----- Open channel, PTY, shell -----
    let channel = match ssh_handle.channel_open_session().await {
        Ok(ch) => ch,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "Web terminal: channel open failed");
            send_error_and_close(&mut socket, "Failed to open SSH session").await;
            return;
        }
    };

    if let Err(error) = channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
    {
        tracing::warn!(service_id = %service_id, error = %error, "Web terminal: PTY request failed");
        send_error_and_close(&mut socket, "Failed to allocate PTY").await;
        return;
    }

    if let Err(error) = channel.request_shell(false).await {
        tracing::warn!(service_id = %service_id, error = %error, "Web terminal: shell request failed");
        send_error_and_close(&mut socket, "Failed to start shell").await;
        return;
    }

    // ----- Send connected -----
    if socket
        .send(Message::Text(server_connected_msg(cols, rows).into()))
        .await
        .is_err()
    {
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
            "cols": cols,
            "rows": rows,
        })),
        ip_address.clone(),
        user_agent.clone(),
    );

    // ----- Bridge loop using split() -----
    // channel.wait() doesn't reliably pump interactive PTY data in russh 0.58.
    // split() gives us a read half (with make_reader for AsyncRead) and a
    // write half (with data() for writing and window_change() for resize).
    let (mut read_half, write_half) = channel.split();
    let mut ssh_reader = read_half.make_reader();

    let idle_timeout = Duration::from_secs(DEFAULT_WEB_TERMINAL_IDLE_TIMEOUT_SECS);
    let max_duration = Duration::from_secs(state.config.ssh_max_tunnel_duration_secs);
    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let mut ssh_buf = vec![0u8; 8192];

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
                        if write_half.data(&data[..]).await.is_err() {
                            break "ssh_write_failed";
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ClientControl::Resize { cols: c, rows: r }) =
                            serde_json::from_str::<ClientControl>(&text)
                        {
                            let _ = write_half
                                .window_change(c.clamp(10, 500), r.clamp(2, 200), 0, 0)
                                .await;
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
            n = tokio::io::AsyncReadExt::read(&mut ssh_reader, &mut ssh_buf) => {
                idle_timer.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                match n {
                    Ok(0) => break "ssh_eof",
                    Ok(n) => {
                        to_client_bytes += n as u64;
                        if socket.send(Message::Binary(ssh_buf[..n].to_vec().into())).await.is_err() {
                            break "client_write_failed";
                        }
                    }
                    Err(_) => break "ssh_read_error",
                }
            }
        }
    };

    let _ = ssh_handle
        .disconnect(russh::Disconnect::ByApplication, "", "English")
        .await;
    let _ = socket.close().await;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_web_terminal_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "principal": principal,
            "duration_ms": started_at.elapsed().as_millis() as u64,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
            "disconnect_reason": disconnect_reason,
        })),
        ip_address,
        user_agent,
    );
}

// ---------------------------------------------------------------------------
// Ephemeral key + certificate
// ---------------------------------------------------------------------------

async fn generate_ephemeral_cert(
    state: &AppState,
    ssh_svc: &crate::models::downstream_service::SshServiceConfig,
    service_id: &str,
    user_id: &str,
    principal: &str,
) -> AppResult<(String, String)> {
    let mut rng = rand::rngs::OsRng;
    let ephemeral_key = ssh_key::PrivateKey::random(&mut rng, ssh_key::Algorithm::Ed25519)
        .map_err(|e| AppError::Internal(format!("Failed to generate ephemeral key: {e}")))?;

    let public_key_openssh = ephemeral_key
        .public_key()
        .to_openssh()
        .map_err(|e| AppError::Internal(format!("Failed to encode ephemeral public key: {e}")))?;

    let private_key_openssh = ephemeral_key
        .to_openssh(ssh_key::LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("Failed to encode ephemeral private key: {e}")))?;

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

    Ok((private_key_openssh.to_string(), issued.certificate))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn send_error_and_close(socket: &mut WebSocket, message: &str) {
    let _ = socket
        .send(Message::Text(server_error_msg(message).into()))
        .await;
    let _ = socket.close().await;
}

#[allow(clippy::too_many_arguments)]
fn log_failed(
    state: &AppState,
    user_id: &str,
    service_id: &str,
    session_id: &str,
    ssh_svc: &crate::models::downstream_service::SshServiceConfig,
    error: &str,
    ip_address: &Option<String>,
    user_agent: &Option<String>,
) {
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.to_string()),
        "ssh_web_terminal_connect_failed".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "target_host": ssh_svc.host,
            "target_port": ssh_svc.port,
            "error": error,
        })),
        ip_address.clone(),
        user_agent.clone(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_connected_msg_is_valid_json() {
        let msg = server_connected_msg(120, 40);
        let parsed: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
        assert_eq!(parsed["type"], "connected");
        assert_eq!(parsed["cols"], 120);
        assert_eq!(parsed["rows"], 40);
    }

    #[test]
    fn server_error_msg_is_valid_json() {
        let msg = server_error_msg("something broke");
        let parsed: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "something broke");
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
