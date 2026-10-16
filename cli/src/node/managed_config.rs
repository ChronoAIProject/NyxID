//! Pure managed capability/resource validation shared by normal and test runtimes.
use super::config::NodeConfig;
use anyhow::{Context, Result, ensure};
use reqwest::Url;
use std::path::PathBuf;

pub(super) fn default_capabilities() -> Vec<String> {
    vec!["files".into(), "shell".into()]
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
    let desktop = names.contains("browser");
    let config = nyxid_machine::config::Config {
        shell: names.contains("shell"),
        files: names.contains("files"),
        browser: Some(desktop),
        computer: names.contains("computer"),
        roots: vec![PathBuf::from("/workspace")],
        max_jobs: 2,
        agent_user: Some("agent".into()),
        browser_user: Some("browser".into()),
        dev_browser_user: desktop.then(|| "devbrowser".into()),
        computer_mode: if desktop {
            nyxid_machine::ComputerMode::Unrestricted
        } else {
            nyxid_machine::ComputerMode::Standard
        },
        cua_driver: desktop.then(|| PathBuf::from("/opt/nyxid/cua/cua-driver")),
        managed_browser: desktop.then(|| nyxid_machine::config::ManagedBrowserConfig {
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

pub(super) fn validate_existing_config(
    config: &NodeConfig,
    binding: &nyxid_machine::managed::Binding,
) -> Result<()> {
    binding.validate().map_err(anyhow::Error::msg)?;
    validate_endpoint(&config.server.url, true)?;
    ensure!(
        config.node.id == binding.node_id && config.managed_runtime.as_ref() == Some(binding),
        "existing managed runtime does not match provider ticket"
    );
    let capabilities = config
        .managed_capabilities
        .clone()
        .unwrap_or_else(default_capabilities);
    let expected = machine_config(&capabilities)?;
    ensure!(
        serde_json::to_value(&config.machine)? == serde_json::to_value(expected)?
            && config.storage_backend == "file",
        "existing managed runtime violates its sealed capabilities or fixed resources"
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
