use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{
        ConnectInfo, Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    approval_service, audit_service, node_routing_service, notification_service, ssh_service,
};

use super::services_helpers::fetch_service;

#[derive(Debug, Deserialize, ToSchema)]
pub struct IssueSshCertificateRequest {
    pub public_key: String,
    pub principal: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssueSshCertificateResponse {
    pub service_id: String,
    pub key_id: String,
    pub principal: String,
    pub certificate: String,
    pub ca_public_key: String,
    pub valid_after: String,
    pub valid_before: String,
}

#[derive(Clone)]
struct TunnelClientMeta {
    ip_address: Option<String>,
    user_agent: Option<String>,
}

const MAX_SSH_BANNER_BYTES: usize = 4 * 1024;

#[utoipa::path(
    post,
    path = "/api/v1/ssh/{service_id}/certificate",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    request_body = IssueSshCertificateRequest,
    responses(
        (status = 200, description = "Issued short-lived SSH certificate", body = IssueSshCertificateResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "SSH service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn issue_ssh_certificate(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<IssueSshCertificateRequest>,
) -> AppResult<Json<IssueSshCertificateResponse>> {
    authorize_ssh_access(&state, &auth_user, &service_id).await?;
    let ssh_service = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    let user_id = auth_user.user_id.to_string();
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let issued = ssh_service::issue_certificate(
        &state.encryption_keys,
        &ssh_service,
        &service_id,
        &user_id,
        &user.email,
        &body.public_key,
        body.principal.trim(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_certificate_issued".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "key_id": issued.key_id,
            "principal": issued.principal,
            "routed_via": "ssh",
            "valid_after": issued.valid_after,
            "valid_before": issued.valid_before,
        })),
        None,
        None,
    );

    Ok(Json(IssueSshCertificateResponse {
        service_id,
        key_id: issued.key_id,
        principal: issued.principal,
        certificate: issued.certificate,
        ca_public_key: issued.ca_public_key,
        valid_after: issued.valid_after.to_rfc3339(),
        valid_before: issued.valid_before.to_rfc3339(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/ssh/{service_id}",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 101, description = "Switching protocols to WebSocket for SSH tunnel"),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "SSH service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn ssh_tunnel_ws(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    authorize_ssh_access(&state, &auth_user, &service_id).await?;
    let ssh_service = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    validate_runtime_ssh_target(&service_id, &ssh_service)?;
    let session_guard = state
        .ssh_session_manager
        .try_acquire(&auth_user.user_id.to_string())?;
    let client_meta = TunnelClientMeta {
        ip_address: Some(addr.ip().to_string()),
        user_agent: headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
    };

    Ok(ws
        .on_upgrade(move |socket| async move {
            handle_ssh_socket(
                state,
                auth_user,
                service_id,
                ssh_service,
                socket,
                session_guard,
                client_meta,
            )
            .await;
        })
        .into_response())
}

async fn handle_ssh_socket(
    state: AppState,
    auth_user: AuthUser,
    service_id: String,
    ssh_service: crate::models::downstream_service::SshServiceConfig,
    mut socket: WebSocket,
    _session_guard: ssh_service::SshSessionGuard,
    client_meta: TunnelClientMeta,
) {
    let user_id = auth_user.user_id.to_string();
    let session_id = uuid::Uuid::new_v4().to_string();
    let started_at = Instant::now();
    let node_route = match node_routing_service::resolve_node_route(
        &state.db,
        &user_id,
        &service_id,
        &state.node_ws_manager,
    )
    .await
    {
        Ok(route) => route,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "Failed to resolve SSH node route");
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Failed to resolve SSH route".into(),
                })))
                .await;
            return;
        }
    };

    if let Some(node_route) = node_route {
        handle_node_ssh_socket(
            state,
            service_id,
            ssh_service,
            socket,
            user_id,
            session_id,
            started_at,
            client_meta,
            node_route,
        )
        .await;
        return;
    }

    let connect_target = format!("{}:{}", ssh_service.host, ssh_service.port);
    let mut tcp_stream = match tokio::time::timeout(
        Duration::from_secs(state.config.ssh_connect_timeout_secs),
        tokio::net::TcpStream::connect(&connect_target),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            tracing::warn!(service_id = %service_id, error = %error, "SSH tunnel connect failed");
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Failed to connect downstream SSH target".into(),
                })))
                .await;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "ssh",
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": error.to_string(),
                })),
                client_meta.ip_address,
                client_meta.user_agent,
            );
            return;
        }
        Err(_) => {
            tracing::warn!(
                service_id = %service_id,
                timeout_secs = state.config.ssh_connect_timeout_secs,
                "SSH tunnel connect timed out"
            );
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "SSH target connect timed out".into(),
                })))
                .await;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "ssh",
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": "connect_timeout",
                    "timeout_secs": state.config.ssh_connect_timeout_secs,
                })),
                client_meta.ip_address,
                client_meta.user_agent,
            );
            return;
        }
    };

    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let initial_downstream_bytes = match read_direct_ssh_banner(
        &mut tcp_stream,
        state.config.ssh_connect_timeout_secs,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(service_id = %service_id, error = %error, "SSH tunnel target failed banner validation");
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Downstream target is not a valid SSH server".into(),
                })))
                .await;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "ssh",
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": error.to_string(),
                })),
                client_meta.ip_address,
                client_meta.user_agent,
            );
            return;
        }
    };
    if socket
        .send(Message::Binary(initial_downstream_bytes.clone().into()))
        .await
        .is_err()
    {
        audit_service::log_async(
            state.db.clone(),
            Some(user_id.clone()),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "ssh",
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "banner_send_failed",
            })),
            client_meta.ip_address.clone(),
            client_meta.user_agent.clone(),
        );
        return;
    }
    to_client_bytes += initial_downstream_bytes.len() as u64;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id.clone()),
        "ssh_tunnel_connected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "ssh",
            "target_host": ssh_service.host,
            "target_port": ssh_service.port,
        })),
        client_meta.ip_address.clone(),
        client_meta.user_agent.clone(),
    );

    let mut read_buf = vec![0_u8; 16 * 1024];
    let tunnel_timeout = tokio::time::sleep(Duration::from_secs(
        state.config.ssh_max_tunnel_duration_secs,
    ));
    tokio::pin!(tunnel_timeout);

    let disconnect_reason = loop {
        tokio::select! {
            _ = &mut tunnel_timeout => {
                let _ = socket
                    .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 1008,
                        reason: "SSH tunnel reached maximum duration".into(),
                    })))
                    .await;
                break Some("max_tunnel_duration_exceeded");
            }
            ws_message = socket.next() => {
                match ws_message {
                    Some(Ok(Message::Binary(bytes))) => {
                        from_client_bytes += bytes.len() as u64;
                        if tcp_stream.write_all(&bytes).await.is_err() {
                            break Some("downstream_write_failed");
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break Some("client_closed");
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1003,
                            reason: "SSH tunnel accepts binary frames only".into(),
                        }))).await;
                        break Some("invalid_client_frame");
                    }
                    Some(Err(_)) => break Some("client_socket_error"),
                }
            }
            tcp_read = tcp_stream.read(&mut read_buf) => {
                match tcp_read {
                    Ok(0) => break Some("downstream_closed"),
                    Ok(n) => {
                        to_client_bytes += n as u64;
                        if socket.send(Message::Binary(read_buf[..n].to_vec().into())).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Err(_) => break Some("downstream_read_failed"),
                }
            }
        }
    };

    let _ = socket.close().await;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_tunnel_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "ssh",
            "duration_ms": started_at.elapsed().as_millis() as u64,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
            "disconnect_reason": disconnect_reason,
        })),
        client_meta.ip_address,
        client_meta.user_agent,
    );
}

#[allow(clippy::too_many_arguments)]
async fn handle_node_ssh_socket(
    state: AppState,
    service_id: String,
    ssh_service: crate::models::downstream_service::SshServiceConfig,
    mut socket: WebSocket,
    user_id: String,
    session_id: String,
    started_at: Instant,
    client_meta: TunnelClientMeta,
    node_route: crate::services::node_routing_service::NodeRoute,
) {
    let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
        .chain(node_route.fallback_node_ids.iter().map(|id| id.as_str()))
        .collect();

    let mut tunnel_rx = None;
    let mut selected_node_id = None;

    for node_id in all_node_ids {
        match state
            .node_ws_manager
            .open_ssh_tunnel(
                node_id,
                crate::services::node_ws_manager::NodeSshTunnelRequest {
                    session_id: session_id.clone(),
                    service_id: service_id.clone(),
                    host: ssh_service.host.clone(),
                    port: ssh_service.port,
                },
            )
            .await
        {
            Ok(rx) => {
                tunnel_rx = Some(rx);
                selected_node_id = Some(node_id.to_string());
                break;
            }
            Err(error) => {
                tracing::warn!(service_id = %service_id, node_id = %node_id, error = %error, "SSH node tunnel open failed");
            }
        }
    }

    let Some(mut tunnel_rx) = tunnel_rx else {
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1011,
                reason: "Failed to connect downstream SSH target via node".into(),
            })))
            .await;
        audit_service::log_async(
            state.db.clone(),
            Some(user_id),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "node",
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "node_connect_failed",
            })),
            client_meta.ip_address,
            client_meta.user_agent,
        );
        return;
    };
    let Some(node_id) = selected_node_id else {
        tracing::error!(
            service_id = %service_id,
            session_id = %session_id,
            "Node-routed SSH tunnel opened without a selected node id"
        );
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1011,
                reason: "Failed to bind SSH tunnel to node".into(),
            })))
            .await;
        audit_service::log_async(
            state.db.clone(),
            Some(user_id),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "node",
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "missing_selected_node_id",
            })),
            client_meta.ip_address,
            client_meta.user_agent,
        );
        return;
    };

    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let initial_downstream_bytes = match read_node_ssh_banner(
        &mut tunnel_rx,
        state.config.ssh_connect_timeout_secs,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(service_id = %service_id, node_id = %node_id, error = %error, "Node-routed SSH tunnel failed banner validation");
            close_node_ssh_tunnel(
                &state,
                &service_id,
                &node_id,
                &session_id,
                "banner_validation_failed",
            );
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1011,
                    reason: "Downstream target is not a valid SSH server".into(),
                })))
                .await;
            audit_service::log_async(
                state.db.clone(),
                Some(user_id),
                "ssh_tunnel_connect_failed".to_string(),
                Some(serde_json::json!({
                    "service_id": service_id,
                    "session_id": session_id,
                    "routed_via": "node",
                    "node_id": node_id,
                    "target_host": ssh_service.host,
                    "target_port": ssh_service.port,
                    "error": error.to_string(),
                })),
                client_meta.ip_address,
                client_meta.user_agent,
            );
            return;
        }
    };
    if socket
        .send(Message::Binary(initial_downstream_bytes.clone().into()))
        .await
        .is_err()
    {
        close_node_ssh_tunnel(
            &state,
            &service_id,
            &node_id,
            &session_id,
            "banner_send_failed",
        );
        audit_service::log_async(
            state.db.clone(),
            Some(user_id.clone()),
            "ssh_tunnel_connect_failed".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "session_id": session_id,
                "routed_via": "node",
                "node_id": node_id,
                "target_host": ssh_service.host,
                "target_port": ssh_service.port,
                "error": "banner_send_failed",
            })),
            client_meta.ip_address.clone(),
            client_meta.user_agent.clone(),
        );
        return;
    }
    to_client_bytes += initial_downstream_bytes.len() as u64;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id.clone()),
        "ssh_tunnel_connected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "node",
            "node_id": node_id,
            "target_host": ssh_service.host,
            "target_port": ssh_service.port,
        })),
        client_meta.ip_address.clone(),
        client_meta.user_agent.clone(),
    );

    let tunnel_timeout = tokio::time::sleep(Duration::from_secs(
        state.config.ssh_max_tunnel_duration_secs,
    ));
    tokio::pin!(tunnel_timeout);

    let disconnect_reason = loop {
        tokio::select! {
            _ = &mut tunnel_timeout => {
                let _ = socket
                    .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 1008,
                        reason: "SSH tunnel reached maximum duration".into(),
                    })))
                    .await;
                break Some("max_tunnel_duration_exceeded");
            }
            ws_message = socket.next() => {
                match ws_message {
                    Some(Ok(Message::Binary(bytes))) => {
                        from_client_bytes += bytes.len() as u64;
                        if state.node_ws_manager.send_ssh_tunnel_data(&node_id, &session_id, &bytes).is_err() {
                            break Some("node_tunnel_send_failed");
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break Some("client_closed");
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1003,
                            reason: "SSH tunnel accepts binary frames only".into(),
                        }))).await;
                        break Some("invalid_client_frame");
                    }
                    Some(Err(_)) => break Some("client_socket_error"),
                }
            }
            tunnel_message = tunnel_rx.recv() => {
                match tunnel_message {
                    Some(crate::services::node_ws_manager::SshTunnelChunk::Data(bytes)) => {
                        to_client_bytes += bytes.len() as u64;
                        if socket.send(Message::Binary(bytes.into())).await.is_err() {
                            break Some("client_write_failed");
                        }
                    }
                    Some(crate::services::node_ws_manager::SshTunnelChunk::Closed(error)) => {
                        break if error.is_some() {
                            Some("node_tunnel_closed_with_error")
                        } else {
                            Some("node_tunnel_closed")
                        };
                    }
                    None => break Some("node_tunnel_channel_closed"),
                }
            }
        }
    };

    close_node_ssh_tunnel(
        &state,
        &service_id,
        &node_id,
        &session_id,
        "session_cleanup",
    );
    let _ = socket.close().await;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "ssh_tunnel_disconnected".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "session_id": session_id,
            "routed_via": "node",
            "node_id": node_id,
            "duration_ms": started_at.elapsed().as_millis() as u64,
            "bytes_from_client": from_client_bytes,
            "bytes_to_client": to_client_bytes,
            "disconnect_reason": disconnect_reason,
        })),
        client_meta.ip_address,
        client_meta.user_agent,
    );
}

async fn authorize_ssh_access(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
) -> AppResult<()> {
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();
    let service = fetch_service(state, service_id).await?;
    ssh_service::ensure_ssh_service(&service)?;

    let requires_approval = approval_service::requires_approval_for_service(
        &state.db,
        &approval_owner_user_id,
        service_id,
    )
    .await?;

    if requires_approval && auth_user.auth_method != AuthMethod::Session {
        let requester_type = auth_user.approval_requester_type().ok_or_else(|| {
            AppError::Forbidden("Session auth does not require approval".to_string())
        })?;
        let has_grant = approval_service::check_approval(
            &state.db,
            &approval_owner_user_id,
            service_id,
            requester_type,
            &auth_user.approval_requester_id(),
        )
        .await?;

        if !has_grant {
            let channel =
                notification_service::get_or_create_channel(&state.db, &approval_owner_user_id)
                    .await?;
            let timeout_secs = channel.approval_timeout_secs;
            let approval_request = approval_service::create_approval_request(
                &state.db,
                &state.config,
                &state.http_client,
                state.fcm_auth.as_deref(),
                state.apns_auth.as_deref(),
                &approval_owner_user_id,
                service_id,
                &service.name,
                &service.slug,
                requester_type,
                &auth_user.approval_requester_id(),
                None,
                "ssh:tunnel",
                timeout_secs,
            )
            .await?;

            approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
                .await?;
        }
    }

    Ok(())
}

async fn read_direct_ssh_banner(
    stream: &mut tokio::net::TcpStream,
    timeout_secs: u64,
) -> AppResult<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut buffer = Vec::with_capacity(256);
        let mut chunk = [0_u8; 512];
        loop {
            let read = stream.read(&mut chunk).await.map_err(|error| {
                AppError::BadRequest(format!(
                    "Failed to read SSH banner from downstream: {error}"
                ))
            })?;
            if read == 0 {
                return Err(AppError::BadRequest(
                    "Downstream target closed before sending an SSH banner".to_string(),
                ));
            }
            buffer.extend_from_slice(&chunk[..read]);
            if ssh_banner_validated(&buffer)? {
                return Ok(buffer);
            }
        }
    })
    .await
    .map_err(|_| {
        AppError::BadRequest("Timed out waiting for SSH banner from downstream".to_string())
    })?
}

async fn read_node_ssh_banner(
    tunnel_rx: &mut tokio::sync::mpsc::Receiver<crate::services::node_ws_manager::SshTunnelChunk>,
    timeout_secs: u64,
) -> AppResult<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut buffer = Vec::with_capacity(256);
        loop {
            match tunnel_rx.recv().await {
                Some(crate::services::node_ws_manager::SshTunnelChunk::Data(bytes)) => {
                    buffer.extend_from_slice(&bytes);
                    if ssh_banner_validated(&buffer)? {
                        return Ok(buffer);
                    }
                }
                Some(crate::services::node_ws_manager::SshTunnelChunk::Closed(Some(error))) => {
                    return Err(AppError::NodeOffline(format!(
                        "Node tunnel closed before SSH banner: {error}"
                    )));
                }
                Some(crate::services::node_ws_manager::SshTunnelChunk::Closed(None)) | None => {
                    return Err(AppError::NodeOffline(
                        "Node tunnel closed before SSH banner".to_string(),
                    ));
                }
            }
        }
    })
    .await
    .map_err(|_| AppError::BadRequest("Timed out waiting for SSH banner from node".to_string()))?
}

fn ssh_banner_validated(buffer: &[u8]) -> AppResult<bool> {
    let mut offset = 0;
    while let Some(relative_end) = buffer[offset..].iter().position(|byte| *byte == b'\n') {
        let end = offset + relative_end + 1;
        let line = &buffer[offset..end];
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);

        if line.starts_with(b"SSH-2.0-") || line.starts_with(b"SSH-1.99-") {
            return Ok(true);
        }
        if line.starts_with(b"SSH-") {
            return Err(AppError::BadRequest(
                "Downstream target returned an unsupported SSH banner".to_string(),
            ));
        }

        offset = end;
    }

    if buffer.len() >= MAX_SSH_BANNER_BYTES {
        return Err(AppError::BadRequest(
            "Downstream target did not present an SSH identification banner".to_string(),
        ));
    }

    Ok(false)
}

fn validate_runtime_ssh_target(
    service_id: &str,
    ssh_service: &crate::models::downstream_service::SshServiceConfig,
) -> AppResult<()> {
    ssh_service::validate_ssh_target(&ssh_service.host, ssh_service.port).map_err(|error| {
        tracing::warn!(
            service_id,
            host = %ssh_service.host,
            port = ssh_service.port,
            error = %error,
            "Rejected invalid SSH target during tunnel setup"
        );
        error
    })
}

fn close_node_ssh_tunnel(
    state: &AppState,
    service_id: &str,
    node_id: &str,
    session_id: &str,
    reason: &str,
) {
    if let Err(error) = state.node_ws_manager.close_ssh_tunnel(node_id, session_id) {
        tracing::warn!(
            service_id,
            node_id,
            session_id,
            reason,
            error = %error,
            "Failed to close node-routed SSH tunnel"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_SSH_BANNER_BYTES, ssh_banner_validated, validate_runtime_ssh_target};
    use crate::models::downstream_service::SshServiceConfig;

    #[test]
    fn accepts_valid_ssh_banner_after_preamble() {
        let buffer = b"NOTICE banner\r\nSSH-2.0-OpenSSH_9.7\r\n";
        assert!(ssh_banner_validated(buffer).expect("valid banner"));
    }

    #[test]
    fn rejects_unsupported_ssh_version_banner() {
        let error = ssh_banner_validated(b"SSH-1.5-legacy\r\n").expect_err("unsupported banner");
        assert!(error.to_string().contains("unsupported SSH banner"));
    }

    #[test]
    fn rejects_non_ssh_target_when_banner_limit_is_exceeded() {
        let buffer = vec![b'x'; MAX_SSH_BANNER_BYTES];
        let error = ssh_banner_validated(&buffer).expect_err("missing banner");
        assert!(
            error
                .to_string()
                .contains("did not present an SSH identification banner")
        );
    }

    #[test]
    fn rejects_invalid_runtime_ssh_target() {
        let error = validate_runtime_ssh_target(
            "svc-1",
            &SshServiceConfig {
                host: "127.0.0.1".to_string(),
                port: 22,
                certificate_auth_enabled: false,
                certificate_ttl_minutes: 30,
                allowed_principals: Vec::new(),
                ca_private_key_encrypted: None,
                ca_public_key: None,
            },
        )
        .expect_err("invalid target");

        assert!(error.to_string().contains("private or internal IP address"));
    }
}
