//! Explicit CLI events travel through authenticated NyxID admission/dispatch.
//! No direct vendor ingest, anonymous identity, or positive account cache.
pub mod account_binding;
pub mod consent;
use serde_json::json;

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
                os,
                arch,
            } => json!({
                "command_group": command_group,
                "subcommand": subcommand,
                "exit_code": exit_code,
                "duration_ms": duration_ms,
                "os": os,
                "arch": arch,
            }),
        }
    }
}

#[derive(Clone)]
pub struct TelemetryClient {
    identity: account_binding::Identity,
    local_revision: String,
    profile: Option<String>,
}
impl TelemetryClient {
    pub fn init(profile: Option<&str>) -> Option<Self> {
        let identity = account_binding::saved_identity(profile)?;
        let local_revision = account_binding::permitted_revision(profile, &identity)?;
        Some(Self {
            identity,
            local_revision,
            profile: profile.map(str::to_owned),
        })
    }
    fn current(&self) -> bool {
        account_binding::permitted_revision(self.profile.as_deref(), &self.identity).as_deref()
            == Some(&self.local_revision)
    }
    pub async fn track(&self, event: CliEvent) {
        // Capture identity at command start. Login/logout/switch cannot attribute
        // the completed command to a successor identity or local permission.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(1000), async {
            if !self.current() {
                return;
            }
            let auth = crate::cli::AuthArgs {
                base_url: Some(self.identity.base_url.clone()),
                access_token: None,
                access_token_env: "NYXID_ACCESS_TOKEN".into(),
                profile: self.profile.clone(),
                output: crate::cli::OutputFormat::Json,
            };
            // Explicit environment credentials are not this captured session.
            if crate::auth::resolve_access_token_override(&auth).is_some() {
                return;
            }
            let Ok(mut api) = crate::api::ApiClient::from_auth_checked(&auth).await else {
                return;
            };
            if !self.current() {
                return;
            }
            let Ok(permission) = api
                .get::<serde_json::Value>("/users/me/preferences/analytics")
                .await
            else {
                return;
            };
            if !self.current()
                || permission["account_id"].as_str() != Some(&self.identity.account_id)
                || permission["enabled"] != true
            {
                return;
            }
            let Some(revision) = permission["revision"].as_str().filter(|v| !v.is_empty()) else {
                return;
            };
            let _ = api
                .post_empty(
                    "/users/me/analytics/events",
                    &json!({
                        "account_id": self.identity.account_id, "revision": revision,
                        "event": { "name": event.name(), "props": event.properties() }
                    }),
                )
                .await;
        })
        .await;
    }
}

/// Cleanup path only: new versions never mint or dispatch anonymous identities.
pub(crate) fn anon_id_path(profile: Option<&str>) -> Option<std::path::PathBuf> {
    let mut base = dirs::home_dir()?.join(".nyxid");
    if let Some(profile) = profile {
        crate::auth::validate_profile_name(profile).ok()?;
        base = base.join("profiles").join(profile);
    }
    Some(base.join("anon_id"))
}
