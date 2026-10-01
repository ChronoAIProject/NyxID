use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ComputerMode;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub shell: bool,
    pub files: bool,
    pub computer: bool,
    pub roots: Vec<PathBuf>,
    pub computer_mode: ComputerMode,
    pub max_jobs: usize,
    pub max_timeout_secs: u64,
    pub output_bytes: usize,
    pub allow_root: bool,
    pub cua_driver: Option<PathBuf>,
    pub agent_user: Option<String>,
    pub browser_user: Option<String>,
    pub managed_browser: Option<ManagedBrowserConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: false,
            files: false,
            computer: false,
            roots: Vec::new(),
            computer_mode: ComputerMode::Standard,
            max_jobs: 4,
            max_timeout_secs: 3600,
            output_bytes: 1024 * 1024,
            allow_root: false,
            cua_driver: None,
            agent_user: None,
            browser_user: None,
            managed_browser: None,
        }
    }
}

impl Config {
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManagedBrowserConfig {
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    pub update_port: u16,
    /// Docker's default seccomp policy prevents Chromium's namespace sandbox.
    /// The machine image uses separate users and the container boundary.
    #[serde(default)]
    pub container: bool,
}
