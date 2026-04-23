use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures::{SinkExt, StreamExt};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

use crate::api::{ApiClient, build_cli_http_client};
use crate::cli::SshCli;

/// Resolve service_id to DownstreamService ID.
/// The user may pass a UserService ID, a slug, or a DownstreamService ID.
/// SSH endpoints need the DownstreamService ID (catalog_service_id).
async fn resolve_ssh_service_id(api: &mut ApiClient, id_or_slug: &str) -> Result<String> {
    // Try to get it as a UserService (from /keys/{id} or by listing and matching slug)
    if let Ok(svc) = api.get_value(&format!("/keys/{id_or_slug}")).await {
        // UserService found -- use its catalog_service_id
        if let Some(catalog_id) = svc.get("catalog_service_id").and_then(|v| v.as_str())
            && !catalog_id.is_empty()
        {
            return Ok(catalog_id.to_string());
        }
        // If no catalog_service_id, the ID itself might be a DownstreamService ID
        return Ok(id_or_slug.to_string());
    }

    // Try to find by slug in the user's services
    if let Ok(resp) = api.get_value("/keys").await
        && let Some(keys) = resp.get("keys").and_then(|v| v.as_array())
        && let Some(svc) = keys.iter().find(|k| k["slug"].as_str() == Some(id_or_slug))
    {
        if let Some(catalog_id) = svc.get("catalog_service_id").and_then(|v| v.as_str())
            && !catalog_id.is_empty()
        {
            return Ok(catalog_id.to_string());
        }
        if let Some(id) = svc["id"].as_str() {
            return Ok(id.to_string());
        }
    }

    // Fall back -- assume it's already a DownstreamService ID
    Ok(id_or_slug.to_string())
}

pub async fn run(cli: SshCli) -> Result<()> {
    match cli.command {
        crate::cli::SshCommand::IssueCert {
            auth,
            service_id,
            public_key_file,
            principal,
            certificate_file,
            ca_public_key_file,
        } => {
            let token = crate::auth::resolve_access_token(&auth)?;
            let base_url = auth.resolved_base_url()?;
            let mut api = ApiClient::from_auth(&auth)?;
            let resolved_id = resolve_ssh_service_id(&mut api, &service_id).await?;
            issue_certificate(
                &base_url,
                &resolved_id,
                &token,
                &public_key_file,
                &principal,
                &certificate_file,
                ca_public_key_file.as_deref(),
                auth.profile.as_deref(),
            )
            .await
        }
        crate::cli::SshCommand::Proxy {
            auth,
            service_id,
            issue_certificate: do_issue,
            public_key_file,
            principal,
            certificate_file,
            ca_public_key_file,
        } => {
            let token = crate::auth::resolve_access_token(&auth)?;
            let base_url = auth.resolved_base_url()?;
            let mut api = ApiClient::from_auth(&auth)?;
            let resolved_id = resolve_ssh_service_id(&mut api, &service_id).await?;

            if do_issue {
                let pk =
                    public_key_file.context("--issue-certificate requires --public-key-file")?;
                let princ = principal.context("--issue-certificate requires --principal")?;
                let cert =
                    certificate_file.context("--issue-certificate requires --certificate-file")?;
                issue_certificate(
                    &base_url,
                    &resolved_id,
                    &token,
                    &pk,
                    &princ,
                    &cert,
                    ca_public_key_file.as_deref(),
                    auth.profile.as_deref(),
                )
                .await?;
            }

            run_proxy(&base_url, &resolved_id, &token).await
        }
        crate::cli::SshCommand::Config {
            host_alias,
            base_url,
            service_id,
            principal,
            identity_file,
            certificate_file,
            access_token_env,
            ca_public_key_file,
        } => run_config(
            &host_alias,
            &base_url,
            &service_id,
            &principal,
            &identity_file,
            &certificate_file,
            &access_token_env,
            ca_public_key_file.as_deref(),
        ),
        crate::cli::SshCommand::Exec {
            auth,
            service_id,
            principal,
            command,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let resolved_id = resolve_ssh_service_id(&mut api, &service_id).await?;
            run_exec(&auth, &resolved_id, &principal, &command).await
        }
        crate::cli::SshCommand::Terminal {
            auth,
            service_id,
            principal,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let resolved_id = resolve_ssh_service_id(&mut api, &service_id).await?;

            // Resolve principal: flag > first allowed principal from service config
            let effective_principal = if let Some(p) = principal {
                p
            } else {
                // Try to get default principal from the service
                let svc: Value = api
                    .get_value(&format!("/keys/{}", &service_id))
                    .await
                    .or_else(|_| -> Result<Value> { Ok(Value::Null) })?;
                let principals = svc
                    .get("ssh_allowed_principals")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                match principals {
                    Some(p) => p,
                    None => {
                        bail!(
                            "No --principal specified and could not determine default. Use --principal <username>."
                        );
                    }
                }
            };

            run_terminal(&auth, &resolved_id, Some(&effective_principal)).await
        }
    }
}

// ---- SSH exec (I27) ----

async fn run_exec(
    auth: &crate::cli::AuthArgs,
    service_id: &str,
    principal: &str,
    command: &[String],
) -> Result<()> {
    let mut api = crate::api::ApiClient::from_auth(auth)?;
    let cmd_str = command.join(" ");

    let body = serde_json::json!({
        "principal": principal,
        "command": cmd_str,
    });

    let result: Value = api.post(&format!("/ssh/{service_id}/exec"), &body).await?;

    match auth.output {
        crate::cli::OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        crate::cli::OutputFormat::Table => {
            if let Some(stdout) = result["stdout"].as_str() {
                print!("{stdout}");
            }
            if let Some(stderr) = result["stderr"].as_str()
                && !stderr.is_empty()
            {
                eprint!("{stderr}");
            }
            if let Some(exit_code) = result["exit_code"].as_i64()
                && exit_code != 0
            {
                std::process::exit(exit_code as i32);
            }
        }
    }
    Ok(())
}

// ---- SSH terminal (I28) ----

async fn run_terminal(
    auth: &crate::cli::AuthArgs,
    service_id: &str,
    principal: Option<&str>,
) -> Result<()> {
    let token = crate::auth::resolve_access_token(auth)?;
    let resolved = auth.resolved_base_url()?;
    let base_url = resolved.trim_end_matches('/');

    // Build the terminal WebSocket URL
    let ws_url = build_terminal_ws_url(base_url, service_id, principal)?;
    let mut request = ws_url.into_client_request()?;
    request
        .headers_mut()
        .insert(AUTHORIZATION, bearer_header(&token)?);

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("Failed to open SSH terminal WebSocket")?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut read_buf = vec![0_u8; 16 * 1024];

    eprintln!("Connected to SSH terminal. Press Ctrl+C to disconnect.");

    loop {
        tokio::select! {
            read = stdin.read(&mut read_buf) => {
                let count = read.context("Failed to read stdin")?;
                if count == 0 {
                    let _ = ws_sink.close().await;
                    break;
                }
                ws_sink
                    .send(Message::Binary(read_buf[..count].to_vec().into()))
                    .await
                    .context("Failed to send to SSH terminal")?;
            }
            message = ws_stream.next() => {
                match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        stdout
                            .write_all(&bytes)
                            .await
                            .context("Failed to write SSH terminal output")?;
                        stdout.flush().await.context("Failed to flush stdout")?;
                    }
                    Some(Ok(Message::Text(text))) => {
                        stdout
                            .write_all(text.as_bytes())
                            .await
                            .context("Failed to write SSH terminal text")?;
                        stdout.flush().await.context("Failed to flush stdout")?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ws_sink
                            .send(Message::Pong(payload))
                            .await
                            .context("Failed to respond to ping")?;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => return Err(error).context("SSH terminal WebSocket failed"),
                }
            }
        }
    }

    eprintln!("Disconnected.");
    Ok(())
}

fn build_terminal_ws_url(
    base_url: &str,
    service_id: &str,
    principal: Option<&str>,
) -> Result<String> {
    let mut url = url::Url::parse(base_url).context("Invalid NyxID base URL")?;
    match url.scheme() {
        "https" => url.set_scheme("wss").expect("valid scheme transition"),
        "http" => url.set_scheme("ws").expect("valid scheme transition"),
        "wss" | "ws" => {}
        scheme => bail!("Unsupported NyxID base URL scheme: {scheme}"),
    }
    url.set_path(&format!("/api/v1/ssh/{service_id}/terminal"));
    if let Some(p) = principal {
        url.set_query(Some(&format!("principal={p}")));
    } else {
        url.set_query(None);
    }
    Ok(url.to_string())
}

// ---- Issue certificate (ported from ssh_cli.rs) ----

#[derive(Serialize)]
struct IssueCertificateRequest {
    public_key: String,
    principal: String,
}

#[derive(Deserialize)]
struct IssueCertificateResponse {
    certificate: String,
    ca_public_key: String,
}

// Adding `profile` to honor the user's telemetry consent when building
// the HTTP client. Refactoring these args into a struct is out of scope
// for the consent fix; the arg count warning is acknowledged here.
#[allow(clippy::too_many_arguments)]
async fn issue_certificate(
    base_url: &str,
    service_id: &str,
    token: &str,
    public_key_file: &Path,
    principal: &str,
    certificate_file: &Path,
    ca_public_key_file: Option<&Path>,
    profile: Option<&str>,
) -> Result<()> {
    let public_key = tokio::fs::read_to_string(public_key_file)
        .await
        .with_context(|| {
            format!(
                "Failed to read public key file {}",
                public_key_file.display()
            )
        })?;

    let request = IssueCertificateRequest {
        public_key,
        principal: principal.to_string(),
    };

    let endpoint = build_issue_cert_url(base_url, service_id)?;
    let client =
        build_cli_http_client(profile).context("Failed to build SSH certificate HTTP client")?;

    let response = client
        .post(endpoint)
        .bearer_auth(token)
        .json(&request)
        .send()
        .await
        .context("Failed to request SSH certificate from NyxID")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("NyxID SSH certificate request failed with HTTP {status}: {body}");
    }

    let issued: IssueCertificateResponse = response
        .json()
        .await
        .context("Failed to decode SSH certificate response")?;

    ensure_parent_dir(certificate_file).await?;
    tokio::fs::write(certificate_file, issued.certificate.as_bytes())
        .await
        .with_context(|| {
            format!(
                "Failed to write certificate file {}",
                certificate_file.display()
            )
        })?;

    if let Some(path) = ca_public_key_file {
        ensure_parent_dir(path).await?;
        tokio::fs::write(path, issued.ca_public_key.as_bytes())
            .await
            .with_context(|| format!("Failed to write CA public key file {}", path.display()))?;
    }

    Ok(())
}

// ---- WebSocket proxy (ported from ssh_cli.rs) ----

async fn run_proxy(base_url: &str, service_id: &str, token: &str) -> Result<()> {
    let ws_url = build_ws_url(base_url, service_id)?;
    let mut request = ws_url.into_client_request()?;
    request
        .headers_mut()
        .insert(AUTHORIZATION, bearer_header(token)?);

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("Failed to open SSH WebSocket tunnel")?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut read_buf = vec![0_u8; 16 * 1024];

    loop {
        tokio::select! {
            read = stdin.read(&mut read_buf) => {
                let count = read.context("Failed to read SSH client stdin")?;
                if count == 0 {
                    let _ = ws_sink.close().await;
                    break;
                }
                ws_sink
                    .send(Message::Binary(read_buf[..count].to_vec().into()))
                    .await
                    .context("Failed to send SSH bytes to NyxID tunnel")?;
            }
            message = ws_stream.next() => {
                match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        stdout
                            .write_all(&bytes)
                            .await
                            .context("Failed to write SSH bytes to stdout")?;
                        stdout.flush().await.context("Failed to flush SSH stdout")?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ws_sink
                            .send(Message::Pong(payload))
                            .await
                            .context("Failed to respond to WebSocket ping")?;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => bail!("NyxID SSH tunnel returned an unexpected text frame"),
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => return Err(error).context("SSH WebSocket tunnel failed"),
                }
            }
        }
    }

    Ok(())
}

// ---- Config generation (ported from ssh_cli.rs) ----

#[allow(clippy::too_many_arguments)]
fn run_config(
    host_alias: &str,
    base_url: &str,
    service_id: &str,
    principal: &str,
    identity_file: &Path,
    certificate_file: &Path,
    access_token_env: &str,
    ca_public_key_file: Option<&Path>,
) -> Result<()> {
    let identity_str = identity_file.display().to_string();
    let public_key_file = public_key_file_for_identity(identity_file)?;

    let mut proxy_command = vec![
        "nyxid".to_string(),
        "ssh".to_string(),
        "proxy".to_string(),
        "--base-url".to_string(),
        base_url.to_string(),
        "--service-id".to_string(),
        service_id.to_string(),
        "--access-token-env".to_string(),
        access_token_env.to_string(),
        "--issue-certificate".to_string(),
        "--public-key-file".to_string(),
        public_key_file.display().to_string(),
        "--principal".to_string(),
        principal.to_string(),
        "--certificate-file".to_string(),
        certificate_file.display().to_string(),
    ];

    if let Some(path) = ca_public_key_file {
        proxy_command.push("--ca-public-key-file".to_string());
        proxy_command.push(path.display().to_string());
    }

    println!("Host {host_alias}");
    println!("  HostName {host_alias}");
    println!("  User {principal}");
    println!("  IdentityFile {identity_str}");
    println!("  CertificateFile {}", certificate_file.display());
    println!(
        "  ProxyCommand {}",
        proxy_command
            .into_iter()
            .map(|arg| shell_escape(&arg))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("  IdentitiesOnly yes");

    Ok(())
}

// ---- Helpers ----

fn build_issue_cert_url(base_url: &str, service_id: &str) -> Result<String> {
    let mut url = url::Url::parse(base_url).context("Invalid NyxID base URL")?;
    url.set_path(&format!("/api/v1/ssh/{service_id}/certificate"));
    url.set_query(None);
    Ok(url.to_string())
}

fn build_ws_url(base_url: &str, service_id: &str) -> Result<String> {
    let mut url = url::Url::parse(base_url).context("Invalid NyxID base URL")?;
    match url.scheme() {
        "https" => url.set_scheme("wss").expect("valid scheme transition"),
        "http" => url.set_scheme("ws").expect("valid scheme transition"),
        "wss" | "ws" => {}
        scheme => bail!("Unsupported NyxID base URL scheme: {scheme}"),
    }
    url.set_path(&format!("/api/v1/ssh/{service_id}"));
    url.set_query(None);
    Ok(url.to_string())
}

fn bearer_header(token: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(&format!("Bearer {token}")).context("Invalid bearer token header")
}

fn public_key_file_for_identity(identity_file: &Path) -> Result<PathBuf> {
    let file_name = identity_file
        .file_name()
        .and_then(|name| name.to_str())
        .context("Identity file must have a valid UTF-8 file name")?;
    Ok(identity_file.with_file_name(format!("{file_name}.pub")))
}

async fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    Ok(())
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::{build_issue_cert_url, build_ws_url, public_key_file_for_identity, shell_escape};
    use std::path::PathBuf;

    #[test]
    fn builds_ws_url_from_https_base() {
        assert_eq!(
            build_ws_url("https://nyxid.example.com/base", "svc-1").expect("ws url"),
            "wss://nyxid.example.com/api/v1/ssh/svc-1"
        );
    }

    #[test]
    fn builds_issue_cert_url_from_http_base() {
        assert_eq!(
            build_issue_cert_url("http://localhost:3000", "svc-2").expect("cert url"),
            "http://localhost:3000/api/v1/ssh/svc-2/certificate"
        );
    }

    #[test]
    fn derives_public_key_file_path() {
        assert_eq!(
            public_key_file_for_identity(&PathBuf::from("/tmp/id_ed25519")).expect("pub key"),
            PathBuf::from("/tmp/id_ed25519.pub")
        );
    }

    #[test]
    fn shell_escape_handles_single_quotes() {
        assert_eq!(shell_escape("a'b"), "'a'\"'\"'b'");
    }
}
