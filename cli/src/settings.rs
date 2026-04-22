use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

const CONFIG_FILE_NAME: &str = "config.toml";
const UPDATE_CACHE_FILE_NAME: &str = "update-cache.json";
const UPDATE_CACHE_TTL_HOURS: i64 = 24;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CliConfig {
    #[serde(default = "default_update_check")]
    pub update_check: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self { update_check: true }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateCheckCache {
    pub base_url: String,
    pub checked_at: DateTime<Utc>,
    pub latest_version: String,
}

pub fn load_cli_config(profile: Option<&str>) -> Result<CliConfig> {
    let path = config_path(profile)?;
    if !path.exists() {
        return Ok(CliConfig::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

pub fn save_cli_config(profile: Option<&str>, config: &CliConfig) -> Result<()> {
    let path = config_path(profile)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let serialized =
        toml::to_string_pretty(config).context("Failed to serialize CLI configuration")?;
    std::fs::write(&path, serialized).with_context(|| format!("Failed to write {}", path.display()))
}

pub fn set_update_check(profile: Option<&str>, enabled: bool) -> Result<()> {
    let mut config = load_cli_config(profile)?;
    config.update_check = enabled;
    save_cli_config(profile, &config)
}

pub fn effective_update_check(profile: Option<&str>) -> Result<bool> {
    if let Some(explicit) =
        parse_update_check_env_override(std::env::var("NYXID_UPDATE_CHECK").ok().as_deref())
    {
        return Ok(explicit);
    }

    if std::env::var("CI")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return Ok(false);
    }

    Ok(load_cli_config(profile)?.update_check)
}

pub fn read_update_cache(profile: Option<&str>) -> Result<Option<UpdateCheckCache>> {
    let path = update_cache_path(profile)?;
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let cache = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(Some(cache))
}

pub fn write_update_cache(profile: Option<&str>, cache: &UpdateCheckCache) -> Result<()> {
    let path = update_cache_path(profile)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let content = serde_json::to_vec_pretty(cache).context("Failed to serialize update cache")?;
    std::fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))
}

pub fn should_refresh_update_cache(
    cache: Option<&UpdateCheckCache>,
    base_url: &str,
    now: DateTime<Utc>,
) -> bool {
    let Some(cache) = cache else {
        return true;
    };

    if cache.base_url != base_url {
        return true;
    }

    now.signed_duration_since(cache.checked_at) >= Duration::hours(UPDATE_CACHE_TTL_HOURS)
}

pub fn config_path(profile: Option<&str>) -> Result<PathBuf> {
    Ok(crate::auth::state_dir_for_profile(profile)?.join(CONFIG_FILE_NAME))
}

pub fn update_cache_path(profile: Option<&str>) -> Result<PathBuf> {
    Ok(crate::auth::state_dir_for_profile(profile)?.join(UPDATE_CACHE_FILE_NAME))
}

fn default_update_check() -> bool {
    true
}

fn parse_update_check_env_override(value: Option<&str>) -> Option<bool> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value)
            if value == "0"
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("off")
                || value.eq_ignore_ascii_case("no") =>
        {
            Some(false)
        }
        Some(value)
            if value == "1"
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("on")
                || value.eq_ignore_ascii_case("yes") =>
        {
            Some(true)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};

    use super::{UpdateCheckCache, parse_update_check_env_override, should_refresh_update_cache};

    #[test]
    fn parses_update_check_env_override() {
        assert_eq!(parse_update_check_env_override(Some("0")), Some(false));
        assert_eq!(parse_update_check_env_override(Some("off")), Some(false));
        assert_eq!(parse_update_check_env_override(Some("1")), Some(true));
        assert_eq!(parse_update_check_env_override(Some("true")), Some(true));
        assert_eq!(parse_update_check_env_override(Some("maybe")), None);
        assert_eq!(parse_update_check_env_override(None), None);
    }

    #[test]
    fn refreshes_cache_when_stale_or_base_url_changes() {
        let now = Utc::now();
        let fresh = UpdateCheckCache {
            base_url: "https://auth.example.com".to_string(),
            checked_at: now - TimeDelta::hours(2),
            latest_version: "0.3.0".to_string(),
        };
        let stale = UpdateCheckCache {
            checked_at: now - TimeDelta::hours(25),
            ..fresh.clone()
        };

        assert!(!should_refresh_update_cache(
            Some(&fresh),
            "https://auth.example.com",
            now,
        ));
        assert!(should_refresh_update_cache(
            Some(&fresh),
            "https://other.example.com",
            now,
        ));
        assert!(should_refresh_update_cache(
            Some(&stale),
            "https://auth.example.com",
            now,
        ));
        assert!(should_refresh_update_cache(
            None,
            "https://auth.example.com",
            now
        ));
    }
}
