use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde::Deserialize;

const TOKEN_DIR_NAME: &str = ".nyxid";
const TOKEN_FILE_NAME: &str = "access_token";
const CALLBACK_TIMEOUT_SECS: u64 = 120;

#[derive(Args)]
pub struct LoginArgs {
    /// NyxID base URL, e.g. https://nyxid.example.com
    #[arg(long)]
    base_url: String,
    /// Use email/password login instead of opening the browser.
    #[arg(long)]
    password: bool,
    /// Email address (only used with --password).
    #[arg(long)]
    email: Option<String>,
}

#[derive(Deserialize)]
struct LoginResponse {
    access_token: String,
}

pub async fn run(args: LoginArgs) -> Result<()> {
    if args.password {
        return run_password_login(&args.base_url, args.email.as_deref()).await;
    }

    run_browser_login(&args.base_url).await
}

/// Browser-based login: start a localhost server, open the NyxID portal,
/// wait for the callback with the access token.
async fn run_browser_login(base_url: &str) -> Result<()> {
    let base_url = base_url.trim_end_matches('/');

    // Ask the backend for the correct frontend URL
    let frontend_url = fetch_frontend_url(base_url).await?;

    // Bind to a random available port
    let listener =
        TcpListener::bind("127.0.0.1:0").context("Failed to bind local callback server")?;
    let port = listener.local_addr()?.port();

    let state = generate_state();
    let auth_url = format!("{frontend_url}/cli-auth?port={port}&state={state}",);

    eprintln!("Opening browser to log in...");
    eprintln!();
    eprintln!("If the browser does not open, visit:");
    eprintln!("  {auth_url}");
    eprintln!();

    // Try to open the browser
    let _ = open::that(&auth_url);

    // Wait for the callback
    let token = wait_for_callback(listener, &state).await?;
    save_token(&token)?;

    eprintln!("Logged in successfully.");
    eprintln!("Token saved to {}", token_file_path()?.display());

    Ok(())
}

/// Wait for the browser to redirect to our localhost callback with the token.
async fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    listener
        .set_nonblocking(true)
        .context("Failed to set listener to non-blocking")?;
    let listener =
        tokio::net::TcpListener::from_std(listener).context("Failed to create async listener")?;

    let timeout = tokio::time::sleep(std::time::Duration::from_secs(CALLBACK_TIMEOUT_SECS));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (mut stream, _) = accept.context("Failed to accept connection")?;
                let mut buf = vec![0u8; 4096];
                let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await
                    .context("Failed to read request")?;
                let request = String::from_utf8_lossy(&buf[..n]);

                // Parse the GET request for /callback?access_token=...&state=...
                if let Some(token) = parse_callback_request(&request, expected_state) {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{}",
                        callback_success_html()
                    );
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
                    return Ok(token);
                }

                // Not our callback -- respond and keep listening
                let response = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
            () = &mut timeout => {
                bail!("Login timed out after {CALLBACK_TIMEOUT_SECS}s. Please try again.");
            }
        }
    }
}

fn parse_callback_request(request: &str, expected_state: &str) -> Option<String> {
    let first_line = request.lines().next()?;
    // e.g. "GET /callback?access_token=xxx&state=yyy HTTP/1.1"
    let path = first_line.split_whitespace().nth(1)?;
    if !path.starts_with("/callback") {
        return None;
    }

    let query = path.split('?').nth(1)?;
    let params: std::collections::HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    let state = *params.get("state")?;
    if state != expected_state {
        return None;
    }

    let token = *params.get("access_token")?;
    if token.is_empty() {
        return None;
    }

    Some(urlencoding::decode(token).ok()?.into_owned())
}

fn generate_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn callback_success_html() -> &'static str {
    r#"<!doctype html>
<html>
<head><title>NyxID CLI</title></head>
<body style="display:flex;align-items:center;justify-content:center;min-height:100vh;font-family:system-ui;background:#0f172a;color:#e2e8f0">
<div style="text-align:center">
<h2>Login successful</h2>
<p style="color:#94a3b8">You can close this tab and return to your terminal.</p>
</div>
</body>
</html>"#
}

async fn fetch_frontend_url(base_url: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct PublicConfig {
        frontend_url: String,
    }

    let config_url = format!("{base_url}/api/v1/public/config");
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client")?;

    let config: PublicConfig = client
        .get(&config_url)
        .send()
        .await
        .context("Failed to reach NyxID server")?
        .json()
        .await
        .context("Failed to parse server config")?;

    Ok(config.frontend_url.trim_end_matches('/').to_string())
}

/// Email/password login fallback for headless environments.
async fn run_password_login(base_url: &str, email: Option<&str>) -> Result<()> {
    let email = match email {
        Some(email) => email.to_string(),
        None => {
            eprint!("Email: ");
            std::io::stderr().flush()?;
            let mut email = String::new();
            std::io::stdin()
                .read_line(&mut email)
                .context("Failed to read email")?;
            email.trim().to_string()
        }
    };

    if email.is_empty() {
        bail!("Email is required");
    }

    let password = rpassword::prompt_password("Password: ").context("Failed to read password")?;
    if password.is_empty() {
        bail!("Password is required");
    }

    let base_url = base_url.trim_end_matches('/');
    let login_url = format!("{base_url}/api/v1/auth/login");

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client")?;

    let response = client
        .post(&login_url)
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "client": "cli",
        }))
        .send()
        .await
        .context("Failed to connect to NyxID server")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Login failed (HTTP {status}): {body}");
    }

    let login: LoginResponse = response
        .json()
        .await
        .context("Failed to parse login response")?;

    save_token(&login.access_token)?;

    eprintln!("Logged in as {email}");
    eprintln!("Token saved to {}", token_file_path()?.display());

    Ok(())
}

/// Read a previously saved access token (used by `ssh_cli` when no token is
/// provided via flag or env var).
pub fn read_saved_token() -> Option<String> {
    let path = token_file_path().ok()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn save_token(token: &str) -> Result<()> {
    let path = token_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    std::fs::write(&path, token)
        .with_context(|| format!("Failed to write token to {}", path.display()))?;

    // Restrict file permissions to owner-only on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }

    Ok(())
}

fn token_file_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(TOKEN_DIR_NAME).join(TOKEN_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::{callback_success_html, parse_callback_request, token_file_path};

    #[test]
    fn token_path_is_under_home() {
        let path = token_file_path().expect("token path");
        assert!(path.to_string_lossy().contains(".nyxid"));
        assert!(path.to_string_lossy().ends_with("access_token"));
    }

    #[test]
    fn parses_valid_callback_request() {
        let request =
            "GET /callback?access_token=tok_abc123&state=deadbeef HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        assert_eq!(
            parse_callback_request(request, "deadbeef"),
            Some("tok_abc123".to_string())
        );
    }

    #[test]
    fn rejects_wrong_state() {
        let request =
            "GET /callback?access_token=tok_abc123&state=wrong HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        assert_eq!(parse_callback_request(request, "deadbeef"), None);
    }

    #[test]
    fn rejects_non_callback_path() {
        let request = "GET /other?access_token=tok_abc123&state=deadbeef HTTP/1.1\r\n";
        assert_eq!(parse_callback_request(request, "deadbeef"), None);
    }

    #[test]
    fn success_html_is_not_empty() {
        assert!(callback_success_html().contains("Login successful"));
    }
}
