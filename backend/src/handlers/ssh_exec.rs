use std::time::Instant;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, node_routing_service, ssh_service};

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

    // -- Generate ephemeral SSH key + certificate --
    let temp_dir =
        write_ephemeral_ssh_files(&state, &ssh_svc, &service_id, &user_id, &principal).await?;

    let key_path = temp_dir.path().join("id_ed25519");
    let cert_path = temp_dir.path().join("id_ed25519-cert.pub");

    // -- Check for node routing --
    let node_route = node_routing_service::resolve_node_route(
        &state.db,
        &user_id,
        &service_id,
        &state.node_ws_manager,
    )
    .await
    .ok()
    .flatten();

    // -- Build ssh command --
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.arg("-o")
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
        .arg("LogLevel=FATAL")
        .arg("-o")
        .arg("RequestTTY=no");

    // If node-routed, add ProxyCommand
    if node_route.is_some() {
        let proxy_token = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &auth_user.user_id,
            "openid",
            None,
        )
        .map_err(|e| AppError::Internal(format!("Failed to generate proxy token: {e}")))?;

        let nyxid_binary =
            std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("nyxid"));
        let base_url = state.config.base_url.trim_end_matches('/');

        let proxy_cmd = format!(
            "{} ssh proxy --base-url {} --service-id {}",
            nyxid_binary.display(),
            base_url,
            service_id,
        );
        cmd.arg("-o").arg(format!("ProxyCommand={proxy_cmd}"));
        cmd.env("NYXID_ACCESS_TOKEN", &proxy_token);
    }

    cmd.arg("-p")
        .arg(ssh_svc.port.to_string())
        .arg(format!("{principal}@{}", ssh_svc.host))
        .arg(&command);

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    // -- Spawn and execute with timeout --
    let started_at = Instant::now();

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to spawn ssh process: {e}")))?;

    // Take stdout/stderr handles so we can read them without consuming `child`.
    let child_stdout = child.stdout.take();
    let child_stderr = child.stderr.take();

    let read_and_wait = async {
        let stdout_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut out) = child_stdout {
                use tokio::io::AsyncReadExt;
                let _ = out.read_to_end(&mut buf).await;
            }
            buf
        });
        let stderr_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut err) = child_stderr {
                use tokio::io::AsyncReadExt;
                let _ = err.read_to_end(&mut buf).await;
            }
            buf
        });

        let status = child.wait().await;
        let stdout_bytes = stdout_handle.await.unwrap_or_default();
        let stderr_bytes = stderr_handle.await.unwrap_or_default();
        (status, stdout_bytes, stderr_bytes)
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs as u64),
        read_and_wait,
    )
    .await;

    let duration_ms = started_at.elapsed().as_millis() as u64;

    // Keep session guard alive until command completes.
    let _ = &session_guard;
    drop(session_guard);

    let response = match result {
        Ok((Ok(status), stdout_bytes, stderr_bytes)) => {
            let stdout = truncate_output(&stdout_bytes);
            let stderr = truncate_output(&stderr_bytes);
            let exit_code = status.code().unwrap_or(-1);

            SshExecResponse {
                exit_code,
                stdout,
                stderr,
                duration_ms,
                timed_out: false,
            }
        }
        Ok((Err(e), _, _)) => {
            return Err(AppError::Internal(format!("SSH process failed: {e}")));
        }
        Err(_) => {
            // Timeout: child was consumed by the async block but the
            // timeout dropped it, which kills the process on drop.
            SshExecResponse {
                exit_code: -1,
                stdout: String::new(),
                stderr: "Command execution timed out".to_string(),
                duration_ms,
                timed_out: true,
            }
        }
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
            "routed_via": if node_route.is_some() { "node" } else { "ssh" },
        })),
        ip_address,
        user_agent,
    );

    Ok(Json(response))
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

/// Generate ephemeral SSH key + certificate and write them to a temp directory.
/// Same pattern as `ssh_web_terminal::write_ephemeral_ssh_files`.
pub(crate) async fn write_ephemeral_ssh_files(
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
