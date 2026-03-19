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
use tokio::io::AsyncReadExt;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, ssh_service};

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

    // ----- Write ephemeral SSH key + certificate to temp files -----
    let temp_dir = match write_ephemeral_ssh_files(
        &state,
        &ssh_svc,
        &service_id,
        &user_id,
        &principal,
    )
    .await
    {
        Ok(dir) => dir,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "Web terminal: credential gen failed");
            send_error_and_close(&mut socket, "Failed to generate SSH credentials").await;
            return;
        }
    };

    let key_path = temp_dir.path().join("id_ed25519");
    let cert_path = temp_dir.path().join("id_ed25519-cert.pub");

    // ----- Create PTY and spawn ssh with pty-process -----
    // pty-process calls setsid() + TIOCSCTTY in pre_exec, giving ssh
    // a proper controlling terminal for PTY negotiation.
    let (pty, pts) = match pty_process::open() {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!(error = %error, "Web terminal: PTY open failed");
            send_error_and_close(&mut socket, "Failed to create PTY").await;
            return;
        }
    };

    if let Err(error) = pty.resize(pty_process::Size::new(rows as u16, cols as u16)) {
        tracing::warn!(error = %error, "Web terminal: PTY resize failed");
    }

    // Don't request a remote PTY from sshd (it rejects it on some macOS
    // configurations). Instead, run `script` on the remote side to create
    // a PTY there, giving us an interactive shell with prompt.
    let remote_cmd = "TERM=xterm-256color script -q /dev/null $SHELL -il 2>/dev/null || TERM=xterm-256color exec $SHELL -il";

    // Check if this service is node-routed. If so, use ProxyCommand to
    // tunnel through the node agent via the existing NyxID WebSocket tunnel.
    let node_route = crate::services::node_routing_service::resolve_node_route(
        &state.db,
        &user_id,
        &service_id,
        &state.node_ws_manager,
    )
    .await
    .ok()
    .flatten();

    let nyxid_binary =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("nyxid"));
    let base_url = state.config.base_url.trim_end_matches('/');

    let mut cmd = pty_process::Command::new("ssh")
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("UserKnownHostsFile=/dev/null")
        .arg("-o")
        .arg(format!("IdentityFile={}", key_path.display()))
        .arg("-o")
        .arg(format!("CertificateFile={}", cert_path.display()))
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg("RequestTTY=no")
        .arg("-o")
        .arg("LogLevel=FATAL");

    // If node-routed, add ProxyCommand to tunnel through the node agent.
    // Generate a short-lived access token for the ProxyCommand to authenticate.
    if node_route.is_some() {
        let proxy_token = match crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &auth_user.user_id,
            "openid",
            None,
        ) {
            Ok(token) => token,
            Err(error) => {
                tracing::warn!(error = %error, "Web terminal: failed to generate proxy token");
                send_error_and_close(&mut socket, "Failed to generate proxy credentials").await;
                return;
            }
        };

        let proxy_cmd = format!(
            "{} ssh proxy --base-url {} --service-id {}",
            nyxid_binary.display(),
            base_url,
            service_id,
        );
        cmd = cmd
            .arg("-o")
            .arg(format!("ProxyCommand={proxy_cmd}"))
            .env("NYXID_ACCESS_TOKEN", &proxy_token);
        tracing::info!(service_id = %service_id, "Web terminal: using node-routed ProxyCommand");
    }

    let cmd = cmd
        .arg("-p")
        .arg(ssh_svc.port.to_string())
        .arg(format!("{principal}@{}", ssh_svc.host))
        .arg(remote_cmd)
        .env("TERM", "xterm-256color");

    let mut child = match cmd.spawn(pts) {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "Web terminal: ssh spawn failed");
            send_error_and_close(&mut socket, "Failed to start SSH").await;
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
    };

    // Set PTY master to raw mode so control characters (Ctrl+C, Ctrl+Z, etc.)
    // and special keys (for top, vim, etc.) are passed through to the remote
    // shell instead of being intercepted locally.
    {
        use std::os::fd::AsFd;
        if let Ok(mut termios) = nix::sys::termios::tcgetattr(pty.as_fd()) {
            nix::sys::termios::cfmakeraw(&mut termios);
            let _ = nix::sys::termios::tcsetattr(
                pty.as_fd(),
                nix::sys::termios::SetArg::TCSANOW,
                &termios,
            );
        }
    }

    tracing::info!(service_id = %service_id, "Web terminal: ssh spawned with pty-process");

    // ----- Send connected -----
    if socket
        .send(Message::Text(server_connected_msg(cols, rows).into()))
        .await
        .is_err()
    {
        let _ = child.kill().await;
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
        })),
        ip_address.clone(),
        user_agent.clone(),
    );

    // ----- Bridge loop: PTY master <-> WebSocket -----
    let (mut pty_reader, mut pty_writer) = pty.into_split();

    let idle_timeout = Duration::from_secs(DEFAULT_WEB_TERMINAL_IDLE_TIMEOUT_SECS);
    let max_duration = Duration::from_secs(state.config.ssh_max_tunnel_duration_secs);
    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let mut pty_buf = vec![0u8; 8192];

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
                        if tokio::io::AsyncWriteExt::write_all(&mut pty_writer, &data).await.is_err() {
                            break "pty_write_failed";
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ClientControl::Resize { cols: c, rows: r }) =
                            serde_json::from_str::<ClientControl>(&text)
                        {
                            let _ = pty_writer.resize(
                                pty_process::Size::new(r.clamp(2, 200) as u16, c.clamp(10, 500) as u16),
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
            n = pty_reader.read(&mut pty_buf) => {
                idle_timer.as_mut().reset(tokio::time::Instant::now() + idle_timeout);
                match n {
                    Ok(0) => break "pty_eof",
                    Ok(n) => {
                        to_client_bytes += n as u64;
                        if socket.send(Message::Binary(pty_buf[..n].to_vec().into())).await.is_err() {
                            break "client_write_failed";
                        }
                    }
                    Err(e) if e.raw_os_error() == Some(5) => break "pty_closed",
                    Err(_) => break "pty_read_error",
                }
            }
        }
    };

    let _ = child.kill().await;
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

async fn write_ephemeral_ssh_files(
    state: &AppState,
    ssh_svc: &crate::models::downstream_service::SshServiceConfig,
    service_id: &str,
    user_id: &str,
    principal: &str,
) -> AppResult<tempfile::TempDir> {
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

    let temp_dir = tempfile::tempdir()
        .map_err(|e| AppError::Internal(format!("Failed to create temp dir: {e}")))?;

    let key_path = temp_dir.path().join("id_ed25519");
    let cert_path = temp_dir.path().join("id_ed25519-cert.pub");

    tokio::fs::write(&key_path, private_key_openssh.as_bytes())
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write key: {e}")))?;
    tokio::fs::write(&cert_path, issued.certificate.as_bytes())
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write cert: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|e| AppError::Internal(format!("Failed to set key permissions: {e}")))?;
    }

    Ok(temp_dir)
}

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
