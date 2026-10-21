//! CLI device controls and separate authenticated account permission controls.
//! Local choices are scoped to deployment base + account + profile. Negative
//! env signals always narrow; env=on grants neither local nor server permission.

use anyhow::Result;

use crate::cli::TelemetryCommands;
use crate::telemetry;

pub async fn run(command: TelemetryCommands, profile: Option<&str>) -> Result<()> {
    match command {
        TelemetryCommands::Enable => {
            telemetry::consent::persist_choice(profile, true)?;
            telemetry::account_binding::persist_choice(profile, true)?;
            eprintln!(
                "This account is enabled on this CLI profile/device. Account permission is separate; use `nyxid telemetry account-enable` to grant it."
            );
            Ok(())
        }
        TelemetryCommands::Disable => {
            telemetry::consent::persist_choice(profile, false)?;
            if telemetry::account_binding::saved_identity(profile).is_some() {
                telemetry::account_binding::persist_choice(profile, false)?;
            }
            // Remove obsolete anonymous identity from older versions.
            if let Some(path) = telemetry::anon_id_path(profile) {
                let _ = std::fs::remove_file(path);
            }
            eprintln!("Telemetry disabled. No events will be sent.");
            Ok(())
        }
        TelemetryCommands::AccountEnable => set_account_permission(profile, true).await,
        TelemetryCommands::AccountDisable => {
            // Local denial precedes network work and remains on failure.
            telemetry::consent::persist_choice(profile, false)?;
            if telemetry::account_binding::saved_identity(profile).is_some() {
                telemetry::account_binding::persist_choice(profile, false)?;
            }
            set_account_permission(profile, false).await
        }
        TelemetryCommands::Status => {
            let state = telemetry::consent::resolve_consent(profile);
            let source = match state.source {
                telemetry::consent::ConsentSource::DoNotTrack => {
                    "env: DO_NOT_TRACK is set (forced disable, not persisted)"
                }
                telemetry::consent::ConsentSource::EnvVarOff => {
                    "env: NYXID_TELEMETRY=off (forced disable)"
                }
                telemetry::consent::ConsentSource::EnvVarOn => {
                    "env: NYXID_TELEMETRY=on (still requires account-bound local choice and live account permission)"
                }
                telemetry::consent::ConsentSource::ConfigEnabled => {
                    "config: ~/.nyxid/config.toml (enabled=true)"
                }
                telemetry::consent::ConsentSource::ConfigDeclined => {
                    "config: ~/.nyxid/config.toml (enabled=false)"
                }
                telemetry::consent::ConsentSource::FirstRunPending => {
                    "default: no choice persisted yet"
                }
            };
            let enabled_str = if state.enabled { "ON" } else { "OFF" };
            println!("Local environment/config narrowing: {enabled_str}");
            if let Some(identity) = telemetry::account_binding::saved_identity(profile) {
                println!(
                    "Account/device binding: {}",
                    if telemetry::account_binding::permitted_revision(profile, &identity).is_some()
                    {
                        "ON"
                    } else {
                        "OFF"
                    }
                );
                match account_client(profile, &identity).await {
                    Ok(mut api) => match api
                        .get::<serde_json::Value>("/users/me/preferences/analytics")
                        .await
                    {
                        Ok(permission)
                            if permission["account_id"].as_str()
                                == Some(identity.account_id.as_str()) =>
                        {
                            println!(
                                "Account permission: {}",
                                if permission["enabled"] == true {
                                    "ON"
                                } else {
                                    "OFF"
                                }
                            )
                        }
                        _ => println!("Account permission: unavailable; collection fails closed"),
                    },
                    Err(_) => println!("Account permission: unavailable; collection fails closed"),
                }
            } else {
                println!("Account/device binding: OFF (sign in first)");
            }
            println!("Source:    {source}");
            println!("Persisted: {}", state.persisted);
            Ok(())
        }
    }
}

async fn account_client(
    profile: Option<&str>,
    identity: &telemetry::account_binding::Identity,
) -> Result<crate::api::ApiClient> {
    let auth = crate::cli::AuthArgs {
        base_url: Some(identity.base_url.clone()),
        access_token: None,
        access_token_env: "NYXID_ACCESS_TOKEN".into(),
        profile: profile.map(str::to_owned),
        output: crate::cli::OutputFormat::Json,
    };
    anyhow::ensure!(
        crate::auth::resolve_access_token_override(&auth).is_none(),
        "Use the selected saved human login for analytics preferences"
    );
    crate::api::ApiClient::from_auth_checked(&auth).await
}
async fn set_account_permission(profile: Option<&str>, enabled: bool) -> Result<()> {
    let identity = telemetry::account_binding::saved_identity(profile)
        .ok_or_else(|| anyhow::anyhow!("Sign in with a human account first"))?;
    let mut api = account_client(profile, &identity).await?;
    anyhow::ensure!(
        telemetry::account_binding::saved_identity(profile).as_ref() == Some(&identity),
        "Account changed; retry"
    );
    let saved: serde_json::Value = api
        .put(
            "/users/me/preferences/analytics",
            &serde_json::json!({"account_id": identity.account_id, "enabled":enabled}),
        )
        .await?;
    anyhow::ensure!(
        telemetry::account_binding::saved_identity(profile).as_ref() == Some(&identity)
            && saved["account_id"].as_str() == Some(identity.account_id.as_str()),
        "Account changed; local analytics remains off"
    );
    anyhow::ensure!(
        saved["enabled"] == enabled,
        "Account analytics preference was not confirmed"
    );
    println!(
        "Account analytics permission {}. Device choice is separate.",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::MutexGuard;

    use super::*;
    use crate::telemetry::consent::{self, ConsentSource};

    /// RAII guard that points `$HOME` at a fresh temp dir and clears the
    /// two telemetry env overrides for the duration of a single test,
    /// restoring everything on drop. Holds the process-global env lock so
    /// concurrent cargo test threads don't race on `$HOME` (cargo runs
    /// tests multi-threaded; every `HOME` mutator must share one mutex).
    /// Mirrors the `HomeGuard` pattern in `api.rs` / `telemetry::consent`.
    struct HomeGuard {
        _lock: MutexGuard<'static, ()>,
        _tmp: tempfile::TempDir,
        home: PathBuf,
        prev_home: Option<OsString>,
        prev_telemetry: Option<OsString>,
        prev_dnt: Option<OsString>,
        credential_env: Vec<(&'static str, Option<OsString>)>,
    }

    impl HomeGuard {
        fn new() -> Self {
            let lock = crate::test_support::env_lock()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let tmp = tempfile::tempdir().expect("tempdir");
            let home = tmp.path().to_path_buf();
            let prev_home = std::env::var_os("HOME");
            let prev_telemetry = std::env::var_os("NYXID_TELEMETRY");
            let prev_dnt = std::env::var_os("DO_NOT_TRACK");
            let credential_env = ["NYXID_ACCESS_TOKEN", "NYXID_API_KEY", "NYXID_PROFILE"]
                .into_iter()
                .map(|name| (name, std::env::var_os(name)))
                .collect::<Vec<_>>();
            // SAFETY: serialized via env_lock; only one test mutates env at a time.
            unsafe {
                std::env::set_var("HOME", &home);
                std::env::remove_var("NYXID_TELEMETRY");
                std::env::remove_var("DO_NOT_TRACK");
                for (name, _) in &credential_env {
                    std::env::remove_var(name);
                }
            }
            Self {
                _lock: lock,
                _tmp: tmp,
                home,
                prev_home,
                prev_telemetry,
                prev_dnt,
                credential_env,
            }
        }

        fn config_path(&self) -> PathBuf {
            self.home.join(".nyxid").join("config.toml")
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            // SAFETY: lock still held until this guard fully drops.
            unsafe {
                for (name, value) in self.credential_env.drain(..) {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
                match self.prev_home.take() {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
                match self.prev_telemetry.take() {
                    Some(v) => std::env::set_var("NYXID_TELEMETRY", v),
                    None => std::env::remove_var("NYXID_TELEMETRY"),
                }
                match self.prev_dnt.take() {
                    Some(v) => std::env::set_var("DO_NOT_TRACK", v),
                    None => std::env::remove_var("DO_NOT_TRACK"),
                }
            }
        }
    }

    fn seed_login(home: &std::path::Path, base: &str) {
        use base64::Engine;
        let token = format!("e30.{}.sig", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::json!({"sub":"11111111-2222-4333-8444-555555555555", "exp":chrono::Utc::now().timestamp()+3600}).to_string()));
        crate::auth::save_tokens_for(None, &token, None).unwrap();
        std::fs::write(home.join(".nyxid/base_url"), base).unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn account_disable_failure_stops_device_even_with_env_on() {
        let guard = HomeGuard::new();
        let server = wiremock::MockServer::start().await;
        seed_login(&guard.home, &server.uri());
        run(TelemetryCommands::Enable, None).await.unwrap();
        unsafe {
            std::env::set_var("NYXID_TELEMETRY", "on");
        }
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .respond_with(wiremock::ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        assert!(run(TelemetryCommands::AccountDisable, None).await.is_err());
        let identity = telemetry::account_binding::saved_identity(None).unwrap();
        assert!(telemetry::account_binding::permitted_revision(None, &identity).is_none());
        assert!(telemetry::TelemetryClient::init(None).is_none());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn relay_uses_deployment_prefix_and_fresh_device_headers_without_dsn() {
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{header, method, path},
        };
        let guard = HomeGuard::new();
        let server = wiremock::MockServer::start().await;
        let base = format!("{}/nyxid", server.uri());
        seed_login(&guard.home, &base);
        run(TelemetryCommands::Enable, None).await.unwrap();
        let event = || telemetry::CliEvent::CommandInvoked {
            command_group: "user",
            subcommand: "whoami",
            exit_code: 0,
            duration_ms: 1,
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        };
        Mock::given(method("GET")).and(path("/nyxid/api/v1/users/me/preferences/analytics"))
            .and(header("x-nyxid-client", "cli")).and(header("x-nyxid-analytics", "enabled"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"account_id":"11111111-2222-4333-8444-555555555555","enabled":true,"revision":"live"})))
            .expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/nyxid/api/v1/users/me/analytics/events"))
            .and(header("x-nyxid-client", "cli"))
            .and(header("x-nyxid-analytics", "enabled"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = telemetry::TelemetryClient::init(None).unwrap();
        client.track(event()).await;
        telemetry::account_binding::persist_choice(None, false).unwrap();
        client.track(event()).await;
        let token = crate::auth::read_saved_token_for(None).unwrap();
        assert!(telemetry::account_binding::device_headers(None, &base, &token).is_empty());
        telemetry::account_binding::persist_choice(None, true).unwrap();
        client.track(event()).await; // old command revision cannot inherit regrant
        let successor = telemetry::TelemetryClient::init(None).unwrap();
        std::fs::write(guard.home.join(".nyxid/login_generation"), "next-login").unwrap();
        successor.track(event()).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn enable_persists_enabled_choice_to_config() {
        let guard = HomeGuard::new();
        use base64::Engine;
        let token = format!(
            "e30.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(r#"{"sub":"11111111-2222-4333-8444-555555555555"}"#)
        );
        crate::auth::save_tokens_for(None, &token, None).unwrap();
        std::fs::write(
            guard.home.join(".nyxid/base_url"),
            "https://test.invalid/nyxid",
        )
        .unwrap();

        run(TelemetryCommands::Enable, None)
            .await
            .expect("telemetry enable should succeed");

        // Observable contract: the resolver now reports a persisted
        // enable sourced from the config file.
        let state = consent::resolve_consent(None);
        assert!(state.enabled, "enable should turn telemetry on");
        assert_eq!(state.source, ConsentSource::ConfigEnabled);
        assert!(state.persisted, "the choice must be on disk");
        assert!(
            guard.config_path().exists(),
            "config.toml should be written"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn disable_persists_declined_choice_to_config() {
        let _guard = HomeGuard::new();

        run(TelemetryCommands::Disable, None)
            .await
            .expect("telemetry disable should succeed");

        let state = consent::resolve_consent(None);
        assert!(!state.enabled, "disable should turn telemetry off");
        // `asked=true` + `enabled=false` resolves to ConfigDeclined, not
        // FirstRunPending — proving the choice was actually persisted.
        assert_eq!(state.source, ConsentSource::ConfigDeclined);
        assert!(state.persisted);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn disable_wipes_cached_anon_id() {
        let _guard = HomeGuard::new();

        // Seed a cached anon UUID at the default-profile path, the way a
        // prior `init()` would have. This wipe-on-disable behavior lives
        // only in this command, not in the consent module.
        let anon = crate::telemetry::anon_id_path(None).expect("anon id path");
        std::fs::create_dir_all(anon.parent().unwrap()).unwrap();
        std::fs::write(&anon, "11111111-2222-3333-4444-555555555555").unwrap();
        assert!(anon.exists(), "precondition: anon id file seeded");

        run(TelemetryCommands::Disable, None)
            .await
            .expect("telemetry disable should succeed");

        assert!(
            !anon.exists(),
            "disable must delete the cached anon id so a re-enable starts fresh"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn status_is_read_only_and_does_not_persist() {
        let guard = HomeGuard::new();

        run(TelemetryCommands::Status, None)
            .await
            .expect("telemetry status should succeed");

        // Status only prints; it must not create or mutate config. With no
        // prior choice the resolver stays at FirstRunPending.
        assert!(
            !guard.config_path().exists(),
            "status must not write config.toml"
        );
        assert_eq!(
            consent::resolve_consent(None).source,
            ConsentSource::FirstRunPending
        );
    }
}
