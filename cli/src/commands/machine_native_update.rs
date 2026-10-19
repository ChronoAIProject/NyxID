//! A distinct launchd/systemd service survives node replacement and rolls back.
use crate::machine_updater as mailbox;
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use nyxid_machine::update::{self, Phase, Progress};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command as Process,
    time::Duration,
};

#[derive(Subcommand)]
pub enum Command {
    #[command(hide = true)]
    Health,
    /// Install updates for an existing native node (run as its supervisor user).
    Install {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        system: bool,
    },
    #[command(hide = true)]
    Watch {
        #[arg(long)]
        config: PathBuf,
    },
    /// Migrate an official container; run on its Docker host, never inside it.
    MigrateContainer {
        container: String,
        #[arg(long)]
        version: String,
    },
    #[command(hide = true)]
    WatchContainer { container: String },
}
#[derive(Serialize, Deserialize)]
struct Native {
    profile: Option<String>,
    system: bool,
    version: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    previous: PathBuf,
    progress: Progress,
}

pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Health => {
            let root = Path::new(update::UPDATE_VOLUME);
            let connected: update::Connected =
                serde_json::from_slice(&mailbox::read(root, "connected.json", 4096)?)?;
            ensure!(
                connected.version == env!("CARGO_PKG_VERSION")
                    && mailbox::now_ms().saturating_sub(connected.at_ms) < 90000,
                "Machine is not connected"
            );
            Ok(())
        }

        Command::Install {
            config,
            profile,
            system,
        } => {
            let config = if system && config.is_none() {
                PathBuf::from("/var/lib/nyxid-machine")
                    .join(safe_profile(profile.as_deref())?)
                    .join("node")
            } else {
                crate::node::config::resolve_config_dir_with_profile(
                    config.as_deref(),
                    profile.as_deref(),
                )?
            };
            crate::node::config::NodeConfig::load(&config.join("config.toml"))?;
            prepare(&config, profile.as_deref(), system)?;
            if system {
                bind_system_node(&config, profile.as_deref())?;
            } else {
                crate::node::daemon::install(config.to_str(), profile.as_deref(), None, true)?;
            }
            install(&config, profile.as_deref(), system)?;
            if system {
                restart(&Native {
                    profile,
                    system,
                    version: env!("CARGO_PKG_VERSION").into(),
                })?;
            } else {
                crate::node::daemon::restart(config.to_str(), profile.as_deref())?;
            }
            Ok(())
        }
        Command::Watch { config } => watch(config).await,
        Command::MigrateContainer { container, version } => {
            let digest = mailbox::docker::Docker::new()?
                .verified_digest(update::UPDATER_IMAGE, &version, None)
                .await?;
            let image = format!("{}@{digest}", update::UPDATER_IMAGE);
            let args =
                update::migration_args(&container, &version, &image).map_err(anyhow::Error::msg)?;
            ensure!(
                tokio::process::Command::new("docker")
                    .args(args)
                    .status()
                    .await?
                    .success(),
                "Machine migration failed; the previous container was retained for recovery"
            );
            Ok(())
        }
        Command::WatchContainer { container } => {
            mailbox::run(update::UPDATE_VOLUME.into(), container).await
        }
    }
}

fn bind_system_node(config: &Path, profile: Option<&str>) -> Result<()> {
    let name = safe_profile(profile)?;
    let path = PathBuf::from(format!("/etc/systemd/system/nyxid-machine-{name}.service"));
    let source = std::fs::read_to_string(&path)?;
    let config = config.canonicalize()?;
    let config = config.to_str().context("Invalid config path")?;
    ensure!(
        !config.contains(['\n', '\r', '%', '$', '"', '\\']),
        "Unsupported service path"
    );
    let mut changed = 0;
    let contents = source
        .lines()
        .map(|line| {
            if line.starts_with("ExecStart=") {
                changed += 1;
                format!(
                    "ExecStart=\"{config}/machine-update/active\" node start --config \"{config}\""
                )
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    ensure!(
        changed == 1,
        "Could not identify supervisor service entry point"
    );
    std::fs::write(path, contents + "\n")?;
    Ok(())
}

fn safe_profile(profile: Option<&str>) -> Result<&str> {
    let name = profile.unwrap_or("default");
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
        "Invalid node profile"
    );
    Ok(name)
}

pub fn prepare(config: &Path, profile: Option<&str>, system: bool) -> Result<PathBuf> {
    safe_profile(profile)?;
    let root = config.join("machine-update");
    mailbox::private_directory(&root)?;
    let executable = std::env::current_exe()?;
    let controller = root.join("controller");
    if !controller.exists() {
        std::fs::copy(&executable, &controller)?;
    }
    let active = root.join("active");
    if !active.exists() {
        let initial = root
            .join("versions")
            .join(format!("v{}", env!("CARGO_PKG_VERSION")))
            .join("nyxid");
        std::fs::create_dir_all(initial.parent().unwrap())?;
        std::fs::copy(executable, &initial)?;
        super::update::activate_machine_binary(&active, &initial)?;
    }
    if !root.join("native.json").exists() {
        mailbox::write(
            &root,
            "native.json",
            &serde_json::to_vec(&Native {
                profile: profile.map(str::to_owned),
                system,
                version: env!("CARGO_PKG_VERSION").into(),
            })?,
        )?;
    }
    Ok(active)
}

pub fn install(config: &Path, profile: Option<&str>, system: bool) -> Result<()> {
    let config = config.canonicalize()?;
    prepare(&config, profile, system)?;
    let root = config.join("machine-update");
    let name = safe_profile(profile)?;
    if cfg!(target_os = "macos") {
        ensure!(!system, "Separated system machines use Linux systemd");
        let label = format!("dev.nyxid.machine-updater.{name}");
        let folder = dirs::home_dir()
            .context("Home unavailable")?
            .join("Library/LaunchAgents");
        std::fs::create_dir_all(&folder)?;
        let path = folder.join(format!("{label}.plist"));
        let mut data = plist::Dictionary::new();
        data.insert("Label".into(), label.clone().into());
        data.insert(
            "ProgramArguments".into(),
            plist::Value::Array(
                [
                    root.join("controller").to_string_lossy().into_owned(),
                    "node".into(),
                    "machine-updater".into(),
                    "watch".into(),
                    "--config".into(),
                    config.to_string_lossy().into_owned(),
                ]
                .into_iter()
                .map(Into::into)
                .collect(),
            ),
        );
        data.insert("RunAtLoad".into(), true.into());
        data.insert("KeepAlive".into(), true.into());
        plist::Value::Dictionary(data).to_file_xml(&path)?;
        let domain = format!("gui/{}", unsafe { libc::geteuid() });
        let _ = Process::new("launchctl")
            .args(["bootout", &format!("{domain}/{label}")])
            .status();
        ensure!(
            Process::new("launchctl")
                .arg("bootstrap")
                .arg(domain)
                .arg(path)
                .status()?
                .success(),
            "Could not start updater service"
        );
    } else {
        let folder = if system {
            PathBuf::from("/etc/systemd/system")
        } else {
            dirs::home_dir()
                .context("Home unavailable")?
                .join(".config/systemd/user")
        };
        std::fs::create_dir_all(&folder)?;
        let unit = format!("nyxid-machine-updater-{name}.service");
        let quote = |p: &Path| -> Result<String> {
            let p = p.to_str().context("Invalid service path")?;
            ensure!(
                !p.contains(['\n', '\r', '%', '$']),
                "Unsupported service path"
            );
            Ok(format!(
                "\"{}\"",
                p.replace('\\', "\\\\").replace('"', "\\\"")
            ))
        };
        std::fs::write(
            folder.join(&unit),
            format!(
                "[Unit]\nDescription=NyxID verified machine updater\nAfter=network-online.target\n[Service]\nExecStart={} node machine-updater watch --config {}\nRestart=always\nUMask=0077\nLimitCORE=0\n[Install]\nWantedBy={}\n",
                quote(&root.join("controller"))?,
                quote(&config)?,
                if system {
                    "multi-user.target"
                } else {
                    "default.target"
                }
            ),
        )?;
        for args in [vec!["daemon-reload"], vec!["enable", "--now", &unit]] {
            let mut cmd = Process::new("systemctl");
            if !system {
                cmd.arg("--user");
            }
            ensure!(
                cmd.args(args).status()?.success(),
                "Could not start updater service"
            );
        }
    }
    Ok(())
}

fn restart(native: &Native) -> Result<()> {
    let name = safe_profile(native.profile.as_deref())?;
    let mut command = if cfg!(target_os = "macos") {
        let label = if name == "default" {
            "dev.nyxid.node".into()
        } else {
            format!("dev.nyxid.node.{name}")
        };
        let mut cmd = Process::new("launchctl");
        cmd.args([
            "kickstart",
            "-k",
            &format!("gui/{}/{label}", unsafe { libc::geteuid() }),
        ]);
        cmd
    } else {
        let mut cmd = Process::new("systemctl");
        if !native.system {
            cmd.arg("--user");
        }
        let unit = if native.system {
            format!("nyxid-machine-{name}.service")
        } else if name == "default" {
            "nyxid-node.service".into()
        } else {
            format!("nyxid-node-{name}.service")
        };
        cmd.args(["restart", &unit]);
        cmd
    };
    ensure!(command.status()?.success(), "Node service restart failed");
    Ok(())
}

fn restore(root: &Path, native: &Native, journal: &Journal) -> Result<()> {
    restore_with(root, journal, || restart(native))
}

fn restore_with(
    root: &Path,
    journal: &Journal,
    restart: impl FnOnce() -> Result<()>,
) -> Result<()> {
    super::update::activate_machine_binary(&root.join("active"), &journal.previous)?;
    restart()?;
    let mut journal = journal.clone();
    journal.progress.phase = Phase::RolledBack;
    journal.progress.code = Some("native_reconnect_timeout".into());
    mailbox::write(root, "native-journal.json", &serde_json::to_vec(&journal)?)?;
    mailbox::report(root, &journal.progress)?;
    mailbox::finish_request(root, "native-journal.json")
}

fn finish_committed(root: &Path, native: &mut Native, journal: &Journal) -> Result<()> {
    native.version.clone_from(&journal.progress.target);
    mailbox::write(root, "native.json", &serde_json::to_vec(native)?)?;
    mailbox::report(root, &journal.progress)?;
    mailbox::finish_request(root, "native-journal.json")
}

fn refresh_controller(root: &Path) -> Result<()> {
    let staged = tempfile::NamedTempFile::new_in(root)?;
    std::fs::copy(root.join("active"), staged.path())?;
    staged.as_file().sync_all()?;
    staged.persist(root.join("controller"))?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

async fn attempt(root: &Path, native: &mut Native, target: &str, rollback: bool) -> Result<()> {
    update::validate_target(&native.version, target, rollback).map_err(anyhow::Error::msg)?;
    let mut progress = Progress {
        updater: None,
        target: target.into(),
        phase: Phase::Verifying,
        started_at_ms: mailbox::now_ms(),
        code: None,
    };
    mailbox::report(root, &progress)?;
    let binary = super::update::download_machine_release(target, &root.join("versions")).await?;
    let mut journal = Journal {
        previous: root.join("active").canonicalize()?,
        progress: progress.clone(),
    };
    mailbox::write(root, "native-journal.json", &serde_json::to_vec(&journal)?)?;
    progress.phase = Phase::Restarting;
    mailbox::report(root, &progress)?;
    super::update::activate_machine_binary(&root.join("active"), &binary)?;
    if restart(native).is_err() {
        return restore(root, native, &journal);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(update::RECONNECT_SECONDS);
    while tokio::time::Instant::now() < deadline {
        mailbox::heartbeat(root)?;
        if mailbox::connected(root, target, progress.started_at_ms) {
            journal.progress.phase = Phase::Connected;
            mailbox::write(root, "native-journal.json", &serde_json::to_vec(&journal)?)?;
            return finish_committed(root, native, &journal);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    restore(root, native, &journal)
}

async fn watch(config: PathBuf) -> Result<()> {
    let root = config.join("machine-update");
    mailbox::private_directory(&root)?;
    let _lock = mailbox::lock(&root)?;
    let _heartbeat = mailbox::heartbeat_task(&root);
    let mut native: Native = serde_json::from_slice(&mailbox::read(&root, "native.json", 4096)?)?;
    if root.join("native-journal.json").exists() {
        let journal: Journal =
            serde_json::from_slice(&mailbox::read(&root, "native-journal.json", 16384)?)?;
        match journal.progress.phase {
            Phase::Connected => finish_committed(&root, &mut native, &journal)?,
            Phase::RolledBack => {
                mailbox::report(&root, &journal.progress)?;
                mailbox::finish_request(&root, "native-journal.json")?;
            }
            _ => restore(&root, &native, &journal)?,
        }
    }
    // A crash after committing the node but before replacing this independent
    // controller must also finish the controller update on service restart.
    if native.version != env!("CARGO_PKG_VERSION") {
        refresh_controller(&root)?;
        return Ok(());
    }
    loop {
        mailbox::heartbeat(&root)?;
        if let Some((target, rollback)) = mailbox::request(&root)? {
            if attempt(&root, &mut native, &target, rollback)
                .await
                .is_err()
            {
                ensure!(
                    !root.join("native-journal.json").exists(),
                    "Native update recovery pending"
                );
                mailbox::report(
                    &root,
                    &Progress {
                        updater: None,
                        target,
                        phase: Phase::Failed,
                        started_at_ms: mailbox::now_ms(),
                        code: Some("native_verification_or_update_failed".into()),
                    },
                )?;
            }
            for name in ["request", "rollback-request"] {
                let _ = std::fs::remove_file(root.join(name));
            }
            let progress: Progress =
                serde_json::from_slice(&mailbox::read(&root, "progress.json", 4096)?)?;
            if progress.phase == Phase::Connected {
                refresh_controller(&root)?;
                // launchd KeepAlive / systemd Restart=always starts the verified
                // controller after all progress and request bookkeeping is durable.
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_native_recovery_preserves_new_binary_and_updates_version_once() {
        let root = tempfile::tempdir().unwrap();
        let active = root.path().join("active");
        std::fs::write(&active, b"new verified binary").unwrap();
        let journal = Journal {
            previous: root.path().join("previous"),
            progress: Progress {
                updater: None,
                target: "0.41.0".into(),
                phase: Phase::Connected,
                started_at_ms: 1,
                code: None,
            },
        };
        for recorded_version in ["0.40.0", "0.41.0"] {
            let mut native = Native {
                profile: None,
                system: false,
                version: recorded_version.into(),
            };
            mailbox::write(
                root.path(),
                "native-journal.json",
                &serde_json::to_vec(&journal).unwrap(),
            )
            .unwrap();
            mailbox::write(root.path(), "request", b"0.41.0").unwrap();
            finish_committed(root.path(), &mut native, &journal).unwrap();
            assert_eq!(native.version, "0.41.0");
            assert_eq!(std::fs::read(&active).unwrap(), b"new verified binary");
            assert!(mailbox::request(root.path()).unwrap().is_none());
            assert!(!root.path().join("native-journal.json").exists());
        }
    }

    #[test]
    fn native_rollback_restores_previous_binary_before_service_restart_and_survives_retry() {
        let root = tempfile::tempdir().unwrap();
        let previous = root.path().join("previous");
        let replacement = root.path().join("replacement");
        std::fs::write(&previous, b"old verified binary").unwrap();
        std::fs::write(&replacement, b"new verified binary").unwrap();
        let active = root.path().join("active");
        super::super::update::activate_machine_binary(&active, &replacement).unwrap();
        let journal = Journal {
            previous: previous.clone(),
            progress: Progress {
                updater: None,
                target: "0.41.0".into(),
                phase: Phase::Restarting,
                started_at_ms: 1,
                code: None,
            },
        };
        mailbox::write(
            root.path(),
            "native-journal.json",
            &serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        assert!(
            restore_with(root.path(), &journal, || {
                assert_eq!(std::fs::read(&active).unwrap(), b"old verified binary");
                anyhow::bail!("service temporarily unavailable")
            })
            .is_err()
        );
        assert!(root.path().join("native-journal.json").exists());
        restore_with(root.path(), &journal, || Ok(())).unwrap();
        let progress: Progress =
            serde_json::from_slice(&mailbox::read(root.path(), "progress.json", 4096).unwrap())
                .unwrap();
        assert_eq!(progress.phase, Phase::RolledBack);
        assert!(!root.path().join("native-journal.json").exists());
    }
}
