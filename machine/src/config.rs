use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ComputerMode;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub shell: bool,
    pub files: bool,
    pub computer: bool,
    /// Absent preserves the pre-v2 computer ceiling. Explicit false always wins.
    pub browser: Option<bool>,
    pub roots: Vec<PathBuf>,
    pub computer_mode: ComputerMode,
    pub max_jobs: usize,
    pub max_timeout_secs: u64,
    pub output_bytes: usize,
    pub allow_root: bool,
    pub cua_driver: Option<PathBuf>,
    pub agent_user: Option<String>,
    pub browser_user: Option<String>,
    pub dev_browser_user: Option<String>,
    pub managed_browser: Option<ManagedBrowserConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: false,
            files: false,
            computer: false,
            browser: None,
            roots: Vec::new(),
            computer_mode: ComputerMode::Standard,
            max_jobs: 4,
            max_timeout_secs: 3600,
            output_bytes: 1024 * 1024,
            allow_root: false,
            cua_driver: None,
            agent_user: None,
            browser_user: None,
            dev_browser_user: None,
            managed_browser: None,
        }
    }
}

impl Config {
    pub fn browser_enabled(&self) -> bool {
        self.browser.unwrap_or(self.computer)
    }
    /// Existing container volumes predate the developer-browser setting. The
    /// official image always provisions this separate identity on upgrade.
    pub fn effective_dev_browser_user(&self) -> Option<&str> {
        self.dev_browser_user.as_deref().or_else(|| {
            self.managed_browser
                .as_ref()
                .filter(|browser| browser.container)
                .map(|_| "devbrowser")
        })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_jobs == 0 || self.max_jobs > 64 {
            return Err("max_jobs must be between 1 and 64");
        }
        if self.max_timeout_secs == 0 || self.max_timeout_secs > 86400 {
            return Err("max_timeout_secs must be between 1 and 86400");
        }
        if !(16 * 1024..=16 * 1024 * 1024).contains(&self.output_bytes) {
            return Err("output_bytes must be between 16 KiB and 16 MiB");
        }
        if (self.shell || self.files) && self.roots.is_empty() {
            return Err("machine access requires at least one workspace root");
        }
        if self.roots.len() > 16 {
            return Err("at most 16 workspace roots are supported");
        }
        if self.roots.iter().any(|root| !root.is_absolute()) {
            return Err("workspace roots must be absolute paths");
        }
        if self.agent_user.is_some() != self.browser_user.is_some() {
            return Err("both isolated OS users must be configured together");
        }
        if self.agent_user.is_some() && self.agent_user == self.browser_user {
            return Err("the browser and agent OS users must differ");
        }
        if let Some(dev) = self.effective_dev_browser_user()
            && (self.agent_user.as_deref() == Some(dev)
                || self.browser_user.as_deref() == Some(dev))
        {
            return Err("the developer browser needs its own OS user on separated installs");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_and_partial_configs_never_enable_capabilities() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(!config.shell && !config.files && !config.computer && !config.allow_root);
        assert_eq!(config.computer_mode, ComputerMode::Standard);
        assert_eq!(config.max_jobs, 4);
        assert_eq!(config.max_timeout_secs, 3600);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn enabled_configs_require_bounded_resources_and_real_roots() {
        let mut config = Config {
            shell: true,
            ..Default::default()
        };
        assert!(config.validate().is_err());
        config.roots.push(PathBuf::from("/workspace"));
        assert!(config.validate().is_ok());
        config.max_jobs = usize::MAX;
        assert!(config.validate().is_err());
    }

    #[test]
    fn existing_container_volumes_get_the_separate_developer_identity() {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "agent_user":"agent", "browser_user":"browser",
            "managed_browser":{
                "binary":"/usr/bin/chromium", "data_dir":"/desktop",
                "update_port":32248, "container":true
            }
        }))
        .unwrap();
        assert_eq!(config.effective_dev_browser_user(), Some("devbrowser"));
        assert!(config.validate().is_ok());
        config.dev_browser_user = Some("browser".into());
        assert!(config.validate().is_err());
        assert_eq!(Config::default().effective_dev_browser_user(), None);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManagedBrowserConfig {
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    pub update_port: u16,
    /// The official image provisions separate browser/agent/developer users and
    /// uses the published seccomp profile for Chromium's namespace sandbox.
    #[serde(default)]
    pub container: bool,
}
