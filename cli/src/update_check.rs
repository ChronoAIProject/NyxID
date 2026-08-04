use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use is_terminal::IsTerminal;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use crate::cli::Commands;
use crate::commands::update;

const CACHE_FILE_NAME: &str = "update-check.json";
const CHECK_INTERVAL_HOURS: i64 = 24;
const NETWORK_TIMEOUT_MS: u64 = 1500;
const DISABLE_ENV: &str = "NYXID_NO_UPDATE_CHECK";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct UpdateCheckCache {
    pub(crate) last_checked: DateTime<Utc>,
    pub(crate) latest_known: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateNotice {
    pub(crate) latest: String,
    pub(crate) installed: String,
}

pub(crate) enum PendingUpdateNotice {
    Ready(Option<UpdateNotice>),
    Task(JoinHandle<Option<UpdateNotice>>),
}

pub(crate) fn start_update_notice(command: &Commands) -> Option<PendingUpdateNotice> {
    if !should_attempt_update_check(command, std::io::stdout().is_terminal()) {
        return None;
    }

    let installed = installed_release_tag();
    if let Some(cache) = read_cache()
        && cache_is_fresh(&cache, Utc::now())
    {
        return Some(PendingUpdateNotice::Ready(notice_from_latest(
            &cache.latest_known,
            &installed,
        )));
    }

    Some(PendingUpdateNotice::Task(tokio::spawn(async move {
        fetch_notice_with_timeout(installed).await
    })))
}

pub(crate) async fn maybe_print_update_notice(pending: Option<PendingUpdateNotice>) {
    let Some(pending) = pending else {
        return;
    };

    let notice = match pending {
        PendingUpdateNotice::Ready(notice) => notice,
        PendingUpdateNotice::Task(handle) => {
            if !handle.is_finished() {
                handle.abort();
                return;
            }
            handle.await.ok().flatten()
        }
    };

    if let Some(notice) = notice {
        eprintln!(
            "A newer release of nyxid ({}) is available. Your version: {}.",
            notice.latest, notice.installed
        );
        eprintln!("Run `nyxid update` to install. (Set {DISABLE_ENV}=1 to disable this check.)");
    }
}

async fn fetch_notice_with_timeout(installed: String) -> Option<UpdateNotice> {
    let previous = read_cache();
    let result = tokio::time::timeout(
        Duration::from_millis(NETWORK_TIMEOUT_MS),
        fetch_latest_release_tag(),
    )
    .await;

    match result {
        Ok(Ok(latest)) => {
            let _ = write_cache(&UpdateCheckCache {
                last_checked: Utc::now(),
                latest_known: latest.clone(),
            });
            notice_from_latest(&latest, &installed)
        }
        _ => {
            let latest_known = previous
                .map(|cache| cache.latest_known)
                .unwrap_or_else(|| installed.clone());
            let _ = write_cache(&UpdateCheckCache {
                last_checked: Utc::now(),
                latest_known,
            });
            None
        }
    }
}

async fn fetch_latest_release_tag() -> Result<String> {
    let client = update::github_client()?;
    let release = update::resolve_release(&client, None)
        .await?
        .context("No NyxID GitHub releases were found yet")?;
    Ok(release.tag_name)
}

pub(crate) fn should_attempt_update_check(command: &Commands, stdout_is_tty: bool) -> bool {
    !matches!(command, Commands::Update(_))
        && stdout_is_tty
        && !ci_is_enabled()
        && !env_opt_out_is_enabled(std::env::var(DISABLE_ENV).ok().as_deref())
}

fn ci_is_enabled() -> bool {
    std::env::var("CI")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

pub(crate) fn env_opt_out_is_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub(crate) fn cache_is_fresh(cache: &UpdateCheckCache, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(cache.last_checked).num_hours() < CHECK_INTERVAL_HOURS
}

pub(crate) fn notice_from_latest(latest: &str, installed: &str) -> Option<UpdateNotice> {
    match update::compare_release_tags(latest, installed).ok()? {
        Ordering::Greater => Some(UpdateNotice {
            latest: update::normalize_release_tag(latest).ok()?,
            installed: update::normalize_release_tag(installed).ok()?,
        }),
        Ordering::Equal | Ordering::Less => None,
    }
}

pub(crate) fn read_cache() -> Option<UpdateCheckCache> {
    let path = cache_path()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn write_cache(cache: &UpdateCheckCache) -> Result<()> {
    let path = cache_path().context("Could not determine update-check cache path")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(cache)?)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub(crate) fn cache_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".nyxid").join(CACHE_FILE_NAME))
}

pub(crate) fn installed_release_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn cache_freshness_uses_twenty_four_hour_window() {
        let now = Utc::now();
        let fresh = UpdateCheckCache {
            last_checked: now - chrono::Duration::hours(23),
            latest_known: "v0.5.0".to_string(),
        };
        let stale = UpdateCheckCache {
            last_checked: now - chrono::Duration::hours(25),
            latest_known: "v0.5.0".to_string(),
        };

        assert!(cache_is_fresh(&fresh, now));
        assert!(!cache_is_fresh(&stale, now));
    }

    #[test]
    fn notice_only_when_latest_is_newer() {
        assert_eq!(
            notice_from_latest("v0.5.0", "v0.4.0").unwrap(),
            UpdateNotice {
                latest: "v0.5.0".to_string(),
                installed: "v0.4.0".to_string()
            }
        );
        assert!(notice_from_latest("v0.4.0", "v0.4.0").is_none());
        assert!(notice_from_latest("v0.4.0-beta.1", "v0.4.0").is_none());
    }

    #[test]
    fn env_var_opt_out_accepts_truthy_values() {
        assert!(env_opt_out_is_enabled(Some("1")));
        assert!(env_opt_out_is_enabled(Some("true")));
        assert!(env_opt_out_is_enabled(Some("yes")));
        assert!(!env_opt_out_is_enabled(Some("0")));
        assert!(!env_opt_out_is_enabled(None));
    }

    #[test]
    fn update_command_is_suppressed() {
        let cli = Cli::parse_from(["nyxid", "update", "--check"]);
        assert!(matches!(cli.command, Commands::Update(_)));
        assert!(!should_attempt_update_check(&cli.command, true));
    }

    #[test]
    fn should_attempt_rejects_non_tty() {
        let cli = Cli::parse_from(["nyxid", "whoami"]);
        assert!(!should_attempt_update_check(&cli.command, false));
    }

    #[test]
    fn ci_is_enabled_respects_env() {
        let _guard = crate::test_support::env_lock().lock().unwrap();
        unsafe { std::env::set_var("CI", "true") };
        assert!(ci_is_enabled());
        unsafe { std::env::set_var("CI", "false") };
        assert!(!ci_is_enabled());
        unsafe { std::env::remove_var("CI") };
        assert!(!ci_is_enabled());
    }

    #[test]
    fn cache_path_under_home_nyxid() {
        let path = cache_path().unwrap();
        assert!(path.ends_with(".nyxid/update-check.json"));
    }
}
