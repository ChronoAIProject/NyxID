use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};
use tokio::time::Instant;
use zeroize::{Zeroize, Zeroizing};

use super::agent_key;
use crate::{
    api::{build_credential_http_client, device_login_user_agent},
    cli::{LoginArgs, LoginCommands, OutputFormat},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginError {
    Pending,
    Denied,
    Expired,
    Delivered,
    RateLimited,
    Busy,
    Missing,
    DestinationMismatch,
    InvalidCode,
    Unavailable,
    Unsupported,
    Storage,
}

impl LoginError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Pending => "login_pending",
            Self::Denied => "login_denied",
            Self::Expired => "login_expired",
            Self::Delivered => "login_already_delivered",
            Self::RateLimited => "login_rate_limited",
            Self::Busy => "login_resume_busy",
            Self::Missing => "login_request_not_found",
            Self::DestinationMismatch => "login_destination_mismatch",
            Self::InvalidCode => "login_code_invalid",
            Self::Unavailable => "login_unavailable",
            Self::Unsupported => "login_unsupported",
            Self::Storage => "login_storage_failed",
        }
    }
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Pending => 10,
            Self::Denied => 11,
            Self::Expired => 12,
            Self::Delivered => 13,
            Self::RateLimited => 14,
            Self::Busy => 15,
            Self::Missing => 16,
            Self::DestinationMismatch => 17,
            Self::InvalidCode => 18,
            Self::Unavailable => 19,
            Self::Unsupported => 20,
            Self::Storage => 21,
        }
    }
    pub fn json(self) -> serde_json::Value {
        serde_json::json!({"error": {"code": self.code(), "message": self.to_string()}})
    }
}
impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pending => "Login is awaiting approval; resume this request later.",
            Self::Denied => "Login was denied. Start a new request to try again.",
            Self::Expired => "Login expired. Start a new request.",
            Self::Delivered => "This login has already been delivered. Check the saved profile with whoami.",
            Self::RateLimited => "Login is rate limited. Wait before retrying this request.",
            Self::Busy => "Another process is resuming this login.",
            Self::Missing => "No local pending login matches this handle.",
            Self::DestinationMismatch => "The requested destination or profile differs from the saved login.",
            Self::InvalidCode => "The login code is invalid or cancelled.",
            Self::Unavailable => "Login could not complete. Check connectivity and resume later.",
            Self::Unsupported => "This backend does not support the requested login flow. Upgrade the server or use browser login.",
            Self::Storage => "The credential could not be saved. Start a new login after checking local storage.",
        })
    }
}
impl std::error::Error for LoginError {}

#[derive(Serialize, Deserialize)]
struct PendingLogin {
    version: u8,
    request_id: String,
    base_url: String,
    profile: Option<String>,
    flow: String,
    device_code: String,
    expires_at: DateTime<Utc>,
    interval: u64,
    #[serde(default)]
    next_poll_at: Option<DateTime<Utc>>,
    state: String,
}
impl Drop for PendingLogin {
    fn drop(&mut self) {
        self.device_code.zeroize();
    }
}

#[derive(Deserialize)]
struct Challenge {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Delivery {
    AgentKey(Box<agent_key::Delivery>),
    Account {
        access_token: String,
        refresh_token: String,
    },
}

pub(super) fn normalized_destination(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw).map_err(|_| LoginError::DestinationMismatch)?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(LoginError::DestinationMismatch.into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn pending_path(id: &str) -> Result<PathBuf> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| LoginError::Missing)?;
    let dir = super::token_dir_for_profile(None)
        .map_err(|_| LoginError::Storage)?
        .join("pending-logins");
    fs::create_dir_all(&dir).map_err(|_| LoginError::Storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| LoginError::Storage)?;
    }
    Ok(dir.join(format!("{id}.json")))
}

fn save_pending(path: &Path, pending: &PendingLogin) -> Result<()> {
    let encoded = Zeroizing::new(serde_json::to_string(pending)?);
    super::write_token_file(path, &encoded).map_err(|_| LoginError::Storage.into())
}

fn load_pending(path: &Path) -> Result<PendingLogin> {
    let metadata = fs::symlink_metadata(path).map_err(|_| LoginError::Missing)?;
    if !metadata.is_file() || metadata.len() > 16384 {
        return Err(LoginError::Missing.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(LoginError::Storage.into());
        }
    }
    let encoded = Zeroizing::new(fs::read(path).map_err(|_| LoginError::Missing)?);
    serde_json::from_slice(&encoded).map_err(|_| LoginError::Missing.into())
}

fn lock_request(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path.with_extension("lock"))
        .map_err(|_| LoginError::Storage)?;
    file.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => LoginError::Busy,
        fs::TryLockError::Error(_) => LoginError::Storage,
    })?;
    Ok(file)
}

fn next_poll_time(now: DateTime<Utc>, interval: u64, expires_at: DateTime<Utc>) -> DateTime<Utc> {
    i64::try_from(interval)
        .ok()
        .and_then(Duration::try_seconds)
        .and_then(|interval| now.checked_add_signed(interval))
        .unwrap_or(expires_at)
        .min(expires_at)
}

fn next_poll_deadline(interval: u64, expires_at: Instant) -> Instant {
    Instant::now()
        .checked_add(std::time::Duration::from_secs(interval))
        .unwrap_or(expires_at)
        .min(expires_at)
}

fn schedule_next_poll(
    path: &Path,
    pending: &mut PendingLogin,
    expires_at: Instant,
) -> Result<Instant> {
    pending.next_poll_at = Some(next_poll_time(
        Utc::now(),
        pending.interval,
        pending.expires_at,
    ));
    let deadline = next_poll_deadline(pending.interval, expires_at);
    save_pending(path, pending)?;
    Ok(deadline)
}

pub async fn run(args: LoginArgs) -> Result<()> {
    if let Some(profile) = args.profile.as_deref() {
        super::validate_profile_name(profile)?;
    }
    if let Some(LoginCommands::Resume { request_id, once }) = &args.command {
        if args.device || args.agent_key || args.no_wait || args.code.is_some() {
            return Err(LoginError::DestinationMismatch.into());
        }
        return resume(
            request_id,
            args.base_url.as_deref(),
            args.profile.as_deref(),
            args.output,
            *once,
        )
        .await;
    }
    let base_url = normalized_destination(
        args.base_url
            .as_deref()
            .unwrap_or(super::DEFAULT_LOGIN_BASE_URL),
    )?;
    if let Some(code) = args.code {
        let code = Zeroizing::new(if code.is_empty() {
            if !std::io::stdin().is_terminal() {
                return Err(LoginError::InvalidCode.into());
            }
            rpassword::prompt_password("One-time login code: ")
                .map_err(|_| LoginError::InvalidCode)?
        } else {
            code
        });
        return redeem(&base_url, args.profile.as_deref(), &code, args.output).await;
    }
    if !args.device
        && !args.agent_key
        && !args.no_wait
        && super::is_ci_environment()
        && std::env::var_os("NYXID_LOGIN_NO_DEVICE_FALLBACK").is_none()
    {
        anyhow::bail!(
            "Use nyxid login --device --no-wait for human-approved CI login, or configure NYXID_API_KEY."
        );
    }
    let flow = if args.agent_key {
        "agent-key"
    } else {
        "device/v2"
    };
    let client = build_credential_http_client(args.profile.as_deref())?;
    let response = client.post(format!("{base_url}/api/v1/auth/{flow}/request"))
        .timeout(std::time::Duration::from_secs(30))
        .json(&serde_json::json!({"client_label": super::client_label(), "client_user_agent": device_login_user_agent(), "requested_profile": args.profile.as_deref().unwrap_or("default")}))
        .send().await.map_err(|_| LoginError::Unavailable)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        if flow == "device/v2" && !args.no_wait && !matches!(args.output, OutputFormat::Json) {
            return super::run_browser_login(&base_url, args.profile.as_deref())
                .await
                .map_err(Into::into);
        }
        return Err(LoginError::Unsupported.into());
    }
    if !response.status().is_success() {
        return Err(response_error(response).await.into());
    }
    let challenge: Challenge = response.json().await.map_err(|_| LoginError::Unavailable)?;
    let mut verification =
        url::Url::parse(&challenge.verification_uri).map_err(|_| LoginError::Unavailable)?;
    if !matches!(verification.scheme(), "http" | "https")
        || !verification.username().is_empty()
        || verification.password().is_some()
    {
        return Err(LoginError::Unavailable.into());
    }
    verification.set_query(None);
    verification.set_fragment(None);
    let id = uuid::Uuid::new_v4().to_string();
    let pending = PendingLogin {
        version: 1,
        request_id: id.clone(),
        base_url: base_url.clone(),
        profile: args.profile.clone(),
        flow: flow.to_owned(),
        device_code: challenge.device_code,
        expires_at: Utc::now() + Duration::seconds(challenge.expires_in.min(3600) as i64 + 60),
        interval: challenge.interval.max(5),
        next_poll_at: None,
        state: "pending".into(),
    };
    let path = pending_path(&id)?;
    save_pending(&path, &pending)?;
    if matches!(args.output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::json!({"request_id": id, "user_code": challenge.user_code,
            "verification_uri": verification.as_str(), "expires_at": pending.expires_at - Duration::seconds(60),
            "interval": pending.interval, "profile": pending.profile, "auth_kind": "pending"})
        );
    } else {
        eprintln!(
            "One-time code: {}\nOpen {} and enter the code.\nResume: nyxid login resume {}",
            challenge.user_code, verification, id
        );
    }
    if args.no_wait {
        return Ok(());
    }
    if !matches!(args.output, OutputFormat::Json) {
        let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        let open = if !args.device && !args.agent_key {
            true
        } else if interactive {
            eprint!("Open in your browser? [Y/n] ");
            std::io::stderr().flush()?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            matches!(
                answer.trim().to_ascii_lowercase().as_str(),
                "" | "y" | "yes"
            )
        } else {
            false
        };
        if open && crate::browser::open_browser(verification.as_str()).is_err() {
            eprintln!("Could not open a browser. Open the displayed URL on another device.");
        }
    }
    resume(
        &id,
        Some(&base_url),
        args.profile.as_deref(),
        args.output,
        false,
    )
    .await
}

async fn resume(
    id: &str,
    base_url: Option<&str>,
    profile: Option<&str>,
    output: OutputFormat,
    once: bool,
) -> Result<()> {
    let path = pending_path(id)?;
    let _lock = lock_request(&path)?;
    let mut pending = load_pending(&path)?;
    if pending.version != 1
        || pending.request_id != id
        || !matches!(pending.flow.as_str(), "device" | "device/v2" | "agent-key")
        || normalized_destination(&pending.base_url)? != pending.base_url
        || base_url
            .map(normalized_destination)
            .transpose()?
            .is_some_and(|base| base != pending.base_url)
        || profile.is_some_and(|p| pending.profile.as_deref() != Some(p))
    {
        return Err(LoginError::DestinationMismatch.into());
    }
    if let Some(profile) = pending.profile.as_deref() {
        super::validate_profile_name(profile)?;
    }
    if pending.state != "pending" {
        return Err(state_error(&pending.state).into());
    }
    let client = build_credential_http_client(pending.profile.as_deref())?;
    // Wall time survives process restarts; active waits use a fixed monotonic mapping.
    let wall_now = Utc::now();
    let monotonic_now = Instant::now();
    let deadline = |at: DateTime<Utc>| {
        monotonic_now
            .checked_add((at - wall_now).to_std().unwrap_or_default())
            .ok_or(LoginError::Missing)
    };
    let expires_at = deadline(pending.expires_at)?;
    let mut next_poll_at = pending
        .next_poll_at
        .map(|at| deadline(at.min(pending.expires_at)))
        .transpose()?;
    loop {
        if Instant::now() >= expires_at || Utc::now() >= pending.expires_at {
            return finish_error(&path, &mut pending, LoginError::Expired);
        }
        if once && next_poll_at.is_some_and(|next| next > Instant::now()) {
            return Err(LoginError::Pending.into());
        }
        if !once {
            let next =
                next_poll_at.unwrap_or_else(|| next_poll_deadline(pending.interval, expires_at));
            tokio::time::sleep_until(next).await;
        }
        if Instant::now() >= expires_at || Utc::now() >= pending.expires_at {
            return finish_error(&path, &mut pending, LoginError::Expired);
        }
        // Retain a crash checkpoint, then restart the full interval after the response.
        schedule_next_poll(&path, &mut pending, expires_at)?;
        let response = client
            .post(format!(
                "{}/api/v1/auth/{}/poll",
                pending.base_url, pending.flow
            ))
            .timeout(std::time::Duration::from_secs(30))
            .json(&serde_json::json!({"device_code": pending.device_code}))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                schedule_next_poll(&path, &mut pending, expires_at)?;
                return Err(LoginError::Unavailable.into());
            }
        };
        if response.status().is_success() {
            let value: serde_json::Value = match response.json().await {
                Ok(value) => value,
                Err(_) => {
                    schedule_next_poll(&path, &mut pending, expires_at)?;
                    return Err(LoginError::Unavailable.into());
                }
            };
            let result = store_delivery(
                &client,
                &pending.base_url,
                pending.profile.as_deref(),
                value,
                pending.flow == "agent-key",
            )
            .await;
            pending.device_code.zeroize();
            pending.state = "login_already_delivered".into();
            let terminal_saved = save_pending(&path, &pending).is_ok();
            if !terminal_saved {
                let _ = fs::remove_file(&path);
            }
            let saved = result?;
            report_delivery(saved, pending.profile.as_deref(), output, terminal_saved);
            return Ok(());
        }
        let (error, interval) = poll_error(response).await;
        if let Some(interval) = interval {
            pending.interval = pending.interval.saturating_add(5).max(interval);
        }
        next_poll_at = Some(schedule_next_poll(&path, &mut pending, expires_at)?);
        if error == LoginError::Pending && !once {
            continue;
        }
        return finish_error(&path, &mut pending, error);
    }
}

fn state_error(state: &str) -> LoginError {
    match state {
        "login_denied" => LoginError::Denied,
        "login_expired" => LoginError::Expired,
        _ => LoginError::Delivered,
    }
}
fn finish_error(path: &Path, pending: &mut PendingLogin, error: LoginError) -> Result<()> {
    if matches!(
        error,
        LoginError::Denied | LoginError::Expired | LoginError::Delivered | LoginError::InvalidCode
    ) {
        pending.device_code.zeroize();
        pending.state = error.code().into();
        save_pending(path, pending)?;
    }
    Err(error.into())
}

async fn poll_error(response: reqwest::Response) -> (LoginError, Option<u64>) {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let value: serde_json::Value = response.json().await.unwrap_or_default();
    let code = value.get("error_code").and_then(serde_json::Value::as_i64);
    let interval = value
        .get("interval")
        .and_then(serde_json::Value::as_u64)
        .or(retry_after)
        .unwrap_or(0);
    match code {
        Some(11202 | 11902) => (LoginError::Pending, None),
        Some(11203 | 11903) => (LoginError::Pending, Some(interval)),
        Some(11204 | 11904) => (LoginError::Denied, None),
        Some(11200 | 11201 | 11900 | 11901) => (LoginError::Expired, None),
        Some(11205 | 11905) => (LoginError::Delivered, None),
        Some(11206 | 11906) => (LoginError::RateLimited, retry_after),
        Some(12000 | 12002) => (LoginError::InvalidCode, None),
        Some(12001) => (LoginError::Expired, None),
        Some(12003) => (LoginError::Delivered, None),
        Some(12004) => (LoginError::RateLimited, retry_after),
        _ if status == reqwest::StatusCode::TOO_MANY_REQUESTS => {
            (LoginError::RateLimited, retry_after)
        }
        _ => (LoginError::Unavailable, None),
    }
}
async fn response_error(response: reqwest::Response) -> LoginError {
    poll_error(response).await.0
}

async fn redeem(
    base_url: &str,
    profile: Option<&str>,
    code: &str,
    output: OutputFormat,
) -> Result<()> {
    let client = build_credential_http_client(profile)?;
    let response = client.post(format!("{base_url}/api/v1/auth/login-code/redeem"))
        .timeout(std::time::Duration::from_secs(30))
        .json(&serde_json::json!({"code": code, "client_label": super::client_label(), "client_user_agent": device_login_user_agent(), "requested_profile": profile}))
        .send().await.map_err(|_| LoginError::Unavailable)?;
    if !response.status().is_success() {
        return Err(response_error(response).await.into());
    }
    let value = response.json().await.map_err(|_| LoginError::Unavailable)?;
    let saved = store_delivery(&client, base_url, profile, value, false).await?;
    report_delivery(saved, profile, output, true);
    Ok(())
}

struct StoredDelivery {
    identity: serde_json::Value,
    description: String,
}

async fn store_delivery(
    client: &reqwest::Client,
    base_url: &str,
    profile: Option<&str>,
    value: serde_json::Value,
    restricted_only: bool,
) -> Result<StoredDelivery> {
    let delivery: Delivery = serde_json::from_value(value).map_err(|_| LoginError::Unavailable)?;
    let identity = match delivery {
        Delivery::AgentKey(delivery) => {
            let credential = Zeroizing::new(delivery.credential);
            if credential.len() != 73
                || !credential.starts_with("nyxid_ag_")
                || !credential[9..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(LoginError::Unavailable.into());
            }
            if agent_key::save(profile, base_url, &credential, &delivery.identity).is_err() {
                let _ = client
                    .delete(format!("{base_url}/api/v1/auth/agent-key/self"))
                    .bearer_auth(credential.as_str())
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;
                return Err(LoginError::Storage.into());
            }
            StoredDelivery {
                description: agent_key::format_identity(&delivery.identity),
                identity: serde_json::json!({"auth_kind": "agent_key", "identity": delivery.identity}),
            }
        }
        Delivery::Account {
            access_token,
            refresh_token,
        } => {
            let access = Zeroizing::new(access_token);
            let refresh = Zeroizing::new(refresh_token);
            if restricted_only {
                let _ = client
                    .post(format!("{base_url}/api/v1/auth/logout"))
                    .bearer_auth(access.as_str())
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;
                return Err(LoginError::Unavailable.into());
            }
            if access.is_empty() || refresh.is_empty() {
                return Err(LoginError::Unavailable.into());
            }
            let result = super::replace_login_files(profile, || {
                super::save_base_url_for(profile, base_url)?;
                super::save_tokens_for(profile, &access, Some(&refresh))
            });
            if result.is_err() {
                let _ = client
                    .post(format!("{base_url}/api/v1/auth/logout"))
                    .bearer_auth(access.as_str())
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;
                return Err(LoginError::Storage.into());
            }
            StoredDelivery {
                description: "Signed in with an account session.".into(),
                identity: serde_json::json!({"auth_kind": "account_session", "user_id": super::read_saved_user_id_for(profile)}),
            }
        }
    };
    Ok(identity)
}

fn report_delivery(
    saved: StoredDelivery,
    profile: Option<&str>,
    output: OutputFormat,
    terminal_saved: bool,
) {
    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::json!({"status": "authenticated", "profile": profile, "auth": saved.identity,
            "resume_state_saved": terminal_saved})
        );
    } else {
        eprintln!("{}", saved.description);
    }
    if !terminal_saved {
        eprintln!(
            "Credentials were saved, but the resume record could not be finalized. Use whoami to check this profile; this request cannot issue another credential."
        );
    }
}

#[cfg(test)]
mod tests;
