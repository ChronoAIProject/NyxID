use std::future::Future;
use std::io::{IsTerminal, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::pin::Pin;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, Utc};
use serde::Deserialize;

use crate::api::{
    AuthDevicePollBody, AuthDevicePollOutcome, AuthDeviceRequestBody, AuthDeviceRequestOutcome,
    CLI_USER_AGENT, build_cli_http_client,
};
use crate::cli::{AuthArgs, LoginArgs};

/// Default NyxID base URL used when prompting for re-login on a session that
/// was never associated with a saved base URL. Mirrors the `LoginArgs::base_url`
/// clap default in `cli.rs`; kept in sync so the prompt path and the explicit
/// `nyxid login` command target the same server by default.
const DEFAULT_LOGIN_BASE_URL: &str = "https://nyx-api.chrono-ai.fun";

/// Clock-skew cushion: an access token is treated as still usable only if it
/// has more than this many seconds of validity left. Small on purpose -- the
/// goal is to refresh tokens that just crossed the line, not to refresh tokens
/// that are still good for several minutes (every refresh rotates the token and
/// counts against the backend's reuse-detection grace window). If the local
/// clock is wrong, the worst case is one wasted refresh; `ApiClient`'s in-flight
/// 401-refresh is the backstop.
const SESSION_SKEW_SECS: i64 = 60;

const TOKEN_DIR_NAME: &str = ".nyxid";
const PROFILES_DIR_NAME: &str = "profiles";
const TOKEN_FILE_NAME: &str = "access_token";
const REFRESH_TOKEN_FILE_NAME: &str = "refresh_token";
const BASE_URL_FILE_NAME: &str = "base_url";
const USER_ID_FILE_NAME: &str = "user_id";
const CALLBACK_TIMEOUT_SECS: u64 = 120;

const AUTH_DEVICE_CODE_EXPIRED: i64 = 11201;
const AUTH_DEVICE_CODE_PENDING: i64 = 11202;
const AUTH_DEVICE_CODE_SLOW_DOWN: i64 = 11203;
const AUTH_DEVICE_CODE_DENIED: i64 = 11204;
const AUTH_DEVICE_CODE_ALREADY_DELIVERED: i64 = 11205;
const AUTH_DEVICE_CODE_RATE_LIMITED: i64 = 11206;

/// Extract the `sub` claim (NyxID user UUID) from a JWT access token.
/// Decodes the payload section only; does not verify the signature, since
/// the server already verified it when issuing the token. Returns `None`
/// for malformed tokens or tokens without a string `sub` claim.
pub fn jwt_sub_from_token(token: &str) -> Option<String> {
    jwt_claim_string_from_token(token, "sub")
}

pub fn jwt_claim_string_from_token(token: &str, claim: &str) -> Option<String> {
    let payload_b64 = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get(claim)?.as_str().map(|s| s.to_string())
}

pub fn jwt_exp_from_token(token: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let payload_b64 = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    let exp = json.get("exp")?.as_i64()?;
    chrono::DateTime::from_timestamp(exp, 0)
}

// ---- Profile validation ----

/// Validate a profile name: 1-64 characters, alphanumeric + hyphens + underscores only.
/// This prevents path traversal attacks (e.g. `../evil`, `foo/bar`).
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("Profile name must be 1-64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "Profile name must contain only alphanumeric characters, hyphens, \
             and underscores (got '{name}')"
        );
    }
    Ok(())
}

// ---- Token storage ----

/// Resolve the token directory for a given profile.
/// `None` = default profile (`~/.nyxid/`)
/// `Some(name)` = named profile (`~/.nyxid/profiles/{name}/`)
fn token_dir_for_profile(profile: Option<&str>) -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let base = home.join(TOKEN_DIR_NAME);
    match profile {
        None => Ok(base),
        Some(name) => {
            validate_profile_name(name)?;
            Ok(base.join(PROFILES_DIR_NAME).join(name))
        }
    }
}

fn token_file_path_for(profile: Option<&str>) -> Result<PathBuf> {
    Ok(token_dir_for_profile(profile)?.join(TOKEN_FILE_NAME))
}

fn refresh_token_file_path_for(profile: Option<&str>) -> Result<PathBuf> {
    Ok(token_dir_for_profile(profile)?.join(REFRESH_TOKEN_FILE_NAME))
}

fn base_url_file_path_for(profile: Option<&str>) -> Result<PathBuf> {
    Ok(token_dir_for_profile(profile)?.join(BASE_URL_FILE_NAME))
}

fn user_id_file_path_for(profile: Option<&str>) -> Result<PathBuf> {
    Ok(token_dir_for_profile(profile)?.join(USER_ID_FILE_NAME))
}

pub fn read_saved_token_for(profile: Option<&str>) -> Option<String> {
    let path = token_file_path_for(profile).ok()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

pub fn read_saved_token() -> Option<String> {
    read_saved_token_for(None)
}

pub fn read_saved_refresh_token_for(profile: Option<&str>) -> Option<String> {
    let path = refresh_token_file_path_for(profile).ok()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
pub fn read_saved_refresh_token() -> Option<String> {
    read_saved_refresh_token_for(None)
}

pub fn read_saved_base_url_for(profile: Option<&str>) -> Option<String> {
    let path = base_url_file_path_for(profile).ok()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Return the authenticated user's NyxID UUID for the given profile, or
/// `None` if no one is logged in. The access-token JWT `sub` claim is the
/// canonical source; the `user_id` file is a cache that can get stale
/// (manual edits, partial writes during logout races) so we only consult
/// it when deriving from the current token fails.
#[allow(dead_code)]
pub fn read_saved_user_id_for(profile: Option<&str>) -> Option<String> {
    if let Some(access_token) = read_saved_token_for(profile)
        && let Some(user_id) = jwt_sub_from_token(&access_token)
    {
        return Some(user_id);
    }
    let path = user_id_file_path_for(profile).ok()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

pub fn read_saved_base_url() -> Option<String> {
    read_saved_base_url_for(None)
}

fn save_base_url_for(profile: Option<&str>, url: &str) -> Result<()> {
    let path = base_url_file_path_for(profile)?;
    let dir = path.parent().context("Invalid token directory")?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, url)?;
    Ok(())
}

fn write_token_file(path: &std::path::Path, token: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::write(path, token)
        .with_context(|| format!("Failed to write token to {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }
    Ok(())
}

/// Save a new access token (and optionally a new refresh token) for a profile.
/// Also persists the JWT `sub` claim as the user's NyxID UUID so telemetry
/// has stable per-user attribution without re-parsing the token every call.
/// If the new token yields no `sub`, any stale `user_id` file is removed so
/// attribution never survives a token shape change.
pub fn save_tokens_for(
    profile: Option<&str>,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<()> {
    write_token_file(&token_file_path_for(profile)?, access_token)?;
    if let Some(rt) = refresh_token {
        write_token_file(&refresh_token_file_path_for(profile)?, rt)?;
    }
    let user_id_path = user_id_file_path_for(profile)?;
    match jwt_sub_from_token(access_token) {
        Some(user_id) => write_token_file(&user_id_path, &user_id)?,
        None => {
            if user_id_path.exists() {
                let _ = std::fs::remove_file(&user_id_path);
            }
        }
    }
    Ok(())
}

/// Save a new access token (and optionally a new refresh token).
#[cfg(test)]
pub fn save_tokens(access_token: &str, refresh_token: Option<&str>) -> Result<()> {
    save_tokens_for(None, access_token, refresh_token)
}

fn clear_token_for(profile: Option<&str>) -> Result<()> {
    let path = token_file_path_for(profile)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove {}", path.display()))?;
    }
    let refresh_path = refresh_token_file_path_for(profile)?;
    if refresh_path.exists() {
        std::fs::remove_file(&refresh_path)
            .with_context(|| format!("Failed to remove {}", refresh_path.display()))?;
    }
    let user_id_path = user_id_file_path_for(profile)?;
    if user_id_path.exists() {
        std::fs::remove_file(&user_id_path)
            .with_context(|| format!("Failed to remove {}", user_id_path.display()))?;
    }
    Ok(())
}

// ---- Token resolution (same 3-step as ssh_cli.rs) ----

pub fn resolve_access_token(auth: &AuthArgs) -> Result<String> {
    // 1. Explicit --access-token flag
    if let Some(token) = &auth.access_token {
        return Ok(token.clone());
    }

    // 2. Environment variable (NYXID_ACCESS_TOKEN by default)
    if let Ok(token) = std::env::var(&auth.access_token_env)
        && !token.is_empty()
    {
        return Ok(token);
    }

    // 3. Saved token from `nyxid login` (profile-aware)
    if let Some(token) = read_saved_token_for(auth.profile.as_deref()) {
        return Ok(token);
    }

    bail!(
        "No access token found. Run `nyxid login --base-url <URL>`, \
         set {}, or pass --access-token",
        auth.access_token_env
    )
}

// ---- Session preflight ----

/// Why a saved session can't be used without re-authenticating.
#[derive(Debug, Clone, Copy)]
enum DeadSessionReason {
    /// No saved access token for this profile (never logged in / logged out).
    NoToken,
    /// Access token expired and the refresh token could not renew it
    /// (missing, expired, revoked, or rejected by the server).
    Expired,
}

impl DeadSessionReason {
    fn headline(self) -> &'static str {
        match self {
            DeadSessionReason::NoToken => "You are not logged in.",
            DeadSessionReason::Expired => "Your session has expired.",
        }
    }
}

/// Outcome of a single silent refresh attempt.
enum SessionRefresh {
    /// New tokens were issued and persisted.
    Refreshed,
    /// The server (or local state) says this session is done; caller should
    /// fall through to the dead-session path (prompt or error).
    Unauthorized,
    /// Could not reach the server to find out; caller should hard-fail with a
    /// connectivity message rather than prompt for a login that also needs
    /// the network.
    Network(anyhow::Error),
}

/// Validate the saved session BEFORE a command does any user-visible work
/// (opening a browser wizard, connecting an SSH socket, firing a proxy
/// request). The fast path is local and free: parse the stored access token's
/// `exp` and return immediately if it has time left. Only when the token is
/// expired do we touch the network (one `/auth/refresh`).
///
/// There is no auto-login: when the session is genuinely dead, an interactive
/// terminal is *prompted* to log in (and, on success, asked to re-run the
/// command), while a headless/non-TTY caller gets a clean error and a non-zero
/// exit -- never a hang.
///
/// A user-supplied token (`--access-token` or `NYXID_ACCESS_TOKEN`) is left
/// untouched: the caller chose that credential explicitly, so we don't try to
/// refresh it or judge its expiry -- any 401 surfaces from the real request.
pub async fn ensure_session(auth: &AuthArgs) -> Result<()> {
    if auth.access_token.is_some()
        || std::env::var(&auth.access_token_env)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    {
        return Ok(());
    }

    let profile = auth.profile.as_deref();

    let Some(access_token) = read_saved_token_for(profile) else {
        return handle_dead_session(auth, DeadSessionReason::NoToken).await;
    };

    // Opaque / unparseable token: we can't judge its validity offline, so let
    // the real request decide (and `ApiClient` refresh-on-401 handle it).
    let Some(exp) = jwt_exp_from_token(&access_token) else {
        return Ok(());
    };

    // Still comfortably valid -> proceed with zero network cost.
    if exp > Utc::now() + Duration::seconds(SESSION_SKEW_SECS) {
        return Ok(());
    }

    // Access token expired -> one silent refresh attempt.
    match refresh_saved_session(auth).await {
        SessionRefresh::Refreshed => Ok(()),
        SessionRefresh::Unauthorized => handle_dead_session(auth, DeadSessionReason::Expired).await,
        SessionRefresh::Network(e) => Err(anyhow!(
            "Couldn't reach NyxID to refresh your session ({e}). \
             Check your connection and try again."
        )),
    }
}

/// Result of the raw `POST /auth/refresh` exchange. This is the single source
/// of truth for the refresh wire protocol; it performs NO token storage so
/// callers can decide how to persist / apply the rotated pair.
pub(crate) enum RefreshExchange {
    /// Server issued a new access + refresh token pair.
    Renewed {
        access_token: String,
        refresh_token: String,
    },
    /// Server rejected the refresh token (4xx) -- the session is not renewable.
    Unauthorized,
    /// Could not determine the outcome (network error, 5xx, malformed body).
    Network(anyhow::Error),
}

/// Canonical `POST /api/v1/auth/refresh` exchange, shared by the session
/// preflight ([`refresh_saved_session`]), the in-flight 401 retry
/// ([`crate::api::ApiClient`]), and the wizard's local proxy
/// (`wizard::server`). `base_url_root` is the NyxID origin WITHOUT the
/// `/api/v1` suffix; this function appends the path. Token I/O (reading the
/// refresh token, persisting the result, updating any in-memory copy) is the
/// caller's responsibility.
pub(crate) async fn exchange_refresh_token(
    client: &reqwest::Client,
    base_url_root: &str,
    refresh_token: &str,
) -> RefreshExchange {
    let url = format!(
        "{}/api/v1/auth/refresh",
        base_url_root.trim_end_matches('/')
    );
    let resp = match client
        .post(&url)
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return RefreshExchange::Network(anyhow!(e)),
    };

    let status = resp.status();
    // 429 (rate limited) and 408 (request timeout) are transient, not "session
    // dead" -- surface them as retryable connectivity-style failures so a
    // logged-in user isn't pushed through a full re-login over a temporary blip.
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
    {
        return RefreshExchange::Network(anyhow!(
            "refresh temporarily unavailable (HTTP {status})"
        ));
    }
    if status.is_client_error() {
        // 401/403/400 etc. -> the refresh token is genuinely not renewable.
        return RefreshExchange::Unauthorized;
    }
    if !status.is_success() {
        return RefreshExchange::Network(anyhow!("refresh failed (HTTP {status})"));
    }

    #[derive(Deserialize)]
    struct RefreshBody {
        access_token: String,
        refresh_token: String,
    }
    match resp.json::<RefreshBody>().await {
        Ok(b) => RefreshExchange::Renewed {
            access_token: b.access_token,
            refresh_token: b.refresh_token,
        },
        Err(e) => RefreshExchange::Network(anyhow!(e)),
    }
}

/// Attempt to renew the access token using the saved refresh token, persisting
/// the rotated pair on success. Standalone so the preflight can run before any
/// client exists; the wire protocol itself lives in [`exchange_refresh_token`].
async fn refresh_saved_session(auth: &AuthArgs) -> SessionRefresh {
    let profile = auth.profile.as_deref();

    let Some(refresh_token) = read_saved_refresh_token_for(profile) else {
        return SessionRefresh::Unauthorized;
    };

    // If the refresh token itself is already expired, skip the round trip --
    // the server would only reject it (and reuse-detection could revoke the
    // whole session).
    if let Some(exp) = jwt_exp_from_token(&refresh_token)
        && exp <= Utc::now()
    {
        return SessionRefresh::Unauthorized;
    }

    let base_url = match auth.resolved_base_url() {
        Ok(url) => url,
        Err(e) => return SessionRefresh::Network(e),
    };
    let client = match build_cli_http_client(profile) {
        Ok(c) => c,
        Err(e) => return SessionRefresh::Network(e),
    };

    match exchange_refresh_token(&client, &base_url, &refresh_token).await {
        RefreshExchange::Renewed {
            access_token,
            refresh_token,
        } => {
            if save_tokens_for(profile, &access_token, Some(&refresh_token)).is_err() {
                return SessionRefresh::Network(anyhow!("failed to persist refreshed tokens"));
            }
            SessionRefresh::Refreshed
        }
        RefreshExchange::Unauthorized => SessionRefresh::Unauthorized,
        RefreshExchange::Network(e) => SessionRefresh::Network(e),
    }
}

/// Handle a session that can't be used: prompt for re-login when interactive,
/// otherwise return a clean error. Never hangs in a non-TTY context.
async fn handle_dead_session(auth: &AuthArgs, reason: DeadSessionReason) -> Result<()> {
    let headline = reason.headline();

    // Only prompt when we can actually read an answer AND surface the prompt.
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stderr());
    if !interactive {
        bail!("{headline} Run `nyxid login` to continue.");
    }

    eprint!("{headline} Log in again now? [Y/n] ");
    std::io::stderr().flush().ok();

    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("Failed to read response")?;
    let answer = answer.trim().to_ascii_lowercase();
    if !(answer.is_empty() || answer == "y" || answer == "yes") {
        bail!("{headline} Run `nyxid login` when you're ready.");
    }

    // No auto-login: only after explicit consent do we open the login flow.
    let base_url = auth
        .resolved_base_url()
        .unwrap_or_else(|_| DEFAULT_LOGIN_BASE_URL.to_string());
    run_login(LoginArgs {
        base_url,
        password: false,
        device: false,
        email: None,
        profile: auth.profile.clone(),
    })
    .await?;

    // Deliberately do NOT resume the original command: the user re-runs it with
    // the fresh token. This keeps the process simple and the behavior
    // predictable.
    eprintln!();
    eprintln!("Logged in. Re-run your command to continue.");
    std::process::exit(0);
}

// ---- Login ----

pub async fn run_login(args: LoginArgs) -> Result<()> {
    let strategies = RealLoginStrategies;
    run_login_with_strategies(args, &strategies).await
}

trait LoginStrategies {
    fn run_password_login<'a>(
        &'a self,
        base_url: &'a str,
        email: Option<&'a str>,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    fn run_device_code_login<'a>(
        &'a self,
        base_url: &'a str,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    fn run_browser_login<'a>(
        &'a self,
        base_url: &'a str,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), BrowserLoginError>> + Send + 'a>>;
}

struct RealLoginStrategies;

impl LoginStrategies for RealLoginStrategies {
    fn run_password_login<'a>(
        &'a self,
        base_url: &'a str,
        email: Option<&'a str>,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(run_password_login(base_url, email, profile))
    }

    fn run_device_code_login<'a>(
        &'a self,
        base_url: &'a str,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(run_device_code_login(base_url, profile))
    }

    fn run_browser_login<'a>(
        &'a self,
        base_url: &'a str,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), BrowserLoginError>> + Send + 'a>> {
        Box::pin(run_browser_login(base_url, profile))
    }
}

async fn run_login_with_strategies(
    args: LoginArgs,
    strategies: &impl LoginStrategies,
) -> Result<()> {
    let profile = args.profile.as_deref();
    if args.password {
        return strategies
            .run_password_login(&args.base_url, args.email.as_deref(), profile)
            .await;
    }
    if args.device {
        return strategies
            .run_device_code_login(&args.base_url, profile)
            .await;
    }

    if std::env::var_os("NYXID_LOGIN_NO_DEVICE_FALLBACK").is_none() {
        if is_ci_environment() {
            bail!(
                "Detected CI environment (CI / GITHUB_ACTIONS / BUILDKITE / CIRCLECI / \
                 JENKINS_URL / GITLAB_CI set). Use `nyxid api-key create` and \
                 `NYXID_API_KEY` instead of interactive login."
            );
        }
        if !crate::wizard::is_wizard_eligible() && stderr_is_tty() {
            return strategies
                .run_device_code_login(&args.base_url, profile)
                .await;
        }
    }

    match strategies.run_browser_login(&args.base_url, profile).await {
        Ok(()) => Ok(()),
        Err(BrowserLoginError::CannotOpenBrowser(_)) => {
            eprintln!("Couldn't open a browser. Falling back to device-code login.");
            strategies
                .run_device_code_login(&args.base_url, profile)
                .await
        }
        Err(e) => Err(e.into()),
    }
}

// ---- Logout ----

pub async fn run_logout(base_url: &str, profile: Option<&str>) -> Result<()> {
    let base_url = base_url.trim_end_matches('/');

    // Best-effort server-side logout
    if let Some(token) = read_saved_token_for(profile) {
        let client = build_cli_http_client(profile)?;

        let _ = client
            .post(format!("{base_url}/api/v1/auth/logout"))
            .bearer_auth(&token)
            .send()
            .await;
    }

    clear_token_for(profile)?;

    // Telemetry: drop the anon id so the next command on this machine
    // starts a fresh distinct_id (same mechanism as `posthog.reset()`
    // on the web/mobile clients). No-op when consent is off.
    if let Some(client) = crate::telemetry::TelemetryClient::init(profile) {
        client.reset();
    }

    eprintln!("Logged out. Token cleared.");
    Ok(())
}

// ---- Browser login (ported from login_cli.rs) ----

#[derive(Debug)]
pub enum BrowserLoginError {
    CannotOpenBrowser(std::io::Error),
    CallbackTimedOut,
    CallbackIo(anyhow::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for BrowserLoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CannotOpenBrowser(e) => write!(f, "couldn't open browser: {e}"),
            Self::CallbackTimedOut => {
                write!(
                    f,
                    "Login timed out after {CALLBACK_TIMEOUT_SECS}s. Please try again."
                )
            }
            Self::CallbackIo(e) => write!(f, "{e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BrowserLoginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CannotOpenBrowser(e) => Some(e),
            Self::CallbackTimedOut => None,
            Self::CallbackIo(e) | Self::Other(e) => e.source(),
        }
    }
}

async fn run_browser_login(
    base_url: &str,
    profile: Option<&str>,
) -> std::result::Result<(), BrowserLoginError> {
    let base_url = base_url.trim_end_matches('/');

    let frontend_url = fetch_frontend_url(base_url, profile)
        .await
        .map_err(BrowserLoginError::Other)?;

    let listener = TcpListener::bind("127.0.0.1:0")
        .context("Failed to bind local callback server")
        .map_err(BrowserLoginError::CallbackIo)?;
    let port = listener
        .local_addr()
        .context("Failed to read local callback server address")
        .map_err(BrowserLoginError::CallbackIo)?
        .port();

    let state = generate_state();
    let auth_url =
        build_cli_auth_url(&frontend_url, port, &state).map_err(BrowserLoginError::Other)?;

    eprintln!("Opening browser to log in...");
    eprintln!();
    eprintln!("If the browser does not open, visit:");
    eprintln!("  {auth_url}");
    eprintln!();
    eprintln!("If login fails, check the browser tab for details");
    eprintln!("(e.g. \"invite code required\" for new social sign-ups).");
    eprintln!();

    if let Err(e) = crate::browser::open_browser(&auth_url) {
        drop(listener);
        return Err(BrowserLoginError::CannotOpenBrowser(e));
    }

    let callback = wait_for_callback(listener, &state).await?;
    save_tokens_for(
        profile,
        &callback.access_token,
        callback.refresh_token.as_deref(),
    )
    .map_err(BrowserLoginError::Other)?;
    save_base_url_for(profile, base_url).map_err(BrowserLoginError::Other)?;

    // Telemetry: identify the now-authenticated user. `save_tokens_for`
    // above derived + persisted `user_id` from the JWT; we read it
    // back and hand it to the wrapper, which handles anon → user_id
    // merge transparently. No-op when consent is off.
    if let Some(user_id) = read_saved_user_id_for(profile)
        && let Some(mut client) = crate::telemetry::TelemetryClient::init(profile)
    {
        client.identify(&user_id).await;
    }

    eprintln!("Logged in successfully.");
    let token_path = token_file_path_for(profile).map_err(BrowserLoginError::Other)?;
    eprintln!("Token saved to {}", token_path.display());

    Ok(())
}

struct CallbackTokens {
    access_token: String,
    refresh_token: Option<String>,
}

async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
) -> std::result::Result<CallbackTokens, BrowserLoginError> {
    listener
        .set_nonblocking(true)
        .context("Failed to set listener to non-blocking")
        .map_err(BrowserLoginError::CallbackIo)?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .context("Failed to create async listener")
        .map_err(BrowserLoginError::CallbackIo)?;

    let timeout = tokio::time::sleep(std::time::Duration::from_secs(CALLBACK_TIMEOUT_SECS));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (mut stream, _) = accept
                    .context("Failed to accept connection")
                    .map_err(BrowserLoginError::CallbackIo)?;
                let mut buf = vec![0u8; 8192];
                let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await
                    .context("Failed to read request")
                    .map_err(BrowserLoginError::CallbackIo)?;
                let request = String::from_utf8_lossy(&buf[..n]);

                if let Some(tokens) = parse_callback_request(&request, expected_state) {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{}",
                        callback_success_html()
                    );
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
                    return Ok(tokens);
                }

                let response = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
            () = &mut timeout => {
                return Err(BrowserLoginError::CallbackTimedOut);
            }
        }
    }
}

fn parse_callback_request(request: &str, expected_state: &str) -> Option<CallbackTokens> {
    let first_line = request.lines().next()?;
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

    let access_token = urlencoding::decode(token).ok()?.into_owned();
    let refresh_token = params
        .get("refresh_token")
        .and_then(|t| urlencoding::decode(t).ok())
        .map(|t| t.into_owned())
        .filter(|t| !t.is_empty());

    Some(CallbackTokens {
        access_token,
        refresh_token,
    })
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

fn build_cli_auth_url(frontend_url: &str, port: u16, state: &str) -> Result<String> {
    let mut url = url::Url::parse(&format!("{}/cli-auth", frontend_url.trim_end_matches('/')))
        .context("Invalid frontend URL")?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("port", &port.to_string());
        query.append_pair("state", state);
        query.append_pair("client_ua", CLI_USER_AGENT);
    }
    Ok(url.into())
}

#[derive(Deserialize)]
struct PublicConfig {
    frontend_url: String,
}

pub async fn fetch_frontend_url(base_url: &str, profile: Option<&str>) -> Result<String> {
    let config_url = format!("{base_url}/api/v1/public/config");
    let client = build_cli_http_client(profile)?;

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

// ---- Device-code login ----

trait AuthDeviceApi {
    fn request<'a>(
        &'a self,
        base_url: &'a str,
        body: &'a AuthDeviceRequestBody,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthDeviceRequestOutcome>> + Send + 'a>>;

    fn poll<'a>(
        &'a self,
        base_url: &'a str,
        body: &'a AuthDevicePollBody,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthDevicePollOutcome>> + Send + 'a>>;
}

struct RealAuthDeviceApi;

impl AuthDeviceApi for RealAuthDeviceApi {
    fn request<'a>(
        &'a self,
        base_url: &'a str,
        body: &'a AuthDeviceRequestBody,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthDeviceRequestOutcome>> + Send + 'a>> {
        Box::pin(crate::api::auth_device_request(base_url, body, profile))
    }

    fn poll<'a>(
        &'a self,
        base_url: &'a str,
        body: &'a AuthDevicePollBody,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthDevicePollOutcome>> + Send + 'a>> {
        Box::pin(crate::api::auth_device_poll(base_url, body, profile))
    }
}

trait LoginSleeper {
    fn sleep<'a>(&'a self, seconds: u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

struct TokioLoginSleeper;

impl LoginSleeper for TokioLoginSleeper {
    fn sleep<'a>(&'a self, seconds: u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(tokio::time::sleep(std::time::Duration::from_secs(seconds)))
    }
}

trait DeviceBrowserFallback {
    fn run_browser_login<'a>(
        &'a self,
        base_url: &'a str,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), BrowserLoginError>> + Send + 'a>>;
}

struct RealDeviceBrowserFallback;

impl DeviceBrowserFallback for RealDeviceBrowserFallback {
    fn run_browser_login<'a>(
        &'a self,
        base_url: &'a str,
        profile: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), BrowserLoginError>> + Send + 'a>> {
        Box::pin(run_browser_login(base_url, profile))
    }
}

async fn run_device_code_login(base_url: &str, profile: Option<&str>) -> Result<()> {
    let api = RealAuthDeviceApi;
    let sleeper = TokioLoginSleeper;
    let browser = RealDeviceBrowserFallback;
    run_device_code_login_with_api(base_url, profile, &api, &sleeper, &browser).await
}

async fn run_device_code_login_with_api(
    base_url: &str,
    profile: Option<&str>,
    api: &impl AuthDeviceApi,
    sleeper: &impl LoginSleeper,
    browser: &impl DeviceBrowserFallback,
) -> Result<()> {
    let base_url = base_url.trim_end_matches('/');
    let request = AuthDeviceRequestBody {
        client_label: client_label(),
        client_user_agent: Some(CLI_USER_AGENT.to_string()),
    };

    let challenge = match api.request(base_url, &request, profile).await? {
        AuthDeviceRequestOutcome::Created(challenge) => challenge,
        AuthDeviceRequestOutcome::NotSupported => {
            eprintln!(
                "This NyxID backend doesn't support device-code login. Falling back to browser login."
            );
            return browser
                .run_browser_login(base_url, profile)
                .await
                .map_err(Into::into);
        }
    };

    eprintln!("! First copy your one-time code: {}", challenge.user_code);
    eprintln!();
    eprintln!(
        "Then open {} and enter the code above.",
        challenge.verification_uri
    );

    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stderr());
    if interactive {
        eprint!("\nOpen in your browser? [Y/n] ");
        std::io::stderr().flush().ok();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_ok() {
            let a = answer.trim().to_ascii_lowercase();
            if (a.is_empty() || a == "y" || a == "yes")
                && let Err(e) = crate::browser::open_browser(&challenge.verification_uri)
            {
                eprintln!("Could not open browser: {e}. Paste the URL above manually.");
            }
        }
    }

    poll_device_code_login(
        base_url,
        profile,
        api,
        sleeper,
        challenge.device_code,
        challenge.interval,
    )
    .await
}

async fn poll_device_code_login(
    base_url: &str,
    profile: Option<&str>,
    api: &impl AuthDeviceApi,
    sleeper: &impl LoginSleeper,
    device_code: String,
    initial_interval: u64,
) -> Result<()> {
    let mut interval = initial_interval.max(1);

    loop {
        sleeper.sleep(interval).await;
        let body = AuthDevicePollBody {
            device_code: device_code.clone(),
        };

        match api.poll(base_url, &body, profile).await? {
            AuthDevicePollOutcome::Delivered(tokens) => {
                save_tokens_for(profile, &tokens.access_token, Some(&tokens.refresh_token))?;
                save_base_url_for(profile, base_url)?;

                if let Some(user_id) = read_saved_user_id_for(profile)
                    && let Some(mut client) = crate::telemetry::TelemetryClient::init(profile)
                {
                    client.identify(&user_id).await;
                }

                eprintln!("Signed in.");
                return Ok(());
            }
            AuthDevicePollOutcome::Error(error) => {
                interval = handle_device_poll_error(error.error_code, &error.message, interval)?;
            }
        }
    }
}

fn handle_device_poll_error(error_code: i64, message: &str, interval: u64) -> Result<u64> {
    match error_code {
        AUTH_DEVICE_CODE_PENDING => Ok(interval),
        AUTH_DEVICE_CODE_SLOW_DOWN => Ok(interval + 5),
        AUTH_DEVICE_CODE_EXPIRED => bail!("Login timed out - run `nyxid login --device` again."),
        AUTH_DEVICE_CODE_DENIED => bail!("Login denied."),
        AUTH_DEVICE_CODE_ALREADY_DELIVERED => {
            bail!("This code was already used. Run `nyxid login --device` again.")
        }
        AUTH_DEVICE_CODE_RATE_LIMITED => bail!("Too many attempts. Try again in a few minutes."),
        _ => bail!("{message}"),
    }
}

fn client_label() -> Option<String> {
    hostname_from_env()
        .or_else(hostname_from_command)
        .map(|s| sanitize_client_label(&s))
        .filter(|s| !s.is_empty())
}

fn hostname_from_env() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
}

fn hostname_from_command() -> Option<String> {
    let output = std::process::Command::new("hostname").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .filter(|s| !s.trim().is_empty())
}

fn sanitize_client_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect()
}

pub(crate) fn is_ci_environment() -> bool {
    [
        "CI",
        "GITHUB_ACTIONS",
        "BUILDKITE",
        "CIRCLECI",
        "JENKINS_URL",
        "GITLAB_CI",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some())
}

fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

// ---- Password login (ported from login_cli.rs) ----

#[derive(Deserialize)]
struct LoginResponse {
    access_token: String,
    refresh_token: Option<String>,
}

async fn run_password_login(
    base_url: &str,
    email: Option<&str>,
    profile: Option<&str>,
) -> Result<()> {
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

    let client = build_cli_http_client(profile)?;

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

    save_tokens_for(profile, &login.access_token, login.refresh_token.as_deref())?;
    save_base_url_for(profile, base_url)?;

    // Telemetry: identify after token persistence (see notes in
    // `run_browser_login`).
    if let Some(user_id) = read_saved_user_id_for(profile)
        && let Some(mut client) = crate::telemetry::TelemetryClient::init(profile)
    {
        client.identify(&user_id).await;
    }

    eprintln!("Logged in as {email}");
    eprintln!("Token saved to {}", token_file_path_for(profile)?.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AUTH_DEVICE_CODE_ALREADY_DELIVERED, AUTH_DEVICE_CODE_DENIED, AUTH_DEVICE_CODE_EXPIRED,
        AUTH_DEVICE_CODE_PENDING, AUTH_DEVICE_CODE_RATE_LIMITED, AUTH_DEVICE_CODE_SLOW_DOWN,
        AuthDeviceApi, BrowserLoginError, DeviceBrowserFallback, LoginSleeper, LoginStrategies,
        build_cli_auth_url, callback_success_html, handle_device_poll_error, is_ci_environment,
        jwt_sub_from_token, parse_callback_request, poll_device_code_login,
        refresh_token_file_path_for, run_login_with_strategies, token_dir_for_profile,
        token_file_path_for, validate_profile_name,
    };
    use crate::api::{
        AuthDevicePollBody, AuthDevicePollOutcome, AuthDevicePollResponse, AuthDeviceRequestBody,
        AuthDeviceRequestOutcome, CLI_USER_AGENT, ErrorEnvelope,
    };
    use anyhow::{Result, bail};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // ---- Profile validation ----

    #[test]
    fn profile_path_default() {
        let path = token_dir_for_profile(None).expect("default profile");
        assert!(path.to_string_lossy().ends_with(".nyxid"));
    }

    #[test]
    fn profile_path_named() {
        let path = token_dir_for_profile(Some("coding-agent")).expect("named profile");
        assert!(path.to_string_lossy().contains("profiles/coding-agent"));
    }

    #[test]
    fn profile_path_with_underscores() {
        let path = token_dir_for_profile(Some("my_agent_1")).expect("underscore profile");
        assert!(path.to_string_lossy().contains("profiles/my_agent_1"));
    }

    #[test]
    fn profile_name_validation_rejects_empty() {
        assert!(validate_profile_name("").is_err());
    }

    #[test]
    fn profile_name_validation_rejects_path_traversal() {
        assert!(validate_profile_name("../evil").is_err());
        assert!(validate_profile_name("foo/bar").is_err());
        assert!(validate_profile_name("..").is_err());
    }

    #[test]
    fn profile_name_validation_rejects_spaces() {
        assert!(validate_profile_name("my agent").is_err());
    }

    #[test]
    fn profile_name_validation_rejects_too_long() {
        let long_name = "a".repeat(65);
        assert!(validate_profile_name(&long_name).is_err());
    }

    #[test]
    fn profile_name_validation_accepts_valid() {
        assert!(validate_profile_name("coding-agent").is_ok());
        assert!(validate_profile_name("research_agent").is_ok());
        assert!(validate_profile_name("a1-b2_c3").is_ok());
        assert!(validate_profile_name("x").is_ok());
        let max_name = "a".repeat(64);
        assert!(validate_profile_name(&max_name).is_ok());
    }

    // ---- Token path tests ----

    #[test]
    fn token_path_is_under_home() {
        let path = token_file_path_for(None).expect("token path");
        assert!(path.to_string_lossy().contains(".nyxid"));
        assert!(path.to_string_lossy().ends_with("access_token"));
    }

    #[test]
    fn token_path_profile_is_under_profiles() {
        let path = token_file_path_for(Some("test-agent")).expect("profile token path");
        assert!(
            path.to_string_lossy()
                .contains(".nyxid/profiles/test-agent/access_token")
        );
    }

    #[test]
    fn refresh_token_path_is_under_home() {
        let path = refresh_token_file_path_for(None).expect("refresh token path");
        assert!(path.to_string_lossy().contains(".nyxid"));
        assert!(path.to_string_lossy().ends_with("refresh_token"));
    }

    #[test]
    fn parses_valid_callback_request() {
        let request =
            "GET /callback?access_token=tok_abc123&state=deadbeef HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        let result = parse_callback_request(request, "deadbeef").expect("should parse");
        assert_eq!(result.access_token, "tok_abc123");
        assert!(result.refresh_token.is_none());
    }

    #[test]
    fn parses_callback_with_refresh_token() {
        let request = "GET /callback?access_token=tok_abc&refresh_token=ref_xyz&state=deadbeef HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        let result = parse_callback_request(request, "deadbeef").expect("should parse");
        assert_eq!(result.access_token, "tok_abc");
        assert_eq!(result.refresh_token.as_deref(), Some("ref_xyz"));
    }

    #[test]
    fn rejects_wrong_state() {
        let request =
            "GET /callback?access_token=tok_abc123&state=wrong HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        assert!(parse_callback_request(request, "deadbeef").is_none());
    }

    #[test]
    fn rejects_non_callback_path() {
        let request = "GET /other?access_token=tok_abc123&state=deadbeef HTTP/1.1\r\n";
        assert!(parse_callback_request(request, "deadbeef").is_none());
    }

    #[test]
    fn success_html_is_not_empty() {
        assert!(callback_success_html().contains("Login successful"));
    }

    #[test]
    fn build_cli_auth_url_includes_cli_user_agent() {
        let auth_url = build_cli_auth_url("https://app.example.com/", 43123, "deadbeef")
            .expect("should build");
        let parsed = url::Url::parse(&auth_url).expect("valid URL");
        let params = parsed
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(parsed.path(), "/cli-auth");
        assert_eq!(params.get("port").map(|v| v.as_ref()), Some("43123"));
        assert_eq!(params.get("state").map(|v| v.as_ref()), Some("deadbeef"));
        assert_eq!(
            params.get("client_ua").map(|v| v.as_ref()),
            Some(CLI_USER_AGENT)
        );
    }

    #[test]
    fn device_poll_error_mapping_matches_contract() {
        assert_eq!(
            handle_device_poll_error(AUTH_DEVICE_CODE_PENDING, "pending", 5).expect("pending"),
            5
        );
        assert_eq!(
            handle_device_poll_error(AUTH_DEVICE_CODE_SLOW_DOWN, "slow", 5).expect("slow"),
            10
        );

        let expired = handle_device_poll_error(AUTH_DEVICE_CODE_EXPIRED, "expired", 5).unwrap_err();
        assert!(
            expired.to_string().contains("Login timed out"),
            "unexpected: {expired}"
        );
        let denied = handle_device_poll_error(AUTH_DEVICE_CODE_DENIED, "denied", 5).unwrap_err();
        assert_eq!(denied.to_string(), "Login denied.");
        let used =
            handle_device_poll_error(AUTH_DEVICE_CODE_ALREADY_DELIVERED, "used", 5).unwrap_err();
        assert!(
            used.to_string().contains("already used"),
            "unexpected: {used}"
        );
        let limited =
            handle_device_poll_error(AUTH_DEVICE_CODE_RATE_LIMITED, "limited", 5).unwrap_err();
        assert!(
            limited.to_string().contains("Too many attempts"),
            "unexpected: {limited}"
        );
        let not_found =
            handle_device_poll_error(11200, "device code not found from server", 5).unwrap_err();
        assert_eq!(not_found.to_string(), "device code not found from server");
        let invalid =
            handle_device_poll_error(11207, "user code invalid from server", 5).unwrap_err();
        assert_eq!(invalid.to_string(), "user code invalid from server");
        let unknown = handle_device_poll_error(11999, "server message", 5).unwrap_err();
        assert_eq!(unknown.to_string(), "server message");
    }

    #[test]
    fn is_ci_environment_detects_standard_ci_vars() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let keys = [
            "CI",
            "GITHUB_ACTIONS",
            "BUILDKITE",
            "CIRCLECI",
            "JENKINS_URL",
            "GITLAB_CI",
        ];
        let previous: Vec<_> = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        unsafe {
            for key in keys {
                std::env::remove_var(key);
            }
        }
        assert!(!is_ci_environment());

        unsafe {
            std::env::set_var("CI", "1");
        }
        assert!(is_ci_environment());

        unsafe {
            for (key, value) in previous {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    // ---- JWT sub extraction ----

    fn build_jwt(payload: &serde_json::Value) -> String {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload_json = serde_json::to_vec(payload).expect("serialize");
        let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_json);
        format!("{header}.{payload_b64}.signature-not-verified")
    }

    #[test]
    fn jwt_sub_extracts_user_uuid() {
        let token = build_jwt(&serde_json::json!({
            "sub": "7a3f1c8e-0000-4000-8000-000000000001",
            "exp": 9999999999i64,
        }));
        assert_eq!(
            jwt_sub_from_token(&token).as_deref(),
            Some("7a3f1c8e-0000-4000-8000-000000000001")
        );
    }

    #[test]
    fn jwt_sub_returns_none_for_malformed_token() {
        assert!(jwt_sub_from_token("not-a-jwt").is_none());
        assert!(jwt_sub_from_token("").is_none());
        assert!(jwt_sub_from_token("header.badbase64!!.sig").is_none());
    }

    #[test]
    fn jwt_sub_returns_none_when_claim_missing() {
        let token = build_jwt(&serde_json::json!({ "exp": 123 }));
        assert!(jwt_sub_from_token(&token).is_none());
    }

    #[test]
    fn jwt_sub_returns_none_when_claim_is_not_string() {
        let token = build_jwt(&serde_json::json!({ "sub": 42, "exp": 123 }));
        assert!(jwt_sub_from_token(&token).is_none());
    }

    #[test]
    fn jwt_exp_from_token_extracts_expiry() {
        let token = build_jwt(&serde_json::json!({"exp": 1700000000i64}));
        let exp = super::jwt_exp_from_token(&token).unwrap();
        assert_eq!(exp.timestamp(), 1700000000);
    }

    #[test]
    fn jwt_exp_from_token_returns_none_for_malformed() {
        assert!(super::jwt_exp_from_token("bad").is_none());
    }

    #[test]
    fn jwt_claim_string_from_token_extracts_arbitrary_claim() {
        let token = build_jwt(&serde_json::json!({"sub": "user-1", "iss": "nyxid", "exp": 999}));
        assert_eq!(
            super::jwt_claim_string_from_token(&token, "iss").as_deref(),
            Some("nyxid")
        );
    }

    #[test]
    fn jwt_claim_string_from_token_returns_none_for_missing_claim() {
        let token = build_jwt(&serde_json::json!({"exp": 999}));
        assert!(super::jwt_claim_string_from_token(&token, "iss").is_none());
    }

    #[test]
    fn dead_session_reason_headline_no_token() {
        assert!(
            super::DeadSessionReason::NoToken
                .headline()
                .contains("not logged in")
        );
    }

    #[test]
    fn dead_session_reason_headline_expired() {
        assert!(
            super::DeadSessionReason::Expired
                .headline()
                .contains("expired")
        );
    }

    #[test]
    fn save_tokens_clears_stale_user_id_when_new_token_has_no_sub() {
        use std::env;
        use tempfile::tempdir;

        // Serialize HOME mutations across test modules — any concurrent
        // test touching $HOME uses the same lock.
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let dir = tempdir().expect("tempdir");
        let prev_home = env::var_os("HOME");
        // SAFETY: guarded by `env_lock` above.
        unsafe {
            env::set_var("HOME", dir.path());
        }

        let profile = Some("stale-cleanup-test");
        let good_token = build_jwt(&serde_json::json!({
            "sub": "11111111-2222-3333-4444-555555555555",
            "exp": 9999999999i64,
        }));
        super::save_tokens_for(profile, &good_token, None).expect("save good");
        assert_eq!(
            super::read_saved_user_id_for(profile).as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );

        // New token carries no `sub` — prior attribution must not linger.
        let bad_token = build_jwt(&serde_json::json!({ "exp": 123 }));
        super::save_tokens_for(profile, &bad_token, None).expect("save bad");
        let user_id_path = super::user_id_file_path_for(profile).expect("path");
        assert!(
            !user_id_path.exists(),
            "stale user_id should be cleared when new token has no sub"
        );

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
        }
    }

    // ---- ensure_session preflight ----

    use crate::cli::{AuthArgs, OutputFormat};

    /// AuthArgs wired so `ensure_session` won't take the user-supplied-token
    /// shortcut: `access_token` is None and `access_token_env` points at a var
    /// we keep unset. Profile isolates token storage under the temp HOME.
    fn preflight_auth_args() -> AuthArgs {
        AuthArgs {
            base_url: Some("http://127.0.0.1:0".to_string()),
            access_token: None,
            access_token_env: "NYXID_ACCESS_TOKEN_PREFLIGHT_UNSET".to_string(),
            profile: Some("ensure-session-test".to_string()),
            output: OutputFormat::Table,
        }
    }

    /// RAII: lock the shared env mutex, point HOME at a fresh temp dir, and
    /// guarantee the preflight env var is unset. Restores HOME on drop. Mirrors
    /// `api.rs`'s `HomeGuard`; holding the lock across `.await` is intentional
    /// (the single-threaded test runtime never moves threads).
    struct PreflightHome {
        _dir: tempfile::TempDir,
        prev_home: Option<std::ffi::OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl PreflightHome {
        fn set() -> Self {
            let guard = crate::test_support::env_lock()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().expect("tempdir");
            let prev_home = std::env::var_os("HOME");
            // SAFETY: serialized by `env_lock`.
            unsafe {
                std::env::set_var("HOME", dir.path());
                std::env::remove_var("NYXID_ACCESS_TOKEN_PREFLIGHT_UNSET");
            }
            Self {
                _dir: dir,
                prev_home,
                _guard: guard,
            }
        }
    }

    impl Drop for PreflightHome {
        fn drop(&mut self) {
            // SAFETY: serialized by `env_lock`.
            unsafe {
                match &self.prev_home {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }

    #[tokio::test]
    async fn ensure_session_skips_when_explicit_token_flag_set() {
        // The explicit-token shortcut returns before touching token storage or
        // the network, so no HOME mutation is needed.
        let mut auth = preflight_auth_args();
        auth.access_token = Some("explicit-token".to_string());
        super::ensure_session(&auth)
            .await
            .expect("explicit token is honored");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // serializes HOME/env mutations across tests
    async fn ensure_session_ok_for_unexpired_saved_token() {
        let _home = PreflightHome::set();
        let auth = preflight_auth_args();
        let token = build_jwt(&serde_json::json!({
            "sub": "11111111-2222-3333-4444-555555555555",
            "exp": 9999999999i64,
        }));
        super::save_tokens_for(auth.profile.as_deref(), &token, Some("refresh"))
            .expect("save token");

        super::ensure_session(&auth)
            .await
            .expect("unexpired token needs no network and is accepted");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ensure_session_errors_when_no_token_and_headless() {
        // Test stdin is not a TTY, so handle_dead_session takes the
        // non-interactive branch and returns a clean error instead of hanging.
        let _home = PreflightHome::set();
        let auth = preflight_auth_args();

        let err = super::ensure_session(&auth)
            .await
            .expect_err("no token must error");
        let msg = format!("{err}");
        assert!(msg.contains("not logged in"), "unexpected message: {msg}");
        assert!(msg.contains("nyxid login"), "should point at login: {msg}");
    }

    /// Minimal one-shot HTTP server that answers the first request with a
    /// 200 `/auth/refresh` body. Returns the base URL to point the client at.
    async fn spawn_refresh_ok_server(access: &str, refresh: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let body = format!(
            r#"{{"access_token":"{access}","expires_in":900,"refresh_token":"{refresh}"}}"#
        );
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await; // body unparsed; respond regardless
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ensure_session_silently_refreshes_expired_access_token() {
        // The headline behavior: expired access token + a usable refresh token
        // -> one silent /auth/refresh -> command proceeds with the new token,
        // no prompt. Exercises the `Refreshed` arm end to end through the
        // consolidated `exchange_refresh_token`.
        let _home = PreflightHome::set();
        let base = spawn_refresh_ok_server("new-access-token", "new-refresh-token").await;
        let mut auth = preflight_auth_args();
        auth.base_url = Some(base);

        let expired = build_jwt(&serde_json::json!({
            "sub": "11111111-2222-3333-4444-555555555555",
            "exp": 1_000_000_000i64, // 2001, expired
        }));
        // A non-JWT refresh token: `jwt_exp_from_token` returns None, so the
        // local short-circuit is skipped and the network refresh is attempted.
        super::save_tokens_for(
            auth.profile.as_deref(),
            &expired,
            Some("usable-refresh-token"),
        )
        .expect("save initial tokens");

        super::ensure_session(&auth)
            .await
            .expect("silent refresh should succeed without prompting");

        assert_eq!(
            super::read_saved_token_for(auth.profile.as_deref()).as_deref(),
            Some("new-access-token"),
            "preflight should persist the refreshed access token"
        );
        assert_eq!(
            super::read_saved_refresh_token_for(auth.profile.as_deref()).as_deref(),
            Some("new-refresh-token"),
            "preflight should persist the rotated refresh token"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ensure_session_errors_when_expired_and_no_refresh_token() {
        // Expired access token + no refresh token -> Unauthorized without any
        // network call -> dead-session path -> clean headless error.
        let _home = PreflightHome::set();
        let auth = preflight_auth_args();
        let expired = build_jwt(&serde_json::json!({
            "sub": "11111111-2222-3333-4444-555555555555",
            "exp": 1_000_000_000i64, // year 2001, well past
        }));
        // Save only the access token; leave no refresh token file.
        super::write_token_file(
            &super::token_file_path_for(auth.profile.as_deref()).expect("path"),
            &expired,
        )
        .expect("write access token");

        let err = super::ensure_session(&auth)
            .await
            .expect_err("expired token with no refresh must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("session has expired"),
            "unexpected message: {msg}"
        );
        assert!(msg.contains("nyxid login"), "should point at login: {msg}");
    }

    #[derive(Clone)]
    struct MockDeviceApi {
        request: AuthDeviceRequestOutcome,
        polls: Arc<Mutex<Vec<AuthDevicePollOutcome>>>,
    }

    impl MockDeviceApi {
        fn with_polls(polls: Vec<AuthDevicePollOutcome>) -> Self {
            Self {
                request: AuthDeviceRequestOutcome::Created(crate::api::AuthDeviceRequestResponse {
                    device_code: "nyx_adc_test".to_string(),
                    user_code: "ADCB-EFGH".to_string(),
                    verification_uri: "https://nyx.example/login/device".to_string(),
                    verification_uri_complete:
                        "https://nyx.example/login/device?user_code=ADCB-EFGH".to_string(),
                    expires_in: 600,
                    interval: 5,
                }),
                polls: Arc::new(Mutex::new(polls)),
            }
        }
    }

    impl AuthDeviceApi for MockDeviceApi {
        fn request<'a>(
            &'a self,
            _base_url: &'a str,
            _body: &'a AuthDeviceRequestBody,
            _profile: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = Result<AuthDeviceRequestOutcome>> + Send + 'a>> {
            let outcome = self.request.clone();
            Box::pin(async move { Ok(outcome) })
        }

        fn poll<'a>(
            &'a self,
            _base_url: &'a str,
            _body: &'a AuthDevicePollBody,
            _profile: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = Result<AuthDevicePollOutcome>> + Send + 'a>> {
            let polls = self.polls.clone();
            Box::pin(async move {
                let mut polls = polls.lock().unwrap_or_else(|e| e.into_inner());
                if polls.is_empty() {
                    bail!("unexpected extra poll")
                }
                Ok(polls.remove(0))
            })
        }
    }

    struct RecordingSleeper {
        intervals: Arc<Mutex<Vec<u64>>>,
    }

    impl RecordingSleeper {
        fn new() -> Self {
            Self {
                intervals: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn intervals(&self) -> Vec<u64> {
            self.intervals
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    impl LoginSleeper for RecordingSleeper {
        fn sleep<'a>(&'a self, seconds: u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            let intervals = self.intervals.clone();
            Box::pin(async move {
                intervals
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(seconds);
            })
        }
    }

    fn poll_error(code: i64) -> AuthDevicePollOutcome {
        AuthDevicePollOutcome::Error(ErrorEnvelope {
            error: "contract_error".to_string(),
            error_code: code,
            message: format!("message for {code}"),
        })
    }

    fn delivered_tokens() -> AuthDevicePollOutcome {
        AuthDevicePollOutcome::Delivered(AuthDevicePollResponse {
            access_token: build_jwt(&serde_json::json!({
                "sub": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
                "exp": 9999999999i64,
            })),
            refresh_token: "refresh-token".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 900,
        })
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn device_poll_loop_handles_pending_slow_down_then_success_and_profile_storage() {
        let _home = PreflightHome::set();
        let api = MockDeviceApi::with_polls(vec![
            poll_error(AUTH_DEVICE_CODE_PENDING),
            poll_error(AUTH_DEVICE_CODE_SLOW_DOWN),
            delivered_tokens(),
        ]);
        let sleeper = RecordingSleeper::new();

        poll_device_code_login(
            "https://nyx-api.example",
            Some("alt"),
            &api,
            &sleeper,
            "nyx_adc_test".to_string(),
            5,
        )
        .await
        .expect("device login");

        assert_eq!(sleeper.intervals(), vec![5, 5, 10]);
        assert!(
            super::token_file_path_for(Some("alt"))
                .expect("token path")
                .to_string_lossy()
                .contains(".nyxid/profiles/alt/access_token")
        );
        assert!(super::read_saved_token_for(Some("alt")).is_some());
        assert_eq!(
            super::read_saved_refresh_token_for(Some("alt")).as_deref(),
            Some("refresh-token")
        );
        assert_eq!(
            super::read_saved_base_url_for(Some("alt")).as_deref(),
            Some("https://nyx-api.example")
        );
    }

    struct MockDeviceBrowser {
        calls: AtomicUsize,
    }

    impl MockDeviceBrowser {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl DeviceBrowserFallback for MockDeviceBrowser {
        fn run_browser_login<'a>(
            &'a self,
            _base_url: &'a str,
            _profile: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = std::result::Result<(), BrowserLoginError>> + Send + 'a>>
        {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn device_request_404_falls_back_to_browser_flow() {
        let api = MockDeviceApi {
            request: AuthDeviceRequestOutcome::NotSupported,
            polls: Arc::new(Mutex::new(Vec::new())),
        };
        let sleeper = RecordingSleeper::new();
        let browser = MockDeviceBrowser::new();

        super::run_device_code_login_with_api(
            "https://nyx-api.example",
            None,
            &api,
            &sleeper,
            &browser,
        )
        .await
        .expect("browser fallback");

        assert_eq!(browser.calls.load(Ordering::SeqCst), 1);
        assert_eq!(sleeper.intervals(), Vec::<u64>::new());
    }

    #[derive(Default)]
    struct MockLoginStrategies {
        browser_result: Mutex<Option<std::result::Result<(), BrowserLoginError>>>,
        password_calls: AtomicUsize,
        device_calls: AtomicUsize,
        browser_calls: AtomicUsize,
    }

    impl LoginStrategies for MockLoginStrategies {
        fn run_password_login<'a>(
            &'a self,
            _base_url: &'a str,
            _email: Option<&'a str>,
            _profile: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.password_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn run_device_code_login<'a>(
            &'a self,
            _base_url: &'a str,
            _profile: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.device_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn run_browser_login<'a>(
            &'a self,
            _base_url: &'a str,
            _profile: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = std::result::Result<(), BrowserLoginError>> + Send + 'a>>
        {
            Box::pin(async move {
                self.browser_calls.fetch_add(1, Ordering::SeqCst);
                self.browser_result
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .unwrap_or(Ok(()))
            })
        }
    }

    fn login_args() -> crate::cli::LoginArgs {
        crate::cli::LoginArgs {
            base_url: "https://nyx-api.example".to_string(),
            password: false,
            device: false,
            email: None,
            profile: None,
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn browser_open_failure_falls_back_to_device_flow() {
        // Must clear CI env vars so the dispatcher reaches the browser branch
        // — on GitHub Actions runners CI=true is preset.
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let ci_keys = [
            "CI",
            "GITHUB_ACTIONS",
            "BUILDKITE",
            "CIRCLECI",
            "JENKINS_URL",
            "GITLAB_CI",
            "NYXID_LOGIN_NO_DEVICE_FALLBACK",
        ];
        let prev: Vec<_> = ci_keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        unsafe {
            for k in &ci_keys {
                std::env::remove_var(k);
            }
        }

        let strategies = MockLoginStrategies {
            browser_result: Mutex::new(Some(Err(BrowserLoginError::CannotOpenBrowser(
                std::io::Error::new(std::io::ErrorKind::NotFound, "browser"),
            )))),
            ..Default::default()
        };

        let result = run_login_with_strategies(login_args(), &strategies).await;

        unsafe {
            for (k, v) in &prev {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }

        result.expect("fallback succeeds");
        assert_eq!(strategies.browser_calls.load(Ordering::SeqCst), 1);
        assert_eq!(strategies.device_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn explicit_device_flag_skips_browser_flow() {
        let strategies = MockLoginStrategies::default();
        let mut args = login_args();
        args.device = true;

        run_login_with_strategies(args, &strategies)
            .await
            .expect("device login");

        assert_eq!(strategies.browser_calls.load(Ordering::SeqCst), 0);
        assert_eq!(strategies.device_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ci_short_circuit_fires_before_network_or_browser() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev_ci = std::env::var_os("CI");
        let prev_disable = std::env::var_os("NYXID_LOGIN_NO_DEVICE_FALLBACK");
        unsafe {
            std::env::set_var("CI", "1");
            std::env::remove_var("NYXID_LOGIN_NO_DEVICE_FALLBACK");
        }

        let strategies = MockLoginStrategies::default();
        let err = run_login_with_strategies(login_args(), &strategies)
            .await
            .expect_err("CI should fail");
        let msg = err.to_string();
        assert!(msg.contains("Detected CI environment"), "unexpected: {msg}");
        assert!(
            msg.contains("nyxid api-key create"),
            "missing api-key hint: {msg}"
        );
        assert_eq!(strategies.browser_calls.load(Ordering::SeqCst), 0);
        assert_eq!(strategies.device_calls.load(Ordering::SeqCst), 0);

        unsafe {
            match prev_ci {
                Some(value) => std::env::set_var("CI", value),
                None => std::env::remove_var("CI"),
            }
            match prev_disable {
                Some(value) => std::env::set_var("NYXID_LOGIN_NO_DEVICE_FALLBACK", value),
                None => std::env::remove_var("NYXID_LOGIN_NO_DEVICE_FALLBACK"),
            }
        }
    }
}
