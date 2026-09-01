use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const LAUNCHD_LABEL_BASE: &str = "dev.nyxid.oracle";
const SYSTEMD_UNIT_BASE: &str = "nyxid-oracle";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OracleWorkerConfig {
    pub pool: String,
    pub label: String,
    pub base_url: String,
    pub node_binary: PathBuf,
    pub bundle_path: PathBuf,
    pub token_file: PathBuf,
    pub state_file: PathBuf,
    pub installation_id_file: PathBuf,
    pub chrome_executable: PathBuf,
    pub chrome_profile_dir: PathBuf,
    pub chrome_debug_port: u16,
}

pub fn install_dir(pool: &str, profile: Option<&str>) -> Result<PathBuf> {
    validate_component(pool)?;
    let root = dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".nyxid-oracle")
        .join(pool);
    match profile {
        None | Some("default") => Ok(root),
        Some(value) => {
            validate_component(value)?;
            Ok(root.join("profiles").join(value))
        }
    }
}

pub fn config_path(pool: &str, profile: Option<&str>) -> Result<PathBuf> {
    Ok(install_dir(pool, profile)?.join("config.toml"))
}

pub fn load_config(pool: &str, profile: Option<&str>) -> Result<OracleWorkerConfig> {
    let path = config_path(pool, profile)?;
    let body = fs::read_to_string(&path)
        .with_context(|| format!("Worker is not installed at {}", path.display()))?;
    toml::from_str(&body).with_context(|| format!("Invalid worker config at {}", path.display()))
}

pub fn save_config(config: &OracleWorkerConfig, profile: Option<&str>) -> Result<PathBuf> {
    let path = config_path(&config.pool, profile)?;
    let parent = path.parent().context("Invalid worker config path")?;
    fs::create_dir_all(parent)?;
    fs::write(&path, toml::to_string_pretty(config)?)?;
    Ok(path)
}

pub fn install_service(
    config: &OracleWorkerConfig,
    profile: Option<&str>,
    force: bool,
) -> Result<()> {
    if cfg!(target_os = "macos") {
        install_launchd(config, profile, force)
    } else if cfg!(target_os = "linux") {
        install_systemd(config, profile, force)
    } else {
        bail!("Oracle worker daemons are supported only on macOS and Linux")
    }
}

pub fn start(pool: &str, profile: Option<&str>) -> Result<()> {
    load_config(pool, profile)?;
    if cfg!(target_os = "macos") {
        start_launchd(pool, profile)
    } else if cfg!(target_os = "linux") {
        systemctl_action(pool, profile, "start", "started")
    } else {
        bail!("Oracle worker daemons are supported only on macOS and Linux")
    }
}

pub fn stop(pool: &str, profile: Option<&str>) -> Result<()> {
    if cfg!(target_os = "macos") {
        stop_launchd(pool, profile)
    } else if cfg!(target_os = "linux") {
        systemctl_action(pool, profile, "stop", "stopped")
    } else {
        bail!("Oracle worker daemons are supported only on macOS and Linux")
    }
}

pub fn status(pool: &str, profile: Option<&str>) -> Result<()> {
    let path = config_path(pool, profile)?;
    println!("NyxID Oracle Worker Service Status");
    println!("==================================");
    println!("Config:     {}", path.display());
    if !path.exists() {
        println!("Installed:  no");
        return Ok(());
    }
    if cfg!(target_os = "macos") {
        status_launchd(pool, profile)
    } else if cfg!(target_os = "linux") {
        status_systemd(pool, profile)
    } else {
        println!("Service:    unsupported platform");
        Ok(())
    }
}

pub fn logs(pool: &str, profile: Option<&str>, follow: bool, lines: usize) -> Result<()> {
    if cfg!(target_os = "macos") {
        let dir = install_dir(pool, profile)?.join("logs");
        let stdout = dir.join("worker.log");
        let stderr = dir.join("worker.err.log");
        let mut command = Command::new("tail");
        if follow {
            command.arg("-f");
        } else {
            command.args(["-n", &lines.to_string()]);
        }
        if stdout.exists() {
            command.arg(stdout);
        }
        if stderr.exists() {
            command.arg(stderr);
        }
        let status = command.status()?;
        if !status.success() {
            bail!("tail failed")
        }
        Ok(())
    } else if cfg!(target_os = "linux") {
        let unit = systemd_unit(pool, profile)?;
        let mut command = Command::new("journalctl");
        command.args([
            "--user",
            "-u",
            &unit,
            "-n",
            &lines.to_string(),
            "--no-pager",
        ]);
        if follow {
            command.arg("-f");
        }
        let status = command.status()?;
        if !status.success() {
            bail!("journalctl failed")
        }
        Ok(())
    } else {
        bail!("Oracle worker daemons are supported only on macOS and Linux")
    }
}

pub fn uninstall(pool: &str, profile: Option<&str>) -> Result<()> {
    let _ = stop(pool, profile);
    if cfg!(target_os = "macos") {
        let path = plist_path(pool, profile)?;
        if path.exists() {
            fs::remove_file(&path)?;
            println!("Removed {}", path.display());
        }
    } else if cfg!(target_os = "linux") {
        let unit = systemd_unit(pool, profile)?;
        let _ = Command::new("systemctl")
            .args(["--user", "disable", &unit])
            .output();
        let path = unit_path(pool, profile)?;
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        println!("Removed {}", path.display());
    } else {
        bail!("Oracle worker daemons are supported only on macOS and Linux")
    }
    println!(
        "Worker files and token were retained under {}",
        install_dir(pool, profile)?.display()
    );
    Ok(())
}

fn service_suffix(pool: &str, profile: Option<&str>) -> Result<String> {
    validate_component(pool)?;
    match profile {
        None | Some("default") => Ok(pool.to_string()),
        Some(value) => {
            validate_component(value)?;
            Ok(format!("{pool}.{value}"))
        }
    }
}

fn launchd_label(pool: &str, profile: Option<&str>) -> Result<String> {
    Ok(format!(
        "{LAUNCHD_LABEL_BASE}.{}",
        service_suffix(pool, profile)?
    ))
}

fn systemd_unit(pool: &str, profile: Option<&str>) -> Result<String> {
    Ok(format!(
        "{SYSTEMD_UNIT_BASE}-{}.service",
        service_suffix(pool, profile)?.replace('.', "-")
    ))
}

fn validate_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("'{value}' is not safe for a service or installation name")
    }
    Ok(())
}

fn plist_path(pool: &str, profile: Option<&str>) -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", launchd_label(pool, profile)?)))
}

fn launchd_domain() -> String {
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}")
}

fn launchd_target(pool: &str, profile: Option<&str>) -> Result<String> {
    Ok(format!(
        "{}/{}",
        launchd_domain(),
        launchd_label(pool, profile)?
    ))
}

fn install_launchd(config: &OracleWorkerConfig, profile: Option<&str>, force: bool) -> Result<()> {
    let path = plist_path(&config.pool, profile)?;
    if path.exists() && !force {
        bail!(
            "Service already installed at {}; use --force",
            path.display()
        )
    }
    if force {
        let _ = launchd_bootout(&config.pool, profile, true);
    }
    let log_dir = install_dir(&config.pool, profile)?.join("logs");
    fs::create_dir_all(&log_dir)?;
    let args = [
        config.node_binary.display().to_string(),
        config.bundle_path.display().to_string(),
    ];
    let args_xml = args
        .iter()
        .map(|value| format!("        <string>{}</string>", xml_escape(value)))
        .collect::<Vec<_>>()
        .join("\n");
    let env_xml = worker_environment(config)
        .iter()
        .map(|(key, value)| {
            format!(
                "        <key>{}</key>\n        <string>{}</string>",
                xml_escape(key),
                xml_escape(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>ProgramArguments</key><array>
{args_xml}
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>5</integer>
<key>StandardOutPath</key><string>{stdout}</string>
<key>StandardErrorPath</key><string>{stderr}</string>
<key>EnvironmentVariables</key><dict>
{env_xml}
</dict>
</dict></plist>
"#,
        label = launchd_label(&config.pool, profile)?,
        stdout = xml_escape(&log_dir.join("worker.log").display().to_string()),
        stderr = xml_escape(&log_dir.join("worker.err.log").display().to_string()),
    );
    fs::create_dir_all(path.parent().context("Invalid LaunchAgent path")?)?;
    fs::write(&path, body)?;
    println!("Service installed at {}", path.display());
    Ok(())
}

fn launchd_bootout(pool: &str, profile: Option<&str>, ignore_missing: bool) -> Result<()> {
    let output = Command::new("launchctl")
        .args(["bootout", &launchd_target(pool, profile)?])
        .output()?;
    if output.status.success()
        || (ignore_missing
            && (output.status.code() == Some(3)
                || String::from_utf8_lossy(&output.stderr).contains("not found")))
    {
        return Ok(());
    }
    bail!(
        "launchctl bootout failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

fn start_launchd(pool: &str, profile: Option<&str>) -> Result<()> {
    let path = plist_path(pool, profile)?;
    if !path.exists() {
        bail!("Service is not installed; run `nyxid oracle worker install --pool {pool}`")
    }
    let target = launchd_target(pool, profile)?;
    if Command::new("launchctl")
        .args(["print", &target])
        .output()?
        .status
        .success()
    {
        let output = Command::new("launchctl")
            .args(["kickstart", "-k", &target])
            .output()?;
        if !output.status.success() {
            bail!(
                "launchctl kickstart failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        }
    } else {
        let output = Command::new("launchctl")
            .args(["bootstrap", &launchd_domain(), &path.display().to_string()])
            .output()?;
        if !output.status.success() {
            bail!(
                "launchctl bootstrap failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        }
    }
    println!("Oracle worker started.");
    Ok(())
}

fn stop_launchd(pool: &str, profile: Option<&str>) -> Result<()> {
    launchd_bootout(pool, profile, true)?;
    println!("Oracle worker stopped.");
    Ok(())
}

fn status_launchd(pool: &str, profile: Option<&str>) -> Result<()> {
    let path = plist_path(pool, profile)?;
    println!(
        "Installed:  {}",
        if path.exists() { "yes (launchd)" } else { "no" }
    );
    if !path.exists() {
        return Ok(());
    }
    let output = Command::new("launchctl")
        .args(["print", &launchd_target(pool, profile)?])
        .output()?;
    println!(
        "Running:    {}",
        if output.status.success() { "yes" } else { "no" }
    );
    Ok(())
}

fn unit_path(pool: &str, profile: Option<&str>) -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".config/systemd/user")
        .join(systemd_unit(pool, profile)?))
}

fn install_systemd(config: &OracleWorkerConfig, profile: Option<&str>, force: bool) -> Result<()> {
    let path = unit_path(&config.pool, profile)?;
    if path.exists() && !force {
        bail!(
            "Service already installed at {}; use --force",
            path.display()
        )
    }
    let body = render_systemd_service(config);
    fs::create_dir_all(path.parent().context("Invalid systemd unit path")?)?;
    fs::write(&path, body)?;
    let unit = systemd_unit(&config.pool, profile)?;
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();
    let output = Command::new("systemctl")
        .args(["--user", "enable", &unit])
        .output()?;
    if !output.status.success() {
        bail!(
            "systemctl enable failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    }
    if let Ok(user) = std::env::var("USER")
        && !user.is_empty()
    {
        let linger = Command::new("loginctl")
            .args(["enable-linger", &user])
            .output();
        if !matches!(linger, Ok(output) if output.status.success()) {
            eprintln!("Warning: could not enable systemd user lingering.");
            eprintln!("Run: sudo loginctl enable-linger {user}");
        }
    }
    println!("Service installed at {}", path.display());
    Ok(())
}

fn render_systemd_service(config: &OracleWorkerConfig) -> String {
    let environment = worker_environment(config)
        .iter()
        .map(|(key, value)| format!("Environment={}", systemd_quote(&format!("{key}={value}"))))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[Unit]\nDescription=NyxID Oracle Worker ({})\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={} {}\nRestart=always\nRestartSec=5\n{}\n\n[Install]\nWantedBy=default.target\n",
        config.pool,
        systemd_quote(&config.node_binary.display().to_string()),
        systemd_quote(&config.bundle_path.display().to_string()),
        environment,
    )
}

fn systemctl_action(pool: &str, profile: Option<&str>, action: &str, past: &str) -> Result<()> {
    let unit = systemd_unit(pool, profile)?;
    let output = Command::new("systemctl")
        .args(["--user", action, &unit])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if action == "stop" && (stderr.contains("not loaded") || stderr.contains("not found")) {
            println!("Oracle worker is not running.");
            return Ok(());
        }
        bail!("systemctl {action} failed: {}", stderr)
    }
    println!("Oracle worker {past}.");
    Ok(())
}

fn status_systemd(pool: &str, profile: Option<&str>) -> Result<()> {
    let path = unit_path(pool, profile)?;
    println!(
        "Installed:  {}",
        if path.exists() { "yes (systemd)" } else { "no" }
    );
    if !path.exists() {
        return Ok(());
    }
    let output = Command::new("systemctl")
        .args(["--user", "is-active", &systemd_unit(pool, profile)?])
        .output()?;
    println!(
        "Running:    {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
    Ok(())
}

fn worker_environment(config: &OracleWorkerConfig) -> Vec<(String, String)> {
    vec![
        ("NYXID_BASE_URL".into(), config.base_url.clone()),
        (
            "NYXID_WORKER_TOKEN_FILE".into(),
            config.token_file.display().to_string(),
        ),
        ("NYXID_WORKER_LABEL".into(), config.label.clone()),
        (
            "NYXID_WORKER_STATE_FILE".into(),
            config.state_file.display().to_string(),
        ),
        (
            "NYXID_INSTALLATION_ID_FILE".into(),
            config.installation_id_file.display().to_string(),
        ),
        (
            "CHROME_CDP_URL".into(),
            format!("http://127.0.0.1:{}", config.chrome_debug_port),
        ),
        (
            "CHROME_DEBUG_PORT".into(),
            config.chrome_debug_port.to_string(),
        ),
        (
            "CHROME_PROFILE_DIR".into(),
            config.chrome_profile_dir.display().to_string(),
        ),
        (
            "NYXID_CHROME_EXECUTABLE".into(),
            config.chrome_executable.display().to_string(),
        ),
    ]
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn systemd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_names_are_profile_scoped_and_safe() {
        assert_eq!(
            launchd_label("pool-1", Some("dev")).unwrap(),
            "dev.nyxid.oracle.pool-1.dev"
        );
        assert_eq!(
            systemd_unit("pool-1", Some("dev")).unwrap(),
            "nyxid-oracle-pool-1-dev.service"
        );
        assert!(launchd_label("../../bad", None).is_err());
    }

    #[test]
    fn service_formats_escape_untrusted_paths() {
        assert_eq!(xml_escape("a&<b>"), "a&amp;&lt;b&gt;");
        assert_eq!(systemd_quote("/tmp/a b"), "\"/tmp/a b\"");
        assert_eq!(systemd_quote("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn systemd_service_keeps_arguments_and_token_out_of_the_unit() {
        let config = OracleWorkerConfig {
            pool: "pool-1".to_string(),
            label: "worker-1".to_string(),
            base_url: "https://nyxid.example".to_string(),
            node_binary: PathBuf::from("/opt/Node JS/bin/node"),
            bundle_path: PathBuf::from("/home/user/My Worker/worker.mjs"),
            token_file: PathBuf::from("/home/user/private/worker-token"),
            state_file: PathBuf::from("/home/user/private/state.json"),
            installation_id_file: PathBuf::from("/home/user/private/installation-id"),
            chrome_executable: PathBuf::from("/opt/Google Chrome/chrome"),
            chrome_profile_dir: PathBuf::from("/home/user/Chrome Profile"),
            chrome_debug_port: 9227,
        };

        let unit = render_systemd_service(&config);
        assert!(
            unit.contains(
                "ExecStart=\"/opt/Node JS/bin/node\" \"/home/user/My Worker/worker.mjs\""
            )
        );
        assert!(
            unit.contains(
                "Environment=\"NYXID_WORKER_TOKEN_FILE=/home/user/private/worker-token\""
            )
        );
        assert!(unit.contains("Environment=\"CHROME_CDP_URL=http://127.0.0.1:9227\""));
        assert!(!unit.contains("nyx_owk_"));
    }
}
