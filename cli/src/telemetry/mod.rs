//! CLI telemetry — vendor-neutral public API, fire-and-forget HTTPS POST.
//!
//! Public surface is the short verb list from `docs/TELEMETRY.md`
//! §5.0 hot-swap contract: `init / track / identify / reset`. Vendor
//! wire format lives inside — callers never see `$identify`,
//! `/capture/`, or `phc_…`-shaped DSNs.
//!
//! Privacy posture:
//!   - `init()` returns `None` unless a DSN resolves (hard off by default)
//!   - every invocation of `nyxid` respects `$NYXID_TELEMETRY=off`
//!   - TTY-interactive first run prompts the user; non-TTY defaults to
//!     off and does not persist a decision (see `consent` submodule).
//!
//! Hot-swap: swapping from PostHog to another vendor = replace the
//! contents of this file. No caller (`main.rs`, `auth.rs`, etc.) holds
//! a reference to anything PostHog-specific.

pub mod consent;

use std::path::PathBuf;

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;

use crate::auth::validate_profile_name;

/// Compiled-in public DSN for the share-back project (isolated from
/// production). Used when `NYXID_SHARE_ANALYTICS=true` is set with no
/// explicit `NYXID_TELEMETRY_DSN`. Safe to publish — PostHog ingest
/// keys cannot read or delete.
///
/// Left empty by default; the release process is expected to bake in
/// the real value. A zero-length constant here means `share_analytics`
/// silently degrades to "off", which is the safest possible default.
const NYXID_PUBLIC_TELEMETRY_DSN: &str = "phc_pHHMZRXY8ymzBy9uwiGmAVDtGvGpDTiyXH2zs7bQWEgM";
const NYXID_PUBLIC_TELEMETRY_HOST: &str = "https://us.i.posthog.com";

const DEFAULT_HOST: &str = "https://us.i.posthog.com";
const ANON_ID_FILE_NAME: &str = "anon_id";
const TRACK_TIMEOUT_MS: u64 = 1000;

/// Canonical CLI-originated event. Only one variant today — the command
/// invocation wrapper. All other per-domain events are emitted by the
/// backend with `surface="cli"` via the `X-NyxID-Client: cli` header.
#[derive(Debug, Clone)]
pub enum CliEvent {
    CommandInvoked {
        command_group: &'static str,
        subcommand: &'static str,
        exit_code: i32,
        duration_ms: u64,
        profile: Option<String>,
        os: &'static str,
        arch: &'static str,
    },
}

impl CliEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::CommandInvoked { .. } => "cli.command_invoked",
        }
    }

    fn properties(&self) -> serde_json::Value {
        match self {
            Self::CommandInvoked {
                command_group,
                subcommand,
                exit_code,
                duration_ms,
                profile,
                os,
                arch,
            } => json!({
                "command_group": command_group,
                "subcommand": subcommand,
                "exit_code": exit_code,
                "duration_ms": duration_ms,
                "profile": profile,
                "os": os,
                "arch": arch,
            }),
        }
    }
}

/// Vendor-neutral telemetry client. Hot-swappable (§5.0).
#[derive(Clone)]
pub struct TelemetryClient {
    dsn: String,
    host: String,
    /// NyxID user UUID after login; anon UUID pre-login. Switches via
    /// [`TelemetryClient::identify`].
    distinct_id: String,
    http: Client,
    cli_version: &'static str,
    profile: Option<String>,
}

impl TelemetryClient {
    /// Resolve DSN + consent per §3 precedence and build a client.
    /// Returns `None` for the hard-off case (no DSN resolvable, or user
    /// has declined consent). Callers treat `None` as no-op.
    ///
    /// Does NOT prompt — see [`consent::prompt_if_needed_interactive`]
    /// which must run before this on an interactive TTY.
    ///
    /// Consent is resolved via
    /// [`consent::resolve_consent_preferring_profile`]: default profile
    /// wins, but any explicit per-profile choice persisted by older
    /// releases is honored so upgrades don't silently override historical
    /// opt-outs. Matches `api::build_cli_http_client` and the `main.rs`
    /// first-run prompt. The `profile` argument is still used below for
    /// the anon distinct_id file path — that's identity isolation (a
    /// per-profile concern), not consent (a user-global concern).
    pub fn init(profile: Option<&str>) -> Option<Self> {
        let state = consent::resolve_consent_preferring_profile(profile);
        if !state.enabled {
            return None;
        }

        let (dsn, host) = resolve_dsn()?;
        let distinct_id = resolve_distinct_id(profile).ok()?;
        let http = Client::builder()
            .timeout(std::time::Duration::from_millis(TRACK_TIMEOUT_MS))
            .user_agent(concat!("nyxid-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;

        Some(Self {
            dsn,
            host,
            distinct_id,
            http,
            cli_version: env!("CARGO_PKG_VERSION"),
            profile: profile.map(str::to_owned),
        })
    }

    /// Bounded-wait emission. Awaits the POST up to `TRACK_TIMEOUT_MS`;
    /// if the vendor is slow or unreachable, the timeout fires and the
    /// CLI proceeds with exit. Under normal network, a successful
    /// capture takes ~50–200ms, so the user-visible cost is small.
    ///
    /// Previously this was fire-and-forget via `tokio::spawn`, but
    /// `#[tokio::main]` tears the runtime down when `main()` returns,
    /// which cancelled the spawned task before the TCP handshake
    /// completed — ~100% loss on short commands. Awaiting inline is
    /// the simplest fix that makes events actually reach the vendor.
    pub async fn track(&self, event: CliEvent) {
        let body = self.build_capture_body(event.name(), event.properties());
        let url = format!("{host}/capture/", host = self.host);
        let fut = self.http.post(&url).json(&body).send();
        let _ = tokio::time::timeout(std::time::Duration::from_millis(TRACK_TIMEOUT_MS), fut).await;
    }

    /// Associate the currently-active anon identity with `user_id`.
    /// Called from `run_login()` after tokens are saved. Subsequent
    /// events use `user_id` as the distinct_id.
    ///
    /// The anon→user_id merge is handed to the vendor; wire protocol is
    /// invisible to the caller. On PostHog today this posts an
    /// `$identify` event with `$anon_distinct_id`. On a future vendor
    /// swap the method body changes; the caller does not.
    pub async fn identify(&mut self, user_id: &str) {
        let anon_id = self.distinct_id.clone();
        let user_id_owned = user_id.to_string();
        // Bump our own distinct_id immediately so subsequent `track`
        // calls go out under the user_id.
        self.distinct_id = user_id_owned.clone();

        let url = format!("{host}/capture/", host = self.host);
        let body = json!({
            "api_key": self.dsn,
            "event": "$identify",
            "distinct_id": user_id_owned,
            "properties": {
                "$anon_distinct_id": anon_id,
                "surface": "cli",
                "app_version": self.cli_version,
            },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        let fut = self.http.post(&url).json(&body).send();
        let _ = tokio::time::timeout(std::time::Duration::from_millis(TRACK_TIMEOUT_MS), fut).await;
    }

    /// Clear the local anon identity — called from `run_logout` and from
    /// `nyxid telemetry disable`. The next command invocation resumes
    /// with a fresh anon UUID.
    pub fn reset(&self) {
        if let Some(path) = anon_id_path(self.profile.as_deref()) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn build_capture_body(
        &self,
        event_name: &str,
        mut properties: serde_json::Value,
    ) -> serde_json::Value {
        if let Some(obj) = properties.as_object_mut() {
            obj.insert("surface".into(), json!("cli"));
            obj.insert("app_version".into(), json!(self.cli_version));
        }
        json!({
            "api_key": self.dsn,
            "event": event_name,
            "distinct_id": self.distinct_id,
            "properties": properties,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })
    }
}

/// Resolve DSN + host per §3 precedence.
fn resolve_dsn() -> Option<(String, String)> {
    // 1. Explicit user-set DSN.
    if let Some(dsn) = std::env::var("NYXID_TELEMETRY_DSN")
        .ok()
        .filter(|s| !s.is_empty())
    {
        let host = std::env::var("NYXID_TELEMETRY_HOST")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_HOST.to_string())
            .trim_end_matches('/')
            .trim_end_matches("/capture")
            .trim_end_matches('/')
            .to_string();
        return Some((dsn, host));
    }
    // 2. Community share-back.
    let share = std::env::var("NYXID_SHARE_ANALYTICS")
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"))
        .unwrap_or(false);
    if share && !NYXID_PUBLIC_TELEMETRY_DSN.is_empty() {
        return Some((
            NYXID_PUBLIC_TELEMETRY_DSN.to_string(),
            NYXID_PUBLIC_TELEMETRY_HOST.to_string(),
        ));
    }
    None
}

/// Read (or lazily create) the anon UUID for the given profile. The file
/// lives at `~/.nyxid/anon_id` (default profile) or
/// `~/.nyxid/profiles/{name}/anon_id`. Shared with the user-id file so
/// deleting one on logout / disable cleans both surfaces.
fn resolve_distinct_id(profile: Option<&str>) -> Result<String> {
    // Authenticated user_id (from prior identity plumbing in `auth.rs`)
    // wins whenever it exists — an existing login means we already
    // aliased anon → user_id on that machine.
    if let Some(uid) = crate::auth::read_saved_user_id_for(profile) {
        return Ok(uid);
    }

    let path = anon_id_path(profile).context("home directory not resolvable")?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    // First invocation on this profile: mint a fresh anon UUID.
    let fresh = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if std::fs::write(&path, &fresh).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
    Ok(fresh)
}

pub(crate) fn anon_id_path(profile: Option<&str>) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let base = home.join(".nyxid");
    let dir = match profile {
        None => base,
        Some(name) => {
            validate_profile_name(name).ok()?;
            base.join("profiles").join(name)
        }
    };
    Some(dir.join(ANON_ID_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_event_name_returns_expected_value() {
        let event = CliEvent::CommandInvoked {
            command_group: "auth",
            subcommand: "login",
            exit_code: 0,
            duration_ms: 150,
            profile: None,
            os: "macos",
            arch: "aarch64",
        };
        assert_eq!(event.name(), "cli.command_invoked");
    }

    #[test]
    fn cli_event_properties_contain_all_fields() {
        let event = CliEvent::CommandInvoked {
            command_group: "service",
            subcommand: "add",
            exit_code: 1,
            duration_ms: 500,
            profile: Some("work".to_string()),
            os: "linux",
            arch: "x86_64",
        };
        let props = event.properties();
        let obj = props.as_object().unwrap();
        assert_eq!(obj.get("command_group").unwrap(), "service");
        assert_eq!(obj.get("subcommand").unwrap(), "add");
        assert_eq!(obj.get("exit_code").unwrap(), 1);
        assert_eq!(obj.get("duration_ms").unwrap(), 500);
        assert_eq!(obj.get("profile").unwrap(), "work");
        assert_eq!(obj.get("os").unwrap(), "linux");
        assert_eq!(obj.get("arch").unwrap(), "x86_64");
    }

    #[test]
    fn cli_event_properties_with_null_profile() {
        let event = CliEvent::CommandInvoked {
            command_group: "auth",
            subcommand: "logout",
            exit_code: 0,
            duration_ms: 10,
            profile: None,
            os: "macos",
            arch: "aarch64",
        };
        let props = event.properties();
        let obj = props.as_object().unwrap();
        assert!(obj.get("profile").unwrap().is_null());
    }

    #[test]
    fn anon_id_path_default_profile() {
        let path = anon_id_path(None).unwrap();
        assert!(path.ends_with(".nyxid/anon_id"));
    }

    #[test]
    fn anon_id_path_named_profile() {
        let path = anon_id_path(Some("work")).unwrap();
        assert!(path.ends_with(".nyxid/profiles/work/anon_id"));
    }
}
