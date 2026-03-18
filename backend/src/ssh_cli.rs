use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use axum::http::{HeaderValue, header::AUTHORIZATION};
use clap::{Args, Subcommand};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[derive(clap::Args)]
pub struct SshCli {
    #[command(subcommand)]
    command: SshCommand,
}

#[derive(Subcommand)]
enum SshCommand {
    /// Issue a short-lived SSH certificate for a public key.
    IssueCert(IssueCertArgs),
    /// Open an SSH-over-WebSocket tunnel for use as an OpenSSH ProxyCommand.
    Proxy(ProxyArgs),
    /// Print an example OpenSSH config stanza for NyxID SSH access.
    Config(ConfigArgs),
}

#[derive(Args)]
struct AuthArgs {
    /// NyxID base URL, e.g. https://nyxid.example.com
    #[arg(long)]
    base_url: String,
    /// Downstream service ID registered in NyxID.
    #[arg(long)]
    service_id: String,
    /// Access token to use for NyxID API authentication.
    #[arg(long)]
    access_token: Option<String>,
    /// Environment variable to read the access token from when --access-token is omitted.
    #[arg(long, default_value = "NYXID_ACCESS_TOKEN")]
    access_token_env: String,
}

#[derive(Args)]
struct CertArgs {
    /// OpenSSH public key file to send to NyxID for certificate issuance.
    #[arg(long)]
    public_key_file: PathBuf,
    /// SSH principal to request from NyxID.
    #[arg(long)]
    principal: String,
    /// Where to write the issued OpenSSH certificate.
    #[arg(long)]
    certificate_file: PathBuf,
    /// Optional path to also write the SSH CA public key.
    #[arg(long)]
    ca_public_key_file: Option<PathBuf>,
}

#[derive(Args)]
struct IssueCertArgs {
    #[command(flatten)]
    auth: AuthArgs,
    #[command(flatten)]
    cert: CertArgs,
}

#[derive(Args)]
struct ProxyArgs {
    #[command(flatten)]
    auth: AuthArgs,
    /// Issue or refresh an SSH certificate before opening the tunnel.
    #[arg(long, default_value_t = false)]
    issue_certificate: bool,
    #[command(flatten)]
    cert: Option<CertArgs>,
}

#[derive(Args)]
struct ConfigArgs {
    /// SSH host alias to print in the generated config stanza.
    #[arg(long)]
    host_alias: String,
    /// NyxID base URL, e.g. https://nyxid.example.com
    #[arg(long)]
    base_url: String,
    /// Downstream service ID registered in NyxID.
    #[arg(long)]
    service_id: String,
    /// SSH user/principal to request from NyxID.
    #[arg(long)]
    principal: String,
    /// Private key file used by ssh(1). Its .pub sibling is used for certificate issuance.
    #[arg(long)]
    identity_file: PathBuf,
    /// Where the helper should write the short-lived certificate.
    #[arg(long)]
    certificate_file: PathBuf,
    /// Environment variable used by ProxyCommand for the NyxID access token.
    #[arg(long, default_value = "NYXID_ACCESS_TOKEN")]
    access_token_env: String,
    /// Optional path where the helper should also write the SSH CA public key.
    #[arg(long)]
    ca_public_key_file: Option<PathBuf>,
}

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

pub async fn run(cli: SshCli) -> Result<()> {
    match cli.command {
        SshCommand::IssueCert(args) => run_issue_cert(args).await,
        SshCommand::Proxy(args) => run_proxy(args).await,
        SshCommand::Config(args) => run_config(args),
    }
}

async fn run_issue_cert(args: IssueCertArgs) -> Result<()> {
    let token = resolve_access_token(&args.auth)?;
    issue_certificate(
        &args.auth.base_url,
        &args.auth.service_id,
        &token,
        &args.cert,
    )
    .await
}

async fn run_proxy(args: ProxyArgs) -> Result<()> {
    let token = resolve_access_token(&args.auth)?;
    if args.issue_certificate {
        let cert = args.cert.as_ref().context(
            "--issue-certificate requires --public-key-file, --principal, and --certificate-file",
        )?;
        issue_certificate(&args.auth.base_url, &args.auth.service_id, &token, cert).await?;
    } else if args.cert.is_some() {
        bail!("certificate arguments require --issue-certificate when using `nyxid ssh proxy`");
    }

    let ws_url = build_ws_url(&args.auth.base_url, &args.auth.service_id)?;
    let mut request = ws_url.into_client_request()?;
    request
        .headers_mut()
        .insert(AUTHORIZATION, bearer_header(&token)?);

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

fn run_config(args: ConfigArgs) -> Result<()> {
    let identity_file = args.identity_file.display().to_string();
    let public_key_file = public_key_file_for_identity(&args.identity_file)?;
    let mut proxy_command = vec![
        "nyxid".to_string(),
        "ssh".to_string(),
        "proxy".to_string(),
        "--base-url".to_string(),
        args.base_url,
        "--service-id".to_string(),
        args.service_id,
        "--access-token-env".to_string(),
        args.access_token_env,
        "--issue-certificate".to_string(),
        "--public-key-file".to_string(),
        public_key_file.display().to_string(),
        "--principal".to_string(),
        args.principal.clone(),
        "--certificate-file".to_string(),
        args.certificate_file.display().to_string(),
    ];

    if let Some(path) = &args.ca_public_key_file {
        proxy_command.push("--ca-public-key-file".to_string());
        proxy_command.push(path.display().to_string());
    }

    println!("Host {}", args.host_alias);
    println!("  HostName ssh.invalid");
    println!("  User {}", args.principal);
    println!("  IdentityFile {}", identity_file);
    println!("  CertificateFile {}", args.certificate_file.display());
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

async fn issue_certificate(
    base_url: &str,
    service_id: &str,
    token: &str,
    cert: &CertArgs,
) -> Result<()> {
    let public_key = tokio::fs::read_to_string(&cert.public_key_file)
        .await
        .with_context(|| {
            format!(
                "Failed to read public key file {}",
                cert.public_key_file.display()
            )
        })?;

    let request = IssueCertificateRequest {
        public_key,
        principal: cert.principal.clone(),
    };
    let endpoint = build_issue_cert_url(base_url, service_id)?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to build SSH certificate HTTP client")?;
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

    ensure_parent_dir(&cert.certificate_file).await?;
    tokio::fs::write(&cert.certificate_file, issued.certificate.as_bytes())
        .await
        .with_context(|| {
            format!(
                "Failed to write certificate file {}",
                cert.certificate_file.display()
            )
        })?;

    if let Some(path) = &cert.ca_public_key_file {
        ensure_parent_dir(path).await?;
        tokio::fs::write(path, issued.ca_public_key.as_bytes())
            .await
            .with_context(|| format!("Failed to write CA public key file {}", path.display()))?;
    }

    Ok(())
}

fn resolve_access_token(auth: &AuthArgs) -> Result<String> {
    if let Some(token) = &auth.access_token {
        return Ok(token.clone());
    }

    std::env::var(&auth.access_token_env).with_context(|| {
        format!(
            "No access token provided. Set {} or pass --access-token",
            auth.access_token_env
        )
    })
}

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
