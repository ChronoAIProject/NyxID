//! `nyxid telemetry {enable,disable,status}` — the canonical editor for
//! the persisted telemetry consent flag at `~/.nyxid/config.toml`.
//!
//! `enable`  → `{enabled=true,  asked=true}` in config
//! `disable` → `{enabled=false, asked=true}` in config + deletes any
//!             cached anon UUID so a re-enable produces a fresh trail
//! `status`  → prints the resolved state and where it came from

use anyhow::Result;

use crate::cli::TelemetryCommands;
use crate::telemetry;

pub async fn run(command: TelemetryCommands, profile: Option<&str>) -> Result<()> {
    match command {
        TelemetryCommands::Enable => {
            telemetry::consent::persist_choice(profile, true)?;
            eprintln!("Telemetry enabled. Run `nyxid telemetry status` to confirm.");
            Ok(())
        }
        TelemetryCommands::Disable => {
            telemetry::consent::persist_choice(profile, false)?;
            // Wipe any cached anon UUID so re-enable starts fresh.
            if let Some(path) = telemetry::anon_id_path(profile) {
                let _ = std::fs::remove_file(path);
            }
            eprintln!("Telemetry disabled. No events will be sent.");
            Ok(())
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
                    "env: NYXID_TELEMETRY=on (forced enable)"
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
            println!("Telemetry: {enabled_str}");
            println!("Source:    {source}");
            println!("Persisted: {}", state.persisted);
            if state.needs_prompt {
                println!(
                    "(A prompt will appear on the next interactive run to persist this choice.)"
                );
            }
            Ok(())
        }
    }
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
            // SAFETY: serialized via env_lock; only one test mutates env at a time.
            unsafe {
                std::env::set_var("HOME", &home);
                std::env::remove_var("NYXID_TELEMETRY");
                std::env::remove_var("DO_NOT_TRACK");
            }
            Self {
                _lock: lock,
                _tmp: tmp,
                home,
                prev_home,
                prev_telemetry,
                prev_dnt,
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

    // `run` is async but performs no `.await` on any IO future — it only
    // calls the synchronous consent/anon-id helpers — so holding the
    // (non-async) env lock across the awaits is safe and is what the
    // clippy allow documents.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn enable_persists_enabled_choice_to_config() {
        let guard = HomeGuard::new();

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
