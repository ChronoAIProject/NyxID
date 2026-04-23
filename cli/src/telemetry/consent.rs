//! First-run consent resolver for the CLI.
//!
//! Implements the precedence ladder from `docs/TELEMETRY.md` §3.
//! `resolve_consent` is pure — it only reads env vars + the config
//! file, never prompts. `prompt_if_needed_interactive` handles the
//! one-time interactive prompt on a real TTY and persists the answer.
//!
//! Resolution order (first match wins):
//!   1. `NYXID_TELEMETRY=off` env var → off (always wins).
//!   2. `NYXID_TELEMETRY=on` env var → on (only if a DSN resolves).
//!   3. `[telemetry] enabled=true` in `~/.nyxid/config.toml` → on.
//!   4. `[telemetry] enabled=false AND asked=true` → off.
//!   5. Config missing OR `asked=false` → `FirstRunPending`.
//!
//! `FirstRunPending` + TTY → prompt. `FirstRunPending` + non-TTY →
//! treated as "No" for this invocation but NOT persisted, so the next
//! interactive run re-prompts.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use is_terminal::IsTerminal;
use serde::{Deserialize, Serialize};

use crate::auth::validate_profile_name;

const CONFIG_FILE_NAME: &str = "config.toml";

/// Where the resolved choice came from, for `nyxid telemetry status` to
/// print an honest explanation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentSource {
    /// `NYXID_TELEMETRY=off` (forced disable)
    EnvVarOff,
    /// `NYXID_TELEMETRY=on` (forced enable)
    EnvVarOn,
    /// Config file says `enabled=true` and `asked=true`.
    ConfigEnabled,
    /// Config file says `enabled=false` and `asked=true`.
    ConfigDeclined,
    /// No durable choice yet — either `asked=false` or no config file.
    /// On interactive TTY the user is prompted once; on non-TTY we
    /// default to "off" but do not persist.
    FirstRunPending,
}

/// Resolved telemetry consent state. `resolve_consent` is pure; the
/// prompt flow is a separate function so tests (and non-interactive
/// CLI paths) can inspect state without triggering IO.
#[derive(Clone, Debug)]
pub struct ConsentState {
    pub enabled: bool,
    pub source: ConsentSource,
    /// True iff the choice is already on disk (or baked into env).
    pub persisted: bool,
    /// True iff `source == FirstRunPending` AND stdin is a TTY. The
    /// caller should invoke [`prompt_if_needed_interactive`] to honor.
    pub needs_prompt: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct TelemetrySection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    asked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct NyxidConfig {
    #[serde(default)]
    telemetry: TelemetrySection,
}

/// Pure function: resolve the current effective consent state. Reads
/// env vars + config file; does not prompt, does not write.
pub fn resolve_consent(profile: Option<&str>) -> ConsentState {
    // Steps 1 and 2: env var override.
    if let Ok(raw) = std::env::var("NYXID_TELEMETRY") {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "false" | "0" | "no" => {
                return ConsentState {
                    enabled: false,
                    source: ConsentSource::EnvVarOff,
                    persisted: true,
                    needs_prompt: false,
                };
            }
            "on" | "true" | "1" | "yes" => {
                return ConsentState {
                    enabled: true,
                    source: ConsentSource::EnvVarOn,
                    persisted: true,
                    needs_prompt: false,
                };
            }
            _ => {} // fall through
        }
    }

    // Steps 3 and 4: config file.
    let cfg = load_config(profile).unwrap_or_default();
    if cfg.telemetry.asked {
        return if cfg.telemetry.enabled {
            ConsentState {
                enabled: true,
                source: ConsentSource::ConfigEnabled,
                persisted: true,
                needs_prompt: false,
            }
        } else {
            ConsentState {
                enabled: false,
                source: ConsentSource::ConfigDeclined,
                persisted: true,
                needs_prompt: false,
            }
        };
    }

    // Step 5: first-run pending. Default off until interactive consent
    // (or explicit env var) flips it on. Prompt only on a TTY.
    ConsentState {
        enabled: false,
        source: ConsentSource::FirstRunPending,
        persisted: false,
        needs_prompt: std::io::stdin().is_terminal(),
    }
}

/// If `state.needs_prompt` is true, print a one-line consent question
/// to stderr, read y/N from stdin, persist the answer to
/// `~/.nyxid/config.toml`, and update `state` in place. Otherwise
/// no-op.
///
/// Never bails the calling command — prompt refusal returns `Ok(())`
/// and the command proceeds (just without telemetry).
pub fn prompt_if_needed_interactive(profile: Option<&str>, state: &mut ConsentState) -> Result<()> {
    if !state.needs_prompt {
        return Ok(());
    }

    eprintln!();
    eprintln!("NyxID collects anonymous usage telemetry to help us improve the CLI.");
    eprintln!("We never capture credentials, command arguments, or file contents.");
    eprintln!("You can change this later with `nyxid telemetry enable|disable`.");
    eprint!("Enable telemetry for this machine? [y/N] ");
    std::io::stderr().flush().ok();

    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        // User killed stdin — treat as declined but don't persist,
        // so the next real interactive run still prompts.
        return Ok(());
    }
    let answer = line.trim().to_ascii_lowercase();
    let enabled = matches!(answer.as_str(), "y" | "yes");

    persist_choice(profile, enabled)?;
    state.enabled = enabled;
    state.source = if enabled {
        ConsentSource::ConfigEnabled
    } else {
        ConsentSource::ConfigDeclined
    };
    state.persisted = true;
    state.needs_prompt = false;
    Ok(())
}

/// Write `{enabled, asked=true}` to `~/.nyxid/config.toml`. Called by
/// the `telemetry enable|disable` subcommand and by the interactive
/// prompt flow.
pub fn persist_choice(profile: Option<&str>, enabled: bool) -> Result<()> {
    let path = config_path(profile).context("home directory not resolvable")?;

    let mut cfg = load_config(profile).unwrap_or_default();
    cfg.telemetry.enabled = enabled;
    cfg.telemetry.asked = true;

    let rendered = toml::to_string_pretty(&cfg).context("render config.toml")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, rendered).context("write config.toml")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn load_config(profile: Option<&str>) -> Option<NyxidConfig> {
    let path = config_path(profile)?;
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<NyxidConfig>(&text).ok()
}

fn config_path(profile: Option<&str>) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let base = home.join(".nyxid");
    let dir = match profile {
        None => base,
        Some(name) => {
            validate_profile_name(name).ok()?;
            base.join("profiles").join(name)
        }
    };
    Some(dir.join(CONFIG_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn with_temp_home<F: FnOnce()>(f: F) {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let prev = std::env::var_os("HOME");
        // SAFETY: serialized via test_lock; only one test at a time.
        unsafe {
            std::env::set_var("HOME", tmp.path());
        }
        let prev_telemetry = std::env::var_os("NYXID_TELEMETRY");
        unsafe {
            std::env::remove_var("NYXID_TELEMETRY");
        }
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match prev_telemetry {
                Some(v) => std::env::set_var("NYXID_TELEMETRY", v),
                None => std::env::remove_var("NYXID_TELEMETRY"),
            }
        }
    }

    #[test]
    fn first_run_is_pending_when_no_config() {
        with_temp_home(|| {
            let s = resolve_consent(None);
            assert_eq!(s.source, ConsentSource::FirstRunPending);
            assert!(!s.enabled);
            assert!(!s.persisted);
        });
    }

    #[test]
    fn env_var_off_beats_config() {
        with_temp_home(|| {
            persist_choice(None, true).unwrap();
            // SAFETY: see above.
            unsafe {
                std::env::set_var("NYXID_TELEMETRY", "off");
            }
            let s = resolve_consent(None);
            assert_eq!(s.source, ConsentSource::EnvVarOff);
            assert!(!s.enabled);
        });
    }

    #[test]
    fn persisted_choice_survives_reload() {
        with_temp_home(|| {
            persist_choice(None, false).unwrap();
            let s = resolve_consent(None);
            assert_eq!(s.source, ConsentSource::ConfigDeclined);
            assert!(!s.enabled);
            assert!(s.persisted);
        });
    }

    #[test]
    fn persisted_enable_is_config_enabled() {
        with_temp_home(|| {
            persist_choice(None, true).unwrap();
            let s = resolve_consent(None);
            assert_eq!(s.source, ConsentSource::ConfigEnabled);
            assert!(s.enabled);
        });
    }
}
