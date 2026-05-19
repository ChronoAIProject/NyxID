//! First-run consent resolver for the CLI.
//!
//! Implements the precedence ladder from `docs/TELEMETRY.md` §3, plus
//! the industry-standard `DO_NOT_TRACK=1` signal honored by Homebrew,
//! Netlify, GitHub CLI, and Meteor (see consoledonottrack.com).
//! `resolve_consent` is pure — it only reads env vars + the config
//! file, never prompts. `prompt_if_needed_interactive` handles the
//! one-time interactive prompt on a real TTY and persists the answer.
//!
//! Resolution order (first match wins):
//!   0. `DO_NOT_TRACK=1` (non-empty, non-zero) → off, never persisted.
//!   1. `NYXID_TELEMETRY=off` env var → off (always wins over config).
//!   2. `NYXID_TELEMETRY=on` env var → on (only if a DSN resolves).
//!   3. `[telemetry] enabled=true` in `~/.nyxid/config.toml` → on.
//!   4. `[telemetry] enabled=false AND asked=true` → off.
//!   5. Config missing OR `asked=false` → `FirstRunPending`.
//!
//! `FirstRunPending` + TTY → prompt. `FirstRunPending` + non-TTY →
//! treated as "No" for this invocation but NOT persisted, so the next
//! interactive run re-prompts.
//!
//! `DO_NOT_TRACK` is checked before `NYXID_TELEMETRY` so a user who has
//! opted out globally across all their dev tools is never overridden by
//! a stale `NYXID_TELEMETRY=on` in the same environment.

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
    /// `DO_NOT_TRACK=1` (industry-standard global opt-out). Beats every
    /// other source. Never persisted to config — honoring the convention
    /// that `DO_NOT_TRACK` is a per-invocation global signal.
    DoNotTrack,
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

/// Migration-aware consent lookup. v1 consent is user-global (read +
/// edited against the default profile), but older releases persisted
/// explicit per-profile consent via the login prompt. Silently
/// erasing those choices on upgrade is a privacy regression — a user
/// who opted OUT on `--profile dev` would find their default
/// profile's "Yes" override that explicit opt-out.
///
/// Resolution order:
///   1. Env overrides (DO_NOT_TRACK / NYXID_TELEMETRY) — same as
///      `resolve_consent`, global regardless of profile.
///   2. If `profile` is Some AND that profile's config has
///      `asked=true`, honor that explicit historical choice.
///   3. Fall back to the default profile's config.
///
/// Going forward, only the default profile is written to (by
/// `nyxid telemetry enable|disable` and the first-run prompt), so
/// step 2 only matches persisted choices from pre-v1 releases.
pub fn resolve_consent_preferring_profile(profile: Option<&str>) -> ConsentState {
    // Env overrides win regardless of profile — these are global
    // signals by design. `resolve_consent(None)` handles them at the
    // top of its ladder, so if any env override is active we'll pick
    // it up via the default-profile path without consulting the
    // named profile's config at all.
    if let Ok(raw) = std::env::var("DO_NOT_TRACK") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && trimmed != "0" {
            return resolve_consent(None);
        }
    }
    // Only treat RECOGNIZED NYXID_TELEMETRY values as a global override.
    // A garbage value (`NYXID_TELEMETRY=maybe`) must not bypass the
    // per-profile historical-consent check below — that would let a
    // stray env export silently re-enable telemetry for a user who
    // had opted out on a named profile in a prior release. Matches
    // the set of values parsed by `resolve_consent` itself.
    if let Ok(raw) = std::env::var("NYXID_TELEMETRY") {
        let norm = raw.trim().to_ascii_lowercase();
        if matches!(
            norm.as_str(),
            "off" | "false" | "0" | "no" | "on" | "true" | "1" | "yes"
        ) {
            return resolve_consent(None);
        }
    }

    // No env override. If the user has an explicit per-profile
    // choice from a prior release, honor it so we don't silently
    // override an opt-out (or an opt-in the user remembers making
    // on that profile).
    if let Some(name) = profile
        && let Some(cfg) = load_config(Some(name))
        && cfg.telemetry.asked
    {
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

    // No env override, no per-profile historical choice. Fall back
    // to the default profile's config / first-run state.
    resolve_consent(None)
}

/// Pure function: resolve the current effective consent state. Reads
/// env vars + config file; does not prompt, does not write.
pub fn resolve_consent(profile: Option<&str>) -> ConsentState {
    // Step 0: `DO_NOT_TRACK` (consoledonottrack.com). Values that count
    // as "please do not track me" are any non-empty string other than
    // literal "0" — mirrors the convention in Homebrew, Netlify, and
    // GitHub CLI. Not persisted: `DO_NOT_TRACK` is a per-invocation
    // signal, not a durable preference.
    if let Ok(raw) = std::env::var("DO_NOT_TRACK") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && trimmed != "0" {
            return ConsentState {
                enabled: false,
                source: ConsentSource::DoNotTrack,
                persisted: false,
                needs_prompt: false,
            };
        }
    }

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
    eprintln!("This choice applies to this machine only — the web dashboard and");
    eprintln!("mobile app manage their own telemetry settings.");
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
        let prev_dnt = std::env::var_os("DO_NOT_TRACK");
        unsafe {
            std::env::remove_var("NYXID_TELEMETRY");
            std::env::remove_var("DO_NOT_TRACK");
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
            match prev_dnt {
                Some(v) => std::env::set_var("DO_NOT_TRACK", v),
                None => std::env::remove_var("DO_NOT_TRACK"),
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

    #[test]
    fn do_not_track_beats_config_enabled() {
        with_temp_home(|| {
            persist_choice(None, true).unwrap();
            // SAFETY: serialized via test_lock; only one test at a time.
            unsafe {
                std::env::set_var("DO_NOT_TRACK", "1");
            }
            let s = resolve_consent(None);
            assert_eq!(s.source, ConsentSource::DoNotTrack);
            assert!(!s.enabled);
            // DO_NOT_TRACK is a per-invocation signal — never persisted.
            assert!(!s.persisted);
        });
    }

    #[test]
    fn do_not_track_beats_env_var_on() {
        with_temp_home(|| {
            // SAFETY: serialized via test_lock; only one test at a time.
            unsafe {
                std::env::set_var("NYXID_TELEMETRY", "on");
                std::env::set_var("DO_NOT_TRACK", "1");
            }
            let s = resolve_consent(None);
            assert_eq!(s.source, ConsentSource::DoNotTrack);
            assert!(!s.enabled);
        });
    }

    #[test]
    fn do_not_track_zero_is_not_active() {
        with_temp_home(|| {
            persist_choice(None, true).unwrap();
            // SAFETY: serialized via test_lock; only one test at a time.
            unsafe {
                std::env::set_var("DO_NOT_TRACK", "0");
            }
            let s = resolve_consent(None);
            // "0" means "do track me" per the consoledonottrack.com spec.
            // Precedence falls through to the config.
            assert_eq!(s.source, ConsentSource::ConfigEnabled);
            assert!(s.enabled);
        });
    }

    #[test]
    fn empty_do_not_track_is_not_active() {
        with_temp_home(|| {
            persist_choice(None, true).unwrap();
            // SAFETY: serialized via test_lock; only one test at a time.
            unsafe {
                std::env::set_var("DO_NOT_TRACK", "");
            }
            let s = resolve_consent(None);
            // Empty = unset = fall through to normal resolution.
            assert_eq!(s.source, ConsentSource::ConfigEnabled);
            assert!(s.enabled);
        });
    }

    #[test]
    fn preferring_profile_honors_prior_release_profile_optout() {
        // Simulates the migration scenario: a user of an older release
        // opted OUT via the login prompt on `--profile dev`, which
        // persisted `{enabled:false, asked:true}` to the profile
        // config. They later opt IN on the default profile (v1
        // behavior). The profile opt-out must still be honored — a
        // silent override would be a privacy regression.
        with_temp_home(|| {
            persist_choice(None, true).unwrap();
            persist_choice(Some("dev"), false).unwrap();
            let s = resolve_consent_preferring_profile(Some("dev"));
            assert_eq!(s.source, ConsentSource::ConfigDeclined);
            assert!(!s.enabled);
        });
    }

    #[test]
    fn preferring_profile_falls_back_to_default_when_profile_unset() {
        with_temp_home(|| {
            persist_choice(None, true).unwrap();
            let s = resolve_consent_preferring_profile(Some("dev"));
            // dev profile has no config, fall through to default.
            assert_eq!(s.source, ConsentSource::ConfigEnabled);
            assert!(s.enabled);
        });
    }

    #[test]
    fn preferring_profile_env_var_beats_profile_config() {
        with_temp_home(|| {
            persist_choice(Some("dev"), true).unwrap();
            // SAFETY: serialized via test_lock.
            unsafe {
                std::env::set_var("NYXID_TELEMETRY", "off");
            }
            let s = resolve_consent_preferring_profile(Some("dev"));
            assert_eq!(s.source, ConsentSource::EnvVarOff);
            assert!(!s.enabled);
        });
    }

    #[test]
    fn preferring_profile_do_not_track_beats_profile_config() {
        with_temp_home(|| {
            persist_choice(Some("dev"), true).unwrap();
            // SAFETY: serialized via test_lock.
            unsafe {
                std::env::set_var("DO_NOT_TRACK", "1");
            }
            let s = resolve_consent_preferring_profile(Some("dev"));
            assert_eq!(s.source, ConsentSource::DoNotTrack);
            assert!(!s.enabled);
        });
    }

    #[test]
    fn preferring_profile_ignores_garbage_telemetry_env_and_honors_profile_optout() {
        // A stray `NYXID_TELEMETRY=maybe` in the environment must not
        // bypass the per-profile consent check. Otherwise a user who
        // opted OUT on `--profile dev` in a prior release could have
        // their explicit choice silently overridden by an unrelated
        // env export.
        with_temp_home(|| {
            persist_choice(Some("dev"), false).unwrap();
            persist_choice(None, true).unwrap();
            // SAFETY: serialized via test_lock.
            unsafe {
                std::env::set_var("NYXID_TELEMETRY", "maybe");
            }
            let s = resolve_consent_preferring_profile(Some("dev"));
            assert_eq!(s.source, ConsentSource::ConfigDeclined);
            assert!(!s.enabled);
        });
    }
}
