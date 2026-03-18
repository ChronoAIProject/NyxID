use std::time::Instant;

use axum::{
    Json,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    approval_service, audit_service, notification_service, proxy_service, ssh_service,
};

use super::services_helpers::{fetch_service, require_admin_or_creator};

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertSshServiceRequest {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SshServiceResponse {
    pub service_id: String,
    pub host: String,
    pub port: u16,
    pub enabled: bool,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteSshServiceResponse {
    pub message: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/services/{service_id}/ssh",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "SSH tunnel configuration", body = SshServiceResponse),
        (status = 404, description = "SSH service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn get_ssh_service_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<SshServiceResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;
    let ssh_service = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    Ok(Json(ssh_service_to_response(ssh_service)))
}

#[utoipa::path(
    put,
    path = "/api/v1/services/{service_id}/ssh",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    request_body = UpsertSshServiceRequest,
    responses(
        (status = 200, description = "Updated SSH tunnel configuration", body = SshServiceResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn upsert_ssh_service_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<UpsertSshServiceRequest>,
) -> AppResult<Json<SshServiceResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;
    ssh_service::validate_ssh_target(&body.host, body.port)?;

    let ssh_service = ssh_service::upsert_ssh_service(
        &state.db,
        &service_id,
        body.host.trim(),
        body.port,
        &auth_user.user_id.to_string(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "ssh_service_upserted".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "host": ssh_service.host,
            "port": ssh_service.port,
        })),
        None,
        None,
    );

    Ok(Json(ssh_service_to_response(ssh_service)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/services/{service_id}/ssh",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "SSH tunnel configuration disabled", body = DeleteSshServiceResponse),
        (status = 404, description = "SSH service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn delete_ssh_service_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<DeleteSshServiceResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;
    ssh_service::disable_ssh_service(&state.db, &service_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "ssh_service_disabled".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
        })),
        None,
        None,
    );

    Ok(Json(DeleteSshServiceResponse {
        message: "SSH service disabled".to_string(),
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
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    authorize_ssh_access(&state, &auth_user, &service_id).await?;
    let ssh_service = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    let session_guard = state
        .ssh_session_manager
        .try_acquire(&auth_user.user_id.to_string())?;

    Ok(ws
        .on_upgrade(move |socket| async move {
            handle_ssh_socket(
                state,
                auth_user,
                service_id,
                ssh_service,
                socket,
                session_guard,
            )
            .await;
        })
        .into_response())
}

async fn handle_ssh_socket(
    state: AppState,
    auth_user: AuthUser,
    service_id: String,
    ssh_service: crate::models::ssh_service::SshService,
    mut socket: WebSocket,
    _session_guard: ssh_service::SshSessionGuard,
) {
    let user_id = auth_user.user_id.to_string();
    let session_id = uuid::Uuid::new_v4().to_string();
    let started_at = Instant::now();

    let connect_target = format!("{}:{}", ssh_service.host, ssh_service.port);
    let mut tcp_stream = match tokio::net::TcpStream::connect(&connect_target).await {
        Ok(stream) => stream,
        Err(error) => {
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
                None,
                None,
            );
            return;
        }
    };

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
        None,
        None,
    );

    let mut from_client_bytes: u64 = 0;
    let mut to_client_bytes: u64 = 0;
    let mut read_buf = vec![0_u8; 16 * 1024];

    loop {
        tokio::select! {
            ws_message = socket.next() => {
                match ws_message {
                    Some(Ok(Message::Binary(bytes))) => {
                        from_client_bytes += bytes.len() as u64;
                        if tcp_stream.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1003,
                            reason: "SSH tunnel accepts binary frames only".into(),
                        }))).await;
                        break;
                    }
                    Some(Err(_)) => break,
                }
            }
            tcp_read = tcp_stream.read(&mut read_buf) => {
                match tcp_read {
                    Ok(0) => break,
                    Ok(n) => {
                        to_client_bytes += n as u64;
                        if socket.send(Message::Binary(read_buf[..n].to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

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
        })),
        None,
        None,
    );
}

async fn authorize_ssh_access(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();

    let target = proxy_service::resolve_proxy_target(
        &state.db,
        &state.encryption_keys,
        &user_id,
        service_id,
    )
    .await?;

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
                &target.service.name,
                &target.service.slug,
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

fn ssh_service_to_response(model: crate::models::ssh_service::SshService) -> SshServiceResponse {
    SshServiceResponse {
        service_id: model.id,
        host: model.host,
        port: model.port,
        enabled: model.enabled,
        created_by: model.created_by,
        created_at: model.created_at.to_rfc3339(),
        updated_at: model.updated_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::ssh_service_to_response;
    use crate::models::ssh_service::SshService;
    use chrono::Utc;

    #[test]
    fn maps_ssh_service_response() {
        let response = ssh_service_to_response(SshService {
            id: "service-1".to_string(),
            host: "ssh.internal".to_string(),
            port: 22,
            enabled: true,
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        assert_eq!(response.service_id, "service-1");
        assert_eq!(response.port, 22);
    }
}
