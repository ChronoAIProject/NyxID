//! Pure managed capability/resource validation shared by normal and test runtimes.
use super::config::NodeConfig;
use anyhow::{Context, Result, ensure};
use reqwest::Url;
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    #[test]
    fn shell_files_prepare_full_infrastructure_without_enabling_graphical_tools() {
        let config = super::machine_config(&["shell".into(), "files".into()]).unwrap();
        assert!(
            config.managed_browser.is_some(),
            "separation needs managed browser infrastructure"
        );
        assert_eq!(config.dev_browser_user.as_deref(), Some("devbrowser"));
        assert!(config.cua_driver.is_some());
        assert!(config.shell && config.files);
        assert!(!config.browser_enabled() && !config.computer);
    }
}

pub(super) fn machine_config(capabilities: &[String]) -> Result<nyxid_machine::config::Config> {
    ensure!(
        !capabilities.is_empty() && capabilities.len() <= 4,
        "invalid managed capabilities"
    );
    let mut names = std::collections::BTreeSet::new();
    for capability in capabilities {
        ensure!(
            matches!(
                capability.as_str(),
                "shell" | "files" | "browser" | "computer"
            ) && names.insert(capability.as_str()),
            "invalid managed capabilities"
        );
    }
    ensure!(
        !names.contains("computer") || names.contains("browser"),
        "managed computer capability requires browser"
    );
    // Hosted infrastructure is present even for Shell/File-only ceilings.
    // Preparation never enables a Browser/Computer API permission.
    let config = nyxid_machine::config::Config {
        shell: names.contains("shell"),
        files: names.contains("files"),
        browser: Some(names.contains("browser")),
        computer: names.contains("computer"),
        roots: vec![PathBuf::from("/workspace")],
        max_jobs: 2,
        agent_user: Some("agent".into()),
        browser_user: Some("browser".into()),
        dev_browser_user: Some("devbrowser".into()),
        computer_mode: nyxid_machine::ComputerMode::Unrestricted,
        cua_driver: Some(PathBuf::from("/opt/nyxid/cua/cua-driver")),
        managed_browser: Some(nyxid_machine::config::ManagedBrowserConfig {
            binary: PathBuf::from("/usr/bin/chromium"),
            data_dir: PathBuf::from("/var/lib/nyxid-machine/desktop"),
            update_port: 32248,
            container: true,
        }),
        ..Default::default()
    };
    config.validate().map_err(anyhow::Error::msg)?;
    Ok(config)
}

/// Derive infrastructure and API ceilings from the one shared semantic contract.
pub(crate) fn configuration_machine(
    configuration: &nyxid_machine::managed::RequiredConfiguration,
) -> Result<nyxid_machine::config::Config> {
    configuration.validate().map_err(anyhow::Error::msg)?;
    let mut machine = machine_config(&configuration.capabilities)?;
    machine.separation_choice = Some(configuration.separation_choice);
    Ok(machine)
}

pub(crate) fn validate_configuration_report(
    machine: &nyxid_machine::config::Config,
    binding: &nyxid_machine::managed::Binding,
    report: &nyxid_machine::managed::ConfigurationReport,
) -> Result<()> {
    binding.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        report.runtime_generation == binding.runtime_generation
            && report.configuration.digest().map_err(anyhow::Error::msg)? == report.digest,
        "managed configuration identity mismatch"
    );
    ensure!(
        serde_json::to_value(machine)?
            == serde_json::to_value(configuration_machine(&report.configuration)?)?,
        "actual managed runtime violates its sealed configuration"
    );
    Ok(())
}

pub(super) fn validate_existing_config(
    config: &NodeConfig,
    binding: &nyxid_machine::managed::Binding,
) -> Result<()> {
    binding.validate().map_err(anyhow::Error::msg)?;
    validate_endpoint(&config.server.url, true)?;
    config
        .hosted_storage
        .as_ref()
        .context("managed retained storage authority is required")?
        .validate()
        .map_err(anyhow::Error::msg)?;
    ensure!(
        config.node.id == binding.node_id && config.managed_runtime.as_ref() == Some(binding),
        "existing managed runtime does not match provider ticket"
    );
    let report = config
        .managed_configuration
        .as_ref()
        .context("managed semantic configuration is required")?;
    validate_configuration_report(&config.machine, binding, report)?;
    ensure!(
        config.storage_backend == "file",
        "managed runtime requires protected file storage"
    );
    config.machine.validate().map_err(anyhow::Error::msg)
}

pub(super) fn validate_endpoint(value: &str, websocket: bool) -> Result<Url> {
    let url = Url::parse(value).context("invalid managed endpoint URL")?;
    let secure_scheme = if websocket { "wss" } else { "https" };
    let local_scheme = if websocket { "ws" } else { "http" };
    let local = url
        .host_str()
        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"));
    ensure!(
        url.scheme() == secure_scheme || (url.scheme() == local_scheme && local),
        "managed endpoint requires encrypted transport outside localhost"
    );
    ensure!(
        url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "managed endpoint must not contain credentials, query, or fragment"
    );
    if websocket {
        ensure!(
            url.path().ends_with("/api/v1/nodes/ws"),
            "managed endpoint must use the Node WebSocket route"
        );
    }
    Ok(url)
}
