use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, node_routing_service, node_service, ssh_service};

use super::ssh_tunnel::authorize_ssh_access;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum allowed timeout for SSH command execution (5 minutes).
const MAX_TIMEOUT_SECS: u32 = 300;

/// Maximum bytes captured per output stream (stdout / stderr).
const MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB

/// Commands (or fragments) that are unconditionally blocked.
const DANGEROUS_COMMANDS: &[&str] = &[
    "rm -rf /",
    "mkfs",
    "dd if=",
    "shutdown",
    "reboot",
    "halt",
    "init 0",
    ":(){ :|:& };:",
];

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

fn default_timeout() -> u32 {
    30
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SshExecRequest {
    /// Shell command to execute on the remote machine.
    pub command: String,
    /// SSH principal (Unix username) to run the command as.
    pub principal: String,
    /// Maximum execution time in seconds (default 30, max 300).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SshExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/ssh/{service_id}/exec",
    params(
        ("service_id" = String, Path, description = "Downstream SSH service ID")
    ),
    request_body = SshExecRequest,
    responses(
        (status = 200, description = "Command execution result", body = SshExecResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "SSH service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "SSH"
)]
pub async fn ssh_exec(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SshExecRequest>,
) -> AppResult<Json<SshExecResponse>> {
    // -- Auth --
    authorize_ssh_access(&state, &auth_user, &service_id).await?;

    let ssh_svc = ssh_service::get_ssh_service(&state.db, &service_id).await?;
    let user_id = auth_user.user_id.to_string();

    // -- Validate: certificate auth must be enabled --
    if !ssh_svc.certificate_auth_enabled {
        return Err(AppError::BadRequest(
            "SSH command execution requires certificate auth to be enabled".to_string(),
        ));
    }

    // -- Validate principal --
    let principal = body.principal.trim().to_string();
    ssh_service::validate_principal(&principal)?;
    if !ssh_svc.allowed_principals.iter().any(|p| p == &principal) {
        return Err(AppError::Forbidden(
            "Requested SSH principal is not allowed for this service".to_string(),
        ));
    }

    // -- Validate timeout --
    let timeout_secs = body.timeout_secs.clamp(1, MAX_TIMEOUT_SECS);

    // -- Validate command --
    let command = body.command.trim().to_string();
    if command.is_empty() {
        return Err(AppError::ValidationError(
            "command must not be empty".to_string(),
        ));
    }
    if command.len() > 8192 {
        return Err(AppError::ValidationError(
            "command must not exceed 8192 characters".to_string(),
        ));
    }
    check_dangerous_command(&command)?;

    // -- Session limiting --
    let session_guard = state.ssh_session_manager.try_acquire(&user_id)?;

    let ip_address = Some(addr.ip().to_string());
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    // -- Require a node agent --
    // SSH commands are executed on the node agent, not the NyxID server.
    let node_route = node_routing_service::resolve_node_route(
        &state.db,
        &user_id,
        &service_id,
        &state.node_ws_manager,
    )
    .await
    .ok()
    .flatten();

    let node_route = node_route.ok_or_else(|| {
        AppError::BadRequest(
            "No node agent is bound to this SSH service. \
             Deploy a NyxID node agent and bind it to this service to execute commands."
                .to_string(),
        )
    })?;

    // -- Generate ephemeral SSH credentials (key + cert as strings, no files) --
    let ephemeral = super::ssh_web_terminal::generate_ephemeral_credentials(
        &state,
        &ssh_svc,
        &service_id,
        &user_id,
        &principal,
    )
    .await?;

    // -- Execute via node agent with failover --
    let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
        .chain(node_route.fallback_node_ids.iter().map(|id| id.as_str()))
        .collect();

    let request_id = uuid::Uuid::new_v4().to_string();
    let mut last_error = None;

    for node_id in &all_node_ids {
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
                        "SSH exec node signing secret resolution failed"
                    );
                    last_error = Some(format!("Signing secret error: {error}"));
                    continue;
                }
            }
        } else {
            None
        };

        match state
            .node_ws_manager
            .exec_ssh_command(
                node_id,
                crate::services::node_ws_manager::NodeSshExecRequest {
                    request_id: request_id.clone(),
                    host: ssh_svc.host.clone(),
                    port: ssh_svc.port,
                    principal: principal.clone(),
                    private_key_pem: ephemeral.private_key_pem.clone(),
                    certificate_openssh: ephemeral.certificate_openssh.clone(),
                    command: command.clone(),
                    timeout_secs,
                },
                signing_secret.as_ref().map(|s| s.as_slice()),
            )
            .await
        {
            Ok(result) => {
                // Keep session guard alive until command completes.
                let _ = &session_guard;
                drop(session_guard);

                let response = SshExecResponse {
                    exit_code: result.exit_code,
                    stdout: truncate_output(result.stdout.as_bytes()),
                    stderr: truncate_output(result.stderr.as_bytes()),
                    duration_ms: result.duration_ms,
                    timed_out: result.timed_out,
                };

                // -- Audit log --
                audit_service::log_async(
                    state.db.clone(),
                    Some(user_id),
                    "ssh_exec_command".to_string(),
                    Some(serde_json::json!({
                        "service_id": service_id,
                        "principal": principal,
                        "command": truncate_for_audit(&command),
                        "exit_code": response.exit_code,
                        "duration_ms": response.duration_ms,
                        "timed_out": response.timed_out,
                        "routed_via": "node",
                        "node_id": node_id,
                    })),
                    ip_address,
                    user_agent,
                );

                return Ok(Json(response));
            }
            Err(error) => {
                tracing::warn!(
                    service_id = %service_id,
                    node_id = %node_id,
                    error = %error,
                    "SSH exec via node failed, trying next"
                );
                last_error = Some(error.to_string());
            }
        }
    }

    // Keep session guard alive until we return.
    let _ = &session_guard;
    drop(session_guard);

    Err(AppError::Internal(format!(
        "SSH exec failed on all nodes: {}",
        last_error.unwrap_or_else(|| "no nodes available".to_string()),
    )))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if the command contains any dangerous patterns.
pub(crate) fn check_dangerous_command(command: &str) -> AppResult<()> {
    let normalized = command.to_lowercase();
    for pattern in DANGEROUS_COMMANDS {
        if *pattern == "rm -rf /" {
            // Special handling: only block "rm -rf /" when it targets root
            // (end of string or followed by whitespace), not "rm -rf /tmp/..."
            if let Some(pos) = normalized.find(pattern) {
                let after = pos + pattern.len();
                if after >= normalized.len() || normalized.as_bytes()[after].is_ascii_whitespace() {
                    return Err(AppError::Forbidden(format!(
                        "Command contains a blocked pattern: {pattern}"
                    )));
                }
            }
        } else if normalized.contains(pattern) {
            return Err(AppError::Forbidden(format!(
                "Command contains a blocked pattern: {pattern}"
            )));
        }
    }
    Ok(())
}

/// Truncate output bytes to MAX_OUTPUT_BYTES and convert to a lossy UTF-8 string.
pub(crate) fn truncate_output(bytes: &[u8]) -> String {
    let truncated = if bytes.len() > MAX_OUTPUT_BYTES {
        &bytes[..MAX_OUTPUT_BYTES]
    } else {
        bytes
    };
    String::from_utf8_lossy(truncated).into_owned()
}

/// Truncate command for audit logging (avoid storing giant payloads).
pub(crate) fn truncate_for_audit(command: &str) -> &str {
    if command.len() > 1024 {
        &command[..1024]
    } else {
        command
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_dangerous_command_blocks_rm_rf() {
        assert!(check_dangerous_command("rm -rf /").is_err());
        assert!(check_dangerous_command("sudo rm -rf /").is_err());
        assert!(check_dangerous_command("rm -rf / --no-preserve-root").is_err());
        // Subpaths are not blocked (legitimate use cases)
        assert!(check_dangerous_command("rm -rf /tmp/build_artifacts").is_ok());
    }

    #[test]
    fn check_dangerous_command_blocks_fork_bomb() {
        assert!(check_dangerous_command(":(){ :|:& };:").is_err());
    }

    #[test]
    fn check_dangerous_command_blocks_mkfs() {
        assert!(check_dangerous_command("mkfs.ext4 /dev/sda1").is_err());
    }

    #[test]
    fn check_dangerous_command_blocks_dd() {
        assert!(check_dangerous_command("dd if=/dev/zero of=/dev/sda").is_err());
    }

    #[test]
    fn check_dangerous_command_blocks_shutdown_reboot_halt() {
        assert!(check_dangerous_command("shutdown -h now").is_err());
        assert!(check_dangerous_command("reboot").is_err());
        assert!(check_dangerous_command("halt").is_err());
        assert!(check_dangerous_command("init 0").is_err());
    }

    #[test]
    fn check_dangerous_command_allows_safe_commands() {
        assert!(check_dangerous_command("ls -la /tmp").is_ok());
        assert!(check_dangerous_command("cat /etc/hostname").is_ok());
        assert!(check_dangerous_command("uname -a").is_ok());
        assert!(check_dangerous_command("rm -rf /tmp/build_artifacts").is_ok());
    }

    #[test]
    fn truncate_output_respects_limit() {
        let small = b"hello";
        assert_eq!(truncate_output(small), "hello");

        let big = vec![b'x'; MAX_OUTPUT_BYTES + 100];
        let result = truncate_output(&big);
        assert_eq!(result.len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn truncate_for_audit_respects_limit() {
        let small = "ls -la";
        assert_eq!(truncate_for_audit(small), small);

        let big: String = "x".repeat(2000);
        assert_eq!(truncate_for_audit(&big).len(), 1024);
    }

    #[test]
    fn default_timeout_is_30() {
        assert_eq!(default_timeout(), 30);
    }
}
