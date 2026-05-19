//! Daemon lifecycle management for the NyxID node agent.
//!
//! Installs, starts, stops, restarts, and queries the node agent as a
//! platform-native background service:
//!
//! - **macOS**: LaunchAgent plist (`~/Library/LaunchAgents/dev.nyxid.node.plist`)
//! - **Linux**: systemd user unit (`~/.config/systemd/user/nyxid-node.service`)

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::config;
use super::error::{Error, Result};

// ---------------------------------------------------------------------------
// Service identity
// ---------------------------------------------------------------------------

const LAUNCHD_LABEL_BASE: &str = "dev.nyxid.node";
const SYSTEMD_UNIT_BASE: &str = "nyxid-node";
const PID_FILE_NAME: &str = "nyxid-node.pid";
const LOG_DIR_NAME: &str = "logs";

const VALID_LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];
const DAEMON_META_FILE: &str = "daemon.toml";

fn launchd_label(profile: Option<&str>) -> Result<String> {
    match profile {
        None | Some("default") => Ok(LAUNCHD_LABEL_BASE.to_string()),
        Some(name) => {
            assert_safe_label(name)?;
            Ok(format!("{LAUNCHD_LABEL_BASE}.{name}"))
        }
    }
}

fn systemd_unit(profile: Option<&str>) -> Result<String> {
    match profile {
        None | Some("default") => Ok(format!("{SYSTEMD_UNIT_BASE}.service")),
        Some(name) => {
            assert_safe_label(name)?;
            Ok(format!("{SYSTEMD_UNIT_BASE}-{name}.service"))
        }
    }
}

/// Defense-in-depth: ensure a profile name is safe for use in service labels.
/// Only alphanumeric, hyphens, and underscores are allowed.
fn assert_safe_label(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::Validation(format!(
            "Profile name '{name}' is not safe for service labels"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Install the node agent as a background service (LaunchAgent / systemd unit).
///
/// If `--force` is passed, existing service files are overwritten.
pub fn install(
    config_path: Option<&str>,
    profile: Option<&str>,
    log_level: Option<&str>,
    force: bool,
) -> Result<()> {
    if let Some(level) = log_level
        && !VALID_LOG_LEVELS.contains(&level)
    {
        return Err(Error::Validation(format!(
            "Invalid log level '{level}'. Valid values: {}",
            VALID_LOG_LEVELS.join(", ")
        )));
    }

    let config_dir = config::resolve_config_dir_with_profile(config_path, profile)?;
    ensure_config_exists(&config_dir)?;
    let config_dir = canonicalize_existing_dir(&config_dir)?;

    // Persist the config dir so other daemon subcommands can find it
    save_daemon_config_dir(&config_dir, profile)?;

    let nyxid_bin = resolve_binary()?;

    if cfg!(target_os = "macos") {
        let log_dir = config_dir.join(LOG_DIR_NAME);
        fs::create_dir_all(&log_dir)?;
        install_launchd(&nyxid_bin, &log_dir, &config_dir, profile, log_level, force)
    } else if cfg!(target_os = "linux") {
        install_systemd(&nyxid_bin, &config_dir, profile, log_level, force)
    } else {
        Err(Error::Validation(
            "Daemon installation is only supported on macOS and Linux".into(),
        ))
    }
}

/// Uninstall the background service and remove service files.
pub fn uninstall(config_path: Option<&str>, profile: Option<&str>) -> Result<()> {
    // Stop first (ignore errors -- service may not be running)
    let _ = stop(config_path, profile);

    let result = if cfg!(target_os = "macos") {
        uninstall_launchd(profile)
    } else if cfg!(target_os = "linux") {
        uninstall_systemd(profile)
    } else {
        Err(Error::Validation(
            "Daemon uninstallation is only supported on macOS and Linux".into(),
        ))
    };

    // Clean up saved config dir metadata
    let default_dir = config::resolve_config_dir_with_profile(None, profile)?;
    let meta_file = default_dir.join(DAEMON_META_FILE);
    let _ = fs::remove_file(meta_file);

    result
}

/// Start the installed service.
pub fn start(config_path: Option<&str>, profile: Option<&str>) -> Result<()> {
    let config_dir = resolve_daemon_config_dir(config_path, profile)?;
    ensure_config_exists(&config_dir)?;

    if cfg!(target_os = "macos") {
        start_launchd(profile)
    } else if cfg!(target_os = "linux") {
        start_systemd(profile)
    } else {
        Err(Error::Validation(
            "Daemon start is only supported on macOS and Linux".into(),
        ))
    }
}

/// Stop the running service.
pub fn stop(config_path: Option<&str>, profile: Option<&str>) -> Result<()> {
    let _ = config_path;

    if cfg!(target_os = "macos") {
        stop_launchd(profile)
    } else if cfg!(target_os = "linux") {
        stop_systemd(profile)
    } else {
        Err(Error::Validation(
            "Daemon stop is only supported on macOS and Linux".into(),
        ))
    }
}

/// Restart the running service (stop + start).
pub fn restart(config_path: Option<&str>, profile: Option<&str>) -> Result<()> {
    if cfg!(target_os = "macos") {
        restart_launchd(profile)
    } else if cfg!(target_os = "linux") {
        let _ = stop(config_path, profile);
        start(config_path, profile)
    } else {
        Err(Error::Validation(
            "Daemon restart is only supported on macOS and Linux".into(),
        ))
    }
}

/// Show service status: installed, running, PID, uptime hints.
pub fn status(config_path: Option<&str>, profile: Option<&str>) -> Result<()> {
    let config_dir = resolve_daemon_config_dir(config_path, profile)?;

    println!("NyxID Node Agent Service Status");
    println!("================================");

    let config_file = config_dir.join("config.toml");
    if !config_file.exists() {
        println!("Config:     not found (run `nyxid node register` first)");
        return Ok(());
    }
    println!("Config:     {}", config_file.display());

    let pid_file = config_dir.join(PID_FILE_NAME);
    let pid_running = read_pid_if_running(&pid_file);
    if let Some(pid) = pid_running {
        println!("PID:        {} (running)", pid);
    } else if pid_file.exists() {
        println!("PID:        stale (process not running)");
    } else {
        println!("PID:        -");
    }

    if cfg!(target_os = "macos") {
        status_launchd(profile)?;
    } else if cfg!(target_os = "linux") {
        status_systemd(profile)?;
    } else {
        println!("Service:    unsupported platform");
    }

    let log_dir = config_dir.join(LOG_DIR_NAME);
    if log_dir.exists() {
        println!("Logs:       {}", log_dir.display());
    }

    Ok(())
}

/// Tail / show recent log output.
pub fn logs(
    config_path: Option<&str>,
    profile: Option<&str>,
    follow: bool,
    lines: usize,
) -> Result<()> {
    let config_dir = resolve_daemon_config_dir(config_path, profile)?;
    let log_dir = config_dir.join(LOG_DIR_NAME);

    if cfg!(target_os = "macos") {
        logs_launchd(&log_dir, follow, lines)
    } else if cfg!(target_os = "linux") {
        logs_systemd(profile, follow, lines)
    } else {
        Err(Error::Validation(
            "Daemon logs are only supported on macOS and Linux".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// macOS LaunchAgent
// ---------------------------------------------------------------------------

fn plist_path_for(profile: Option<&str>) -> Result<PathBuf> {
    let home = home_dir()?;
    let label = launchd_label(profile)?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist")))
}

fn install_launchd(
    nyxid_bin: &Path,
    log_dir: &Path,
    config_dir: &Path,
    profile: Option<&str>,
    log_level: Option<&str>,
    force: bool,
) -> Result<()> {
    let label = launchd_label(profile)?;
    let plist = plist_path_for(profile)?;
    if plist.exists() && !force {
        return Err(Error::Validation(format!(
            "Service already installed at {}. Use --force to overwrite.",
            plist.display()
        )));
    }
    if force {
        let _ = launchd_bootout(profile, true);
    }

    // Build ProgramArguments as separate tokens (handles paths with spaces)
    let bin_str = nyxid_bin.to_string_lossy();
    let mut args = vec![bin_str.to_string(), "node".into(), "start".into()];
    args.push("--config".to_string());
    args.push(config_dir.display().to_string());
    if let Some(ll) = log_level {
        args.push("--log-level".to_string());
        args.push(ll.to_string());
    }

    let program_args_xml: String = args
        .iter()
        .map(|a| format!("        <string>{}</string>", xml_escape(a)))
        .collect::<Vec<_>>()
        .join("\n");

    let stdout_log = log_dir.join("node-agent.log");
    let stderr_log = log_dir.join("node-agent.err.log");
    let cargo_bin = home_dir()?.join(".cargo/bin").display().to_string();

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{program_args_xml}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
    <key>ThrottleInterval</key>
    <integer>5</integer>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:{cargo_bin}</string>
    </dict>
</dict>
</plist>"#,
        stdout = xml_escape(&stdout_log.display().to_string()),
        stderr = xml_escape(&stderr_log.display().to_string()),
        cargo_bin = xml_escape(&cargo_bin),
    );

    let parent = plist
        .parent()
        .ok_or_else(|| Error::Config(format!("Invalid plist path: {}", plist.display())))?;
    fs::create_dir_all(parent)?;
    fs::write(&plist, plist_content)?;

    println!("Service installed at {}", plist.display());
    println!();
    println!("Start it with:");
    println!("  nyxid node daemon start");
    println!();
    println!("The agent will start automatically on login and restart on crash.");

    Ok(())
}

fn uninstall_launchd(profile: Option<&str>) -> Result<()> {
    let _ = launchd_bootout(profile, true);
    let plist = plist_path_for(profile)?;
    if plist.exists() {
        fs::remove_file(&plist)?;
        println!("Removed {}", plist.display());
    } else {
        println!("No service installed (nothing to remove).");
    }
    Ok(())
}

fn start_launchd(profile: Option<&str>) -> Result<()> {
    let plist = plist_path_for(profile)?;
    if !plist.exists() {
        return Err(Error::Validation(
            "Service not installed. Run `nyxid node daemon install` first.".into(),
        ));
    }

    if launchd_is_loaded(profile)? {
        let target = launchd_target_for(profile)?;
        let kick = Command::new("launchctl")
            .args(["kickstart", &target])
            .output()?;

        if kick.status.success() {
            println!("Node agent started.");
        } else {
            let stderr = String::from_utf8_lossy(&kick.stderr);
            return Err(Error::Config(format!(
                "launchctl kickstart failed: {stderr}"
            )));
        }
    } else {
        bootstrap_launchd(&plist)?;
        println!("Node agent started.");
    }

    Ok(())
}

fn stop_launchd(profile: Option<&str>) -> Result<()> {
    if !launchd_is_loaded(profile)? {
        println!("Node agent is not running.");
        Ok(())
    } else {
        launchd_bootout(profile, false)?;
        println!("Node agent stopped.");
        Ok(())
    }
}

fn restart_launchd(profile: Option<&str>) -> Result<()> {
    let plist = plist_path_for(profile)?;
    if !plist.exists() {
        return Err(Error::Validation(
            "Service not installed. Run `nyxid node daemon install` first.".into(),
        ));
    }

    // If the job is already loaded, use `launchctl kickstart -k` to atomically
    // kill the current instance and respawn it. This avoids the race where an
    // immediate `bootstrap` after `bootout` hits launchd's async teardown and
    // fails with "Input/output error" (errno 5) because the job label is still
    // in the domain.
    if launchd_is_loaded(profile)? {
        let target = launchd_target_for(profile)?;
        let output = Command::new("launchctl")
            .args(["kickstart", "-k", &target])
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Config(format!(
                "launchctl kickstart -k failed: {stderr}"
            )));
        }
        println!("Node agent restarted.");
        return Ok(());
    }

    bootstrap_launchd(&plist)?;
    println!("Node agent restarted.");

    Ok(())
}

fn status_launchd(profile: Option<&str>) -> Result<()> {
    let plist = plist_path_for(profile)?;
    if !plist.exists() {
        println!("Installed:  no");
        println!();
        println!("Run `nyxid node daemon install` to install as a background service.");
        return Ok(());
    }
    println!("Installed:  yes (launchd)");

    let target = launchd_target_for(profile)?;

    let output = Command::new("launchctl")
        .args(["print", &target])
        .output()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);

        let pid = stdout
            .lines()
            .find(|l| l.trim().starts_with("pid ="))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().to_string());

        let exit_status = stdout
            .lines()
            .find(|l| l.trim().starts_with("last exit code ="))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().to_string());

        if let Some(ref pid) = pid {
            if pid != "0" && !pid.is_empty() {
                println!("Running:    yes (PID {pid})");
            } else {
                println!("Running:    no");
            }
        } else {
            println!("Running:    no");
        }

        if let Some(ref code) = exit_status {
            println!("Last exit:  {code}");
        }
    } else {
        println!("Running:    no (service not loaded)");
    }

    Ok(())
}

fn logs_launchd(log_dir: &Path, follow: bool, lines: usize) -> Result<()> {
    let log_file = log_dir.join("node-agent.log");
    let err_file = log_dir.join("node-agent.err.log");

    if !log_file.exists() && !err_file.exists() {
        println!("No log files found at {}", log_dir.display());
        println!("Start the service first with `nyxid node daemon start`.");
        return Ok(());
    }

    if follow {
        let mut cmd = Command::new("tail");
        cmd.arg("-f");
        if log_file.exists() {
            cmd.arg(&log_file);
        }
        if err_file.exists() {
            cmd.arg(&err_file);
        }
        let status = cmd.status()?;
        if !status.success() {
            return Err(Error::Io(std::io::Error::other("tail command failed")));
        }
    } else {
        let target = if log_file.exists() {
            &log_file
        } else {
            &err_file
        };
        let output = Command::new("tail")
            .args(["-n", &lines.to_string()])
            .arg(target)
            .output()?;
        print!("{}", String::from_utf8_lossy(&output.stdout));
        let stderr_out = String::from_utf8_lossy(&output.stderr);
        if !stderr_out.is_empty() {
            eprint!("{stderr_out}");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Linux systemd
// ---------------------------------------------------------------------------

fn unit_path_for(profile: Option<&str>) -> Result<PathBuf> {
    let home = home_dir()?;
    let unit = systemd_unit(profile)?;
    Ok(home.join(".config/systemd/user").join(unit))
}

fn install_systemd(
    nyxid_bin: &Path,
    config_dir: &Path,
    profile: Option<&str>,
    log_level: Option<&str>,
    force: bool,
) -> Result<()> {
    let unit_name = systemd_unit(profile)?;
    let unit = unit_path_for(profile)?;
    if unit.exists() && !force {
        return Err(Error::Validation(format!(
            "Service already installed at {}. Use --force to overwrite.",
            unit.display()
        )));
    }

    let exec_start = build_systemd_exec_start(nyxid_bin, config_dir, log_level);

    let cargo_bin = home_dir()?.join(".cargo/bin").display().to_string();

    let unit_content = format!(
        r#"[Unit]
Description=NyxID Node Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={exec_start}
Restart=always
RestartSec=5
Environment=PATH=/usr/local/bin:/usr/bin:/bin:{cargo_bin}

[Install]
WantedBy=default.target
"#
    );

    let parent = unit
        .parent()
        .ok_or_else(|| Error::Config(format!("Invalid unit path: {}", unit.display())))?;
    fs::create_dir_all(parent)?;
    fs::write(&unit, unit_content)?;

    // Reload systemd
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();

    // Enable the service so it starts on login
    let _ = Command::new("systemctl")
        .args(["--user", "enable", &unit_name])
        .output();

    // Enable lingering so service runs after logout
    let user = std::env::var("USER").unwrap_or_default();
    if !user.is_empty() {
        let linger = Command::new("loginctl")
            .args(["enable-linger", &user])
            .output();
        match linger {
            Ok(out) if out.status.success() => {}
            _ => {
                eprintln!("  [warn] Could not enable linger. The service may stop after logout.");
                eprintln!("         Run: sudo loginctl enable-linger {user}");
            }
        }
    } else {
        eprintln!("  [warn] USER env not set -- could not enable linger.");
    }

    println!("Service installed at {}", unit.display());
    println!();
    println!("Start it with:");
    println!("  nyxid node daemon start");
    println!();
    println!("The agent will start automatically on login and restart on crash.");

    Ok(())
}

fn uninstall_systemd(profile: Option<&str>) -> Result<()> {
    let unit_name = systemd_unit(profile)?;
    let unit = unit_path_for(profile)?;

    let _ = Command::new("systemctl")
        .args(["--user", "disable", &unit_name])
        .output();

    if unit.exists() {
        fs::remove_file(&unit)?;
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        println!("Removed {}", unit.display());
    } else {
        println!("No service installed (nothing to remove).");
    }
    Ok(())
}

fn start_systemd(profile: Option<&str>) -> Result<()> {
    let unit_name = systemd_unit(profile)?;
    let unit = unit_path_for(profile)?;
    if !unit.exists() {
        return Err(Error::Validation(
            "Service not installed. Run `nyxid node daemon install` first.".into(),
        ));
    }

    let output = Command::new("systemctl")
        .args(["--user", "start", &unit_name])
        .output()?;

    if output.status.success() {
        println!("Node agent started.");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Config(format!("systemctl start failed: {stderr}")));
    }

    Ok(())
}

fn stop_systemd(profile: Option<&str>) -> Result<()> {
    let unit_name = systemd_unit(profile)?;

    let output = Command::new("systemctl")
        .args(["--user", "stop", &unit_name])
        .output()?;

    if output.status.success() {
        println!("Node agent stopped.");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not loaded") || stderr.contains("not found") {
            println!("Node agent is not running.");
        } else {
            return Err(Error::Config(format!("systemctl stop failed: {stderr}")));
        }
    }

    Ok(())
}

fn status_systemd(profile: Option<&str>) -> Result<()> {
    let unit_name = systemd_unit(profile)?;
    let unit = unit_path_for(profile)?;
    if !unit.exists() {
        println!("Installed:  no");
        println!();
        println!("Run `nyxid node daemon install` to install as a background service.");
        return Ok(());
    }
    println!("Installed:  yes (systemd)");

    let output = Command::new("systemctl")
        .args(["--user", "is-active", &unit_name])
        .output()?;

    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match state.as_str() {
        "active" => println!("Running:    yes"),
        "inactive" => println!("Running:    no (inactive)"),
        "failed" => println!("Running:    no (failed)"),
        other => println!("Running:    {other}"),
    }

    let pid_output = Command::new("systemctl")
        .args(["--user", "show", &unit_name, "--property=MainPID"])
        .output()?;
    let pid_line = String::from_utf8_lossy(&pid_output.stdout);
    if let Some(pid_str) = pid_line.trim().strip_prefix("MainPID=")
        && pid_str != "0"
        && !pid_str.is_empty()
    {
        println!("PID:        {pid_str}");
    }

    Ok(())
}

fn logs_systemd(profile: Option<&str>, follow: bool, lines: usize) -> Result<()> {
    let unit_name = systemd_unit(profile)?;
    let mut cmd = Command::new("journalctl");
    cmd.args([
        "--user-unit",
        &unit_name,
        "-n",
        &lines.to_string(),
        "--no-pager",
    ]);
    if follow {
        cmd.arg("-f");
    }

    let status = cmd.status()?;
    if !status.success() {
        return Err(Error::Io(std::io::Error::other(
            "journalctl command failed",
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the user's home directory, returning an error if unavailable.
fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| Error::Config("Could not determine home directory".into()))
}

/// Resolve the effective config directory for daemon subcommands.
///
/// If the user passes `--config`, use that. Otherwise, check if `install`
/// previously saved a config path in `daemon.toml` and use that. Falls back
/// to the default `~/.nyxid-node/`.
fn resolve_daemon_config_dir(config_path: Option<&str>, profile: Option<&str>) -> Result<PathBuf> {
    if config_path.is_some() {
        return Ok(config::resolve_config_dir(config_path));
    }

    // Try to read the path saved by `daemon install`
    if let Some(saved) = load_daemon_config_dir(profile)? {
        return Ok(saved);
    }

    config::resolve_config_dir_with_profile(None, profile)
}

/// Save the canonical config directory so other daemon subcommands can find it.
fn save_daemon_config_dir(config_dir: &Path, profile: Option<&str>) -> Result<()> {
    let default_dir = config::resolve_config_dir_with_profile(None, profile)?;
    let meta_file = default_dir.join(DAEMON_META_FILE);
    fs::create_dir_all(&default_dir)?;
    let content = format!("config_dir = \"{}\"\n", config_dir.display());
    fs::write(&meta_file, content)?;
    Ok(())
}

/// Load the config directory saved by `daemon install`.
fn load_daemon_config_dir(profile: Option<&str>) -> Result<Option<PathBuf>> {
    let default_dir = config::resolve_config_dir_with_profile(None, profile)?;
    let meta_file = default_dir.join(DAEMON_META_FILE);
    let Ok(content) = fs::read_to_string(&meta_file) else {
        return Ok(None);
    };
    let Ok(table) = content.parse::<toml::Table>() else {
        return Ok(None);
    };
    let Some(dir) = table.get("config_dir").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let path = PathBuf::from(dir);
    Ok(if path.exists() { Some(path) } else { None })
}

/// Resolve a config directory to an absolute path for long-lived service files.
fn canonicalize_existing_dir(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|e| {
        Error::Config(format!(
            "Failed to resolve absolute config directory {}: {e}",
            path.display()
        ))
    })
}

fn launchd_domain() -> String {
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}")
}

fn launchd_target_for(profile: Option<&str>) -> Result<String> {
    Ok(format!("{}/{}", launchd_domain(), launchd_label(profile)?))
}

fn launchd_is_loaded(profile: Option<&str>) -> Result<bool> {
    let output = Command::new("launchctl")
        .args(["print", &launchd_target_for(profile)?])
        .output()?;
    Ok(output.status.success())
}

fn launchd_bootout(profile: Option<&str>, ignore_missing: bool) -> Result<()> {
    let target = launchd_target_for(profile)?;
    let output = Command::new("launchctl")
        .args(["bootout", &target])
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if ignore_missing
        && (stderr.contains("No such process")
            || stderr.contains("Could not find service")
            || stderr.contains("not found")
            || output.status.code() == Some(3))
    {
        return Ok(());
    }

    Err(Error::Config(format!("launchctl bootout failed: {stderr}")))
}

fn bootstrap_launchd(plist: &Path) -> Result<()> {
    let domain = launchd_domain();
    let output = Command::new("launchctl")
        .args(["bootstrap", &domain, &plist.to_string_lossy()])
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::Config(format!(
            "launchctl bootstrap failed: {stderr}"
        )))
    }
}

/// Escape a string for use inside XML plist `<string>` elements.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Resolve the absolute path of the `nyxid` binary (current executable).
fn resolve_binary() -> Result<PathBuf> {
    std::env::current_exe().map_err(|e| {
        Error::Config(format!(
            "Could not determine nyxid binary path: {e}. \
             Make sure nyxid is installed via `cargo install`."
        ))
    })
}

/// Check that config.toml exists in the config directory.
fn ensure_config_exists(config_dir: &Path) -> Result<()> {
    let config_file = config_dir.join("config.toml");
    if !config_file.exists() {
        return Err(Error::Validation(format!(
            "Node not registered. Run `nyxid node register` first.\n\
             Expected config at: {}",
            config_file.display()
        )));
    }
    Ok(())
}

fn systemd_quote_arg(arg: &str) -> String {
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn build_systemd_exec_start(
    nyxid_bin: &Path,
    config_dir: &Path,
    log_level: Option<&str>,
) -> String {
    let mut parts = vec![
        systemd_quote_arg(&nyxid_bin.display().to_string()),
        systemd_quote_arg("node"),
        systemd_quote_arg("start"),
        systemd_quote_arg("--config"),
        systemd_quote_arg(&config_dir.display().to_string()),
    ];
    if let Some(level) = log_level {
        parts.push(systemd_quote_arg("--log-level"));
        parts.push(systemd_quote_arg(level));
    }
    parts.join(" ")
}

/// Read PID from file and check if process is still alive.
fn read_pid_if_running(pid_file: &Path) -> Option<u32> {
    let content = fs::read_to_string(pid_file).ok()?;
    let pid: u32 = content.trim().parse().ok()?;

    // Use nix crate for portable signal-0 check (handles EPERM correctly)
    use nix::sys::signal;
    use nix::unistd::Pid;

    match signal::kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => Some(pid),
        Err(nix::errno::Errno::EPERM) => Some(pid), // exists but different user
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use tempfile::NamedTempFile;

    #[test]
    fn launchd_label_default() {
        assert_eq!(launchd_label(None).unwrap(), "dev.nyxid.node");
        assert_eq!(launchd_label(Some("default")).unwrap(), "dev.nyxid.node");
    }

    #[test]
    fn launchd_label_profiled() {
        assert_eq!(
            launchd_label(Some("coding-agent")).unwrap(),
            "dev.nyxid.node.coding-agent"
        );
    }

    #[test]
    fn launchd_label_rejects_unsafe_name() {
        assert!(launchd_label(Some("../../etc")).is_err());
        assert!(launchd_label(Some("; rm -rf /")).is_err());
    }

    #[test]
    fn systemd_unit_default() {
        assert_eq!(systemd_unit(None).unwrap(), "nyxid-node.service");
        assert_eq!(systemd_unit(Some("default")).unwrap(), "nyxid-node.service");
    }

    #[test]
    fn systemd_unit_profiled() {
        assert_eq!(
            systemd_unit(Some("research")).unwrap(),
            "nyxid-node-research.service"
        );
    }

    #[test]
    fn systemd_unit_rejects_unsafe_name() {
        assert!(systemd_unit(Some("../../etc")).is_err());
        assert!(systemd_unit(Some("; rm -rf /")).is_err());
    }

    #[test]
    fn xml_escape_special_chars() {
        assert_eq!(xml_escape("hello"), "hello");
        assert_eq!(xml_escape("<foo>&bar"), "&lt;foo&gt;&amp;bar");
        assert_eq!(xml_escape(r#"a"b'c"#), "a&quot;b&apos;c");
    }

    #[test]
    fn xml_escape_path_with_spaces() {
        let path = "/Users/John Doe/.nyxid-node";
        assert_eq!(xml_escape(path), path); // no special chars, should be unchanged
    }

    #[test]
    fn read_pid_missing_file() {
        let result = read_pid_if_running(Path::new("/nonexistent/pid"));
        assert!(result.is_none());
    }

    #[test]
    fn read_pid_invalid_content() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "not-a-number").unwrap();
        assert!(read_pid_if_running(f.path()).is_none());
    }

    #[test]
    fn read_pid_stale() {
        let mut f = NamedTempFile::new().unwrap();
        // PID 99999999 almost certainly doesn't exist
        writeln!(f, "99999999").unwrap();
        assert!(read_pid_if_running(f.path()).is_none());
    }

    #[test]
    fn read_pid_current_process() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "{}", std::process::id()).unwrap();
        assert_eq!(read_pid_if_running(f.path()), Some(std::process::id()));
    }

    #[test]
    fn ensure_config_exists_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = ensure_config_exists(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn ensure_config_exists_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.toml"), "# test").unwrap();
        assert!(ensure_config_exists(dir.path()).is_ok());
    }

    #[test]
    fn log_level_validation() {
        for valid in VALID_LOG_LEVELS {
            assert!(VALID_LOG_LEVELS.contains(valid));
        }
        assert!(!VALID_LOG_LEVELS.contains(&"invalid"));
        assert!(!VALID_LOG_LEVELS.contains(&"info --config /tmp/evil"));
    }

    #[test]
    fn canonicalize_existing_dir_makes_relative_paths_absolute() {
        let temp = tempfile::tempdir_in(".").unwrap();
        let relative = PathBuf::from(temp.path().file_name().unwrap());
        let absolute = canonicalize_existing_dir(&relative).unwrap();
        assert!(absolute.is_absolute());
        assert_eq!(absolute, temp.path().canonicalize().unwrap());
    }

    #[test]
    fn systemd_exec_start_quotes_paths_and_config() {
        let command = build_systemd_exec_start(
            Path::new("/tmp/Nyx ID/bin/nyxid"),
            Path::new("/tmp/Nyx ID/config dir"),
            Some("debug"),
        );
        assert_eq!(
            command,
            "\"/tmp/Nyx ID/bin/nyxid\" \"node\" \"start\" \"--config\" \"/tmp/Nyx ID/config dir\" \"--log-level\" \"debug\""
        );
    }
}
