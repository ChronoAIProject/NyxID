use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde::Deserialize;

const TOKEN_DIR_NAME: &str = ".nyxid";
const TOKEN_FILE_NAME: &str = "access_token";

#[derive(Args)]
pub struct LoginArgs {
    /// NyxID base URL, e.g. https://nyxid.example.com
    #[arg(long)]
    base_url: String,
    /// Email address for login (prompted if omitted).
    #[arg(long)]
    email: Option<String>,
    /// Password for login (prompted securely if omitted).
    #[arg(long, hide = true)]
    password: Option<String>,
}

#[derive(Deserialize)]
struct LoginResponse {
    access_token: String,
}

pub async fn run(args: LoginArgs) -> Result<()> {
    let email = match args.email {
        Some(email) => email,
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

    let password = match args.password {
        Some(password) => password,
        None => rpassword::prompt_password("Password: ").context("Failed to read password")?,
    };

    if password.is_empty() {
        bail!("Password is required");
    }

    let base_url = args.base_url.trim_end_matches('/');
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
    use super::token_file_path;

    #[test]
    fn token_path_is_under_home() {
        let path = token_file_path().expect("token path");
        assert!(path.to_string_lossy().contains(".nyxid"));
        assert!(path.to_string_lossy().ends_with("access_token"));
    }
}
