use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Write};
use zeroize::Zeroizing;

use super::{LoginSleeper, TokioLoginSleeper};
use crate::api::{ApiClient, ErrorEnvelope, build_cli_http_client, device_login_user_agent};

pub const TOKEN_FILE: &str = "token";
const AUTH_KIND_FILE: &str = "auth_kind";
const METADATA_FILE: &str = "agent_key.json";
const ERR_NOT_FOUND: i64 = 11900;
const ERR_EXPIRED: i64 = 11901;
const ERR_PENDING: i64 = 11902;
const ERR_SLOW_DOWN: i64 = 11903;
const ERR_DENIED: i64 = 11904;
const ERR_ALREADY_DELIVERED: i64 = 11905;
const ERR_RATE_LIMITED: i64 = 11906;
const ERR_USER_CODE_INVALID: i64 = 11907;
const ERR_KEY_INELIGIBLE: i64 = 11908;
const ERR_CREDENTIAL_NOT_FOUND: i64 = 11909;
pub const REJECTED_MESSAGE: &str = "Your Agent Key credential was rejected (revoked, expired, or the key was deleted). Run `nyxid login --agent-key` to authorize again.";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KeyMetadata {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub owner_type: String,
    pub owner_id: String,
    pub owner_name: String,
    pub scopes: String,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub expires_at: Option<String>,
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    pub platform: Option<String>,
    #[serde(default)]
    pub created_now: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Identity {
    pub api_key: KeyMetadata,
    pub credential_id: String,
    pub credential_expires_at: Option<String>,
    pub label: String,
}

#[derive(Deserialize)]
struct Delivery {
    credential: String,
    #[serde(flatten)]
    identity: Identity,
}

#[derive(Deserialize)]
struct Challenge {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentKeyDelivery").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Challenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentKeyChallenge").finish_non_exhaustive()
    }
}

pub fn is_agent_key_profile(profile: Option<&str>) -> bool {
    super::token_dir_for_profile(profile)
        .ok()
        .and_then(|dir| std::fs::read_to_string(dir.join(AUTH_KIND_FILE)).ok())
        .is_some_and(|value| value.trim() == "agent_key")
}

pub fn read_metadata(profile: Option<&str>) -> Option<Identity> {
    let path = super::token_dir_for_profile(profile)
        .ok()?
        .join(METADATA_FILE);
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn remove_if_present(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Failed to remove {}", path.display())),
    }
}

pub(super) fn clear_agent_files(profile: Option<&str>) -> Result<()> {
    let dir = super::token_dir_for_profile(profile)?;
    // Clear the mode marker last, after the restricted credential is gone.
    for name in [TOKEN_FILE, METADATA_FILE, AUTH_KIND_FILE] {
        remove_if_present(&dir.join(name))?;
    }
    Ok(())
}

fn save(
    profile: Option<&str>,
    base_url: &str,
    credential: &str,
    identity: &Identity,
) -> Result<()> {
    let _lock = super::acquire_refresh_lock(profile)?;
    let dir = super::token_dir_for_profile(profile)?;
    // Persist the mode first. Even an interrupted write must never expose a
    // stale account token as the fallback identity for this profile.
    super::write_token_file(&dir.join(AUTH_KIND_FILE), "agent_key")?;
    for name in [
        super::TOKEN_FILE_NAME,
        super::REFRESH_TOKEN_FILE_NAME,
        super::USER_ID_FILE_NAME,
    ] {
        remove_if_present(&dir.join(name))?;
    }
    super::write_token_file(
        &dir.join(METADATA_FILE),
        &serde_json::to_string_pretty(identity)?,
    )?;
    super::save_base_url_for(profile, base_url)?;
    super::write_token_file(&dir.join(TOKEN_FILE), credential)
}

pub async fn run_login(base_url: &str, profile: Option<&str>) -> Result<()> {
    run_login_with_sleeper(base_url, profile, &TokioLoginSleeper).await
}

async fn run_login_with_sleeper(
    base_url: &str,
    profile: Option<&str>,
    sleeper: &impl LoginSleeper,
) -> Result<()> {
    if let Some(profile) = profile {
        super::validate_profile_name(profile)?;
    }
    let base_url = base_url.trim_end_matches('/');
    let client = build_cli_http_client(profile)?;
    let response = client.post(format!("{base_url}/api/v1/auth/agent-key/request"))
        .json(&serde_json::json!({"client_label": super::client_label(), "client_user_agent": device_login_user_agent(), "requested_profile": profile.unwrap_or("default")}))
        .send().await.context("Agent Key login request failed")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("This NyxID backend doesn't support Agent Key login");
    }
    if !response.status().is_success() {
        bail!(
            "Agent Key login request failed (HTTP {})",
            response.status()
        );
    }
    let challenge: Challenge = response
        .json()
        .await
        .context("Invalid Agent Key login response")?;
    let device_code = Zeroizing::new(challenge.device_code);
    let mut verification_uri = url::Url::parse(&challenge.verification_uri)
        .context("Invalid Agent Key verification URL")?;
    if !matches!(verification_uri.scheme(), "http" | "https")
        || !verification_uri.username().is_empty()
        || verification_uri.password().is_some()
    {
        bail!("Invalid Agent Key verification URL");
    }
    verification_uri.set_query(None);
    verification_uri.set_fragment(None);
    eprintln!("! First copy your one-time code: {}", challenge.user_code);
    eprintln!("\nThen open {verification_uri} and enter the code above.");
    if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        eprint!("\nOpen in your browser? [Y/n] ");
        std::io::stderr().flush().ok();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_ok() {
            let answer = answer.trim().to_ascii_lowercase();
            if (answer.is_empty() || answer == "y" || answer == "yes")
                && let Err(error) = crate::browser::open_browser(verification_uri.as_str())
            {
                eprintln!("Could not open browser: {error}. Paste the URL above manually.");
            }
        }
    }
    let mut interval = challenge.interval.max(5);
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(challenge.expires_in.saturating_add(60));
    loop {
        sleeper.sleep(interval).await;
        if std::time::Instant::now() >= deadline {
            bail!("Agent Key login timed out. Run `nyxid login --agent-key` again.");
        }
        let response = client
            .post(format!("{base_url}/api/v1/auth/agent-key/poll"))
            .json(&serde_json::json!({"device_code": device_code.as_str()}))
            .send()
            .await
            .context("Agent Key login poll failed")?;
        if response.status().is_success() {
            let delivery: Delivery = response
                .json()
                .await
                .context("Invalid Agent Key delivery response")?;
            let credential = Zeroizing::new(delivery.credential);
            if credential.len() != 73
                || !credential.starts_with("nyxid_ag_")
                || !credential[9..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                bail!("Invalid Agent Key credential returned by the backend");
            }
            if let Err(error) = save(profile, base_url, &credential, &delivery.identity) {
                let _ = client
                    .delete(format!("{base_url}/api/v1/auth/agent-key/self"))
                    .bearer_auth(credential.as_str())
                    .send()
                    .await;
                return Err(error);
            }
            eprintln!("{}", format_identity(&delivery.identity));
            return Ok(());
        }
        let status = response.status();
        let error: ErrorEnvelope = response
            .json()
            .await
            .with_context(|| format!("Agent Key login poll failed (HTTP {status})"))?;
        interval = handle_poll_error(error.error_code, interval)?;
    }
}

fn handle_poll_error(code: i64, interval: u64) -> Result<u64> {
    match code {
        ERR_PENDING => Ok(interval),
        ERR_SLOW_DOWN => Ok(interval.saturating_add(5)),
        ERR_NOT_FOUND | ERR_EXPIRED => {
            bail!("Agent Key login timed out. Run `nyxid login --agent-key` again.")
        }
        ERR_DENIED => bail!("Agent Key login was denied."),
        ERR_ALREADY_DELIVERED => {
            bail!("This code was already used. Run `nyxid login --agent-key` again.")
        }
        ERR_RATE_LIMITED => bail!("Too many attempts. Try again in a few minutes."),
        ERR_USER_CODE_INVALID => {
            bail!("Invalid Agent Key login code. Run `nyxid login --agent-key` again.")
        }
        ERR_KEY_INELIGIBLE => {
            bail!("The selected key is no longer eligible. Run `nyxid login --agent-key` again.")
        }
        ERR_CREDENTIAL_NOT_FOUND => bail!(
            "The Agent Key login credential is no longer available. Run `nyxid login --agent-key` again."
        ),
        _ => bail!("Agent Key login failed (error {code})."),
    }
}

pub fn format_identity(identity: &Identity) -> String {
    let key = &identity.api_key;
    format!(
        "Authentication: Agent Key\nKey: {} ({})\nOwner: {} ({})\nScopes: {}\nServices: {} (allow all: {})\nNodes: {} (allow all: {})\nKey expiry: {}\nCredential expiry: {}\nCredential label: {}\nRate limit: {} requests/s; burst: {}",
        key.name,
        key.key_prefix,
        key.owner_name,
        key.owner_type,
        key.scopes,
        key.allowed_service_ids.len(),
        key.allow_all_services,
        key.allowed_node_ids.len(),
        key.allow_all_nodes,
        key.expires_at.as_deref().unwrap_or("None"),
        identity.credential_expires_at.as_deref().unwrap_or("None"),
        identity.label,
        key.rate_limit_per_second
            .map_or_else(|| "Default".into(), |v| v.to_string()),
        key.rate_limit_burst
            .map_or_else(|| "Default".into(), |v| v.to_string())
    )
}

pub async fn show_identity(api: &mut ApiClient, output: crate::cli::OutputFormat) -> Result<()> {
    let identity: Identity = api.get("/auth/agent-key/self").await?;
    match output {
        crate::cli::OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"auth": {"kind": "agent_key", "api_key": identity.api_key, "credential_id": identity.credential_id, "credential_expires_at": identity.credential_expires_at, "label": identity.label}})
            )?
        ),
        crate::cli::OutputFormat::Table => println!("{}", format_identity(&identity)),
    }
    Ok(())
}

pub(super) async fn run_logout(base_url: &str, profile: Option<&str>) -> Result<()> {
    let revoked = match (
        super::read_saved_token_for(profile),
        build_cli_http_client(profile),
    ) {
        (Some(token), Ok(client)) => client
            .delete(format!(
                "{}/api/v1/auth/agent-key/self",
                base_url.trim_end_matches('/')
            ))
            .bearer_auth(&token)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success()),
        _ => false,
    };
    let _lock = super::acquire_refresh_lock(profile)?;
    // Remove any old human files before removing the mode marker.
    let dir = super::token_dir_for_profile(profile)?;
    for name in [
        super::TOKEN_FILE_NAME,
        super::REFRESH_TOKEN_FILE_NAME,
        super::USER_ID_FILE_NAME,
    ] {
        remove_if_present(&dir.join(name))?;
    }
    clear_agent_files(profile)?;
    if let Some(client) = crate::telemetry::TelemetryClient::init(profile) {
        client.reset();
    }
    if revoked {
        eprintln!("Logged out. Agent Key credential revoked on the server and cleared locally.");
    } else {
        eprintln!(
            "Logged out. Local Agent Key credential cleared; server-side revocation could not be confirmed."
        );
    }
    Ok(())
}
