use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cli::AutoUpdateCommands;

#[cfg(test)]
mod tests;

const LABEL: &str = "dev.nyxid.update";
const TIMER: &str = "nyxid-update.timer";
const SERVICE: &str = "nyxid-update.service";
const TICK_SECONDS: u64 = 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Policy {
    pub enabled: bool,
    pub interval_hours: u32,
    pub held_version: Option<String>,
    pub hold_reason: Option<String>,
    pub active_binary: Option<PathBuf>,
    pub last_attempt: Option<DateTime<Utc>>,
    pub last_result: Option<String>,
    pub next_eligible_check: Option<DateTime<Utc>>,
    pub node_policy: String,
    #[serde(default)]
    pub pending_skills_version: Option<String>,
    #[serde(default)]
    pub pending_controller_version: Option<String>,
    #[serde(default)]
    pub last_activated_version: Option<String>,
    #[serde(default)]
    pub retention_result: Option<String>,
    #[serde(default)]
    pub activation_intent: Option<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: 24,
            held_version: None,
            hold_reason: None,
            active_binary: None,
            last_attempt: None,
            last_result: None,
            next_eligible_check: None,
            node_policy: "defer".into(),
            pending_skills_version: None,
            pending_controller_version: None,
            last_activated_version: None,
            retention_result: None,
            activation_intent: None,
        }
    }
}

fn root() -> Result<PathBuf> {
    let root = super::install_versions_root()?;
    Ok(if root.is_absolute() {
        root
    } else {
        std::env::current_dir()?.join(root)
    })
}

pub(crate) fn lock_update() -> Result<fs::File> {
    lock_in(&root()?)
}

fn lock_in(root: &Path) -> Result<fs::File> {
    fs::create_dir_all(root)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options.open(root.join(".update.lock"))?;
    lock.try_lock()
        .context("Another automatic, manual, or rollback update is running; retry later")?;
    Ok(lock)
}

fn read_in(root: &Path) -> Result<Policy> {
    match fs::read(root.join(".auto-update.json")) {
        Ok(bytes) => {
            let policy: Policy = serde_json::from_slice(&bytes).context("Automatic update policy is invalid; restore the policy file or disable automatic updates")?;
            anyhow::ensure!(
                (1..=720).contains(&policy.interval_hours) && policy.node_policy == "defer",
                "Unsupported automatic update policy; reconfigure with nyxid update auto enable"
            );
            Ok(policy)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Policy::default()),
        Err(error) => Err(error.into()),
    }
}

fn write_in(root: &Path, policy: &Policy) -> Result<()> {
    fs::create_dir_all(root)?;
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    temp.write_all(&serde_json::to_vec_pretty(policy)?)?;
    temp.as_file().sync_all()?;
    temp.persist(root.join(".auto-update.json"))?;
    #[cfg(unix)]
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

pub(crate) fn local_policy() -> Result<Policy> {
    read_in(&root()?)
}

pub(crate) fn hold_for_rollback(version: &str) -> Result<()> {
    let root = root()?;
    let mut policy = read_in(&root)?;
    policy.held_version = Some(version.into());
    policy.hold_reason =
        Some("Deliberate rollback; run nyxid update auto resume to allow upgrades".into());
    policy.next_eligible_check = None;
    write_in(&root, &policy)
}

fn active_version(root: &Path, binary: &Path) -> Result<String> {
    anyhow::ensure!(
        cfg!(any(target_os = "macos", target_os = "linux")),
        "Automatic upgrades require macOS launchd or Linux systemd user services"
    );
    anyhow::ensure!(
        fs::symlink_metadata(binary)?.file_type().is_symlink(),
        "Automatic upgrades require a versioned prebuilt installation. Run nyxid update first, then enable automatic upgrades"
    );
    let versions = super::installed_versions_in(root, Some(binary))?;
    versions.into_iter().find(|v| v.active).map(|v| v.tag)
        .context("Active binary is outside this versioned installation; reinstall using the documented terminal installer")
}

pub async fn run(command: AutoUpdateCommands) -> Result<()> {
    let root = root()?;
    if let AutoUpdateCommands::Status { json } = command {
        return show_status(&root, json).await;
    }
    let _lock = lock_in(&root)?;
    let mut policy = match read_in(&root) {
        Ok(policy) => policy,
        Err(_)
            if matches!(
                command,
                AutoUpdateCommands::Disable | AutoUpdateCommands::Enable { .. }
            ) =>
        {
            Policy::default()
        }
        Err(error) => return Err(error),
    };
    match command {
        AutoUpdateCommands::Enable { interval_hours } => {
            let binary = super::active_binary_path()?;
            let binary = if binary.is_absolute() {
                binary
            } else {
                std::env::current_dir()?.join(binary)
            };
            active_version(&root, &binary)?;
            anyhow::ensure!(
                (1..=720).contains(&interval_hours),
                "Interval must be between 1 and 720 hours"
            );
            policy.enabled = false;
            policy.interval_hours = interval_hours;
            policy.active_binary = Some(binary.clone());
            policy.next_eligible_check = Some(Utc::now());
            write_in(&root, &policy)?;
            install_controller(&root, &std::env::current_exe()?)?;
            if let Err(error) = install_scheduler(&root, &binary).await {
                policy.last_result = Some("scheduler_install_failed".into());
                write_in(&root, &policy)?;
                return Err(error);
            }
            policy.enabled = true;
            write_in(&root, &policy)?;
            eprintln!(
                "Automatic verified upgrades enabled every {interval_hours} hours. Running nodes defer adoption until manually restarted."
            );
            eprintln!(
                "After rollback to an older CLI, use: {} update auto status",
                root.join(".update-controller").display()
            );
            if policy.held_version.is_some() {
                eprintln!(
                    "The version hold remains active. Run nyxid update auto resume to clear it."
                );
            }
        }
        AutoUpdateCommands::Disable => {
            policy.enabled = false;
            policy.next_eligible_check = None;
            write_in(&root, &policy)?;
            remove_scheduler().await?;
            eprintln!("Automatic upgrades disabled.");
        }
        AutoUpdateCommands::Hold => {
            let binary = policy
                .active_binary
                .clone()
                .unwrap_or(super::active_binary_path()?);
            policy.held_version = Some(active_version(&root, &binary)?);
            policy.hold_reason = Some("manual".into());
            policy.next_eligible_check = None;
            write_in(&root, &policy)?;
            eprintln!(
                "Automatic upgrades held at {}.",
                policy.held_version.as_deref().unwrap_or("current")
            );
        }
        AutoUpdateCommands::Resume => {
            policy.held_version = None;
            policy.hold_reason = None;
            policy.next_eligible_check = policy.enabled.then(Utc::now);
            write_in(&root, &policy)?;
            eprintln!(
                "Version hold cleared. Automatic upgrades are {}.",
                if policy.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        }
        AutoUpdateCommands::Run => {
            run_due(&root, &mut policy).await?;
        }
        AutoUpdateCommands::Status { .. } => unreachable!(),
    }
    Ok(())
}

fn is_due(policy: &Policy, now: DateTime<Utc>) -> bool {
    policy.enabled
        && policy.held_version.is_none()
        && policy.next_eligible_check.is_none_or(|next| next <= now)
}

async fn run_due(root: &Path, policy: &mut Policy) -> Result<()> {
    let now = Utc::now();
    if !is_due(policy, now) {
        return Ok(());
    }
    policy.last_attempt = Some(now);
    policy.last_result = Some("running".into());
    policy.next_eligible_check =
        Some(now + chrono::Duration::hours(i64::from(policy.interval_hours)));
    write_in(root, policy)?;
    let result = tokio::time::timeout(Duration::from_secs(900), perform_update(root, policy)).await;
    let (status, failure) = match result {
        Ok(Ok(status)) => (status, None),
        Ok(Err(error)) => (failure_status(policy, "failed"), Some(error)),
        Err(_) => (
            failure_status(policy, "timed_out"),
            Some(anyhow::anyhow!("Automatic update timed out")),
        ),
    };
    policy.last_result = Some(status);
    write_in(root, policy)?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(())
}

trait ReleaseSource {
    async fn latest(&self) -> Result<Option<super::GitHubRelease>>;
    async fn verify(&self, archive: &Path, tag: &str) -> Result<()>;
}

struct GitHubSource<'a>(&'a reqwest::Client);
impl ReleaseSource for GitHubSource<'_> {
    async fn latest(&self) -> Result<Option<super::GitHubRelease>> {
        super::resolve_release(self.0, None).await
    }
    async fn verify(&self, archive: &Path, tag: &str) -> Result<()> {
        super::verify_release_attestation(self.0, archive, tag).await
    }
}

async fn perform_update(root: &Path, policy: &mut Policy) -> Result<String> {
    let client = super::github_client()?;
    perform_update_using(root, policy, &client, &GitHubSource(&client)).await
}

async fn perform_update_using(
    root: &Path,
    policy: &mut Policy,
    client: &reqwest::Client,
    source: &impl ReleaseSource,
) -> Result<String> {
    let binary = policy
        .active_binary
        .clone()
        .context("Automatic updater has no configured active binary; enable it again")?;
    let installed = active_version(root, &binary)?;
    reconcile_activation(root, policy, &installed)?;
    #[cfg(unix)]
    if policy.last_activated_version.as_deref() == Some(&installed) && pending_phases(policy) {
        let versioned = fs::canonicalize(&binary)?;
        finish_installation(root, policy, &versioned, &installed, &None).await?;
        return Ok(format!(
            "installed {installed}; pending phases completed; nodes deferred"
        ));
    }
    let release = source
        .latest()
        .await?
        .context("No stable prebuilt release is available; retry later")?;
    let tag = super::normalize_release_tag(&release.tag_name)?;
    anyhow::ensure!(
        !tag.split('+').next().unwrap_or(&tag).contains('-'),
        "Automatic upgrades only install stable releases"
    );
    if super::compare_release_tags(&tag, &installed)? != std::cmp::Ordering::Greater {
        return Ok("already_current".into());
    }
    let asset_name = super::asset_name_for_target(super::current_target())?;
    let asset = super::release_asset(&release, &asset_name)
        .context("No supported prebuilt asset; automatic upgrades will not compile from source")?;
    let download = tempfile::tempdir()?;
    let archive = download.path().join(&asset_name);
    super::download_asset(client, asset, &archive).await?;
    source
        .verify(&archive, &tag)
        .await
        .context("Release verification failed; existing binary retained")?;
    #[cfg(unix)]
    {
        let versioned = super::extract_binary_to_version_root(&archive, &tag, root)?;
        // Intent survives a crash on either side of the symlink switch.
        policy.activation_intent = Some(tag.clone());
        write_in(root, policy)?;
        super::retarget_active_symlink(&binary, &versioned)?;
        super::retarget_secondary_symlinks(root, &versioned, &binary);
        reconcile_activation(root, policy, &tag)?;
        finish_installation(root, policy, &versioned, &tag, &None).await?;
        Ok(format!("installed {tag}; skills refreshed; nodes deferred"))
    }
    #[cfg(not(unix))]
    anyhow::bail!("Automatic upgrades require macOS or Linux")
}

fn reconcile_activation(root: &Path, policy: &mut Policy, installed: &str) -> Result<()> {
    if let Some(tag) = policy.activation_intent.take() {
        if tag == installed {
            policy.pending_skills_version = Some(tag.clone());
            policy.pending_controller_version = Some(tag.clone());
            policy.last_activated_version = Some(tag.clone());
            policy.retention_result = Some("pending".into());
            policy.last_result = Some(format!(
                "installed {tag}; completion pending; nodes deferred"
            ));
        } else {
            policy.last_result = Some(format!(
                "activation of {tag} did not complete; active version is {installed}"
            ));
        }
        write_in(root, policy)?;
    }
    Ok(())
}

fn pending_phases(policy: &Policy) -> bool {
    policy.pending_skills_version.is_some()
        || policy.pending_controller_version.is_some()
        || policy
            .retention_result
            .as_deref()
            .is_some_and(|result| result != "complete")
}

fn failure_status(policy: &Policy, failure: &str) -> String {
    if let Some(version) = &policy.last_activated_version {
        format!(
            "{failure}; last activated {version}; controller {}; skills {}; retention {}",
            if policy.pending_controller_version.is_some() {
                "pending"
            } else {
                "complete"
            },
            if policy.pending_skills_version.is_some() {
                "pending"
            } else {
                "complete"
            },
            policy
                .retention_result
                .as_deref()
                .unwrap_or("not attempted")
        )
    } else {
        failure.into()
    }
}

#[cfg(unix)]
async fn finish_installation(
    root: &Path,
    policy: &mut Policy,
    binary: &Path,
    tag: &str,
    base_url: &Option<String>,
) -> Result<()> {
    let mut failures = Vec::new();
    if policy.pending_controller_version.as_deref() == Some(tag) {
        match install_controller(root, binary) {
            Ok(()) => policy.pending_controller_version = None,
            Err(error) => failures.push(format!("controller refresh: {error}")),
        }
        write_in(root, policy)?;
    }
    if policy.retention_result.as_deref() != Some("complete") {
        match super::cleanup_old_versions(root, Some(binary), super::RETAINED_VERSION_COUNT) {
            Ok(()) => policy.retention_result = Some("complete".into()),
            Err(error) => {
                policy.retention_result = Some("failed; retry pending".into());
                failures.push(format!("retention cleanup: {error}"));
            }
        }
        write_in(root, policy)?;
    }
    if policy.pending_skills_version.as_deref() == Some(tag) {
        match super::exec_skills_update(binary, base_url).await {
            Ok(()) => policy.pending_skills_version = None,
            Err(error) => failures.push(format!("skills refresh: {error}")),
        }
        write_in(root, policy)?;
    }
    anyhow::ensure!(
        failures.is_empty(),
        "Release {tag} is activated; unfinished phases will retry: {}",
        failures.join("; ")
    );
    Ok(())
}

async fn show_status(root: &Path, json: bool) -> Result<()> {
    let policy = read_in(root)?;
    let scheduler = scheduler_loaded().await;
    let binary = policy
        .active_binary
        .clone()
        .unwrap_or(super::active_binary_path()?);
    let installed = active_version(root, &binary).ok();
    let effective = if !policy.enabled {
        "disabled"
    } else if policy.held_version.is_some() {
        "held"
    } else if !scheduler {
        "scheduler_unavailable"
    } else if installed.is_none() {
        "unsupported_install"
    } else {
        "enabled"
    };
    let next = if effective == "enabled" {
        policy.next_eligible_check
    } else {
        None
    };
    let status = serde_json::json!({"policy": policy, "effective_policy": effective,
        "installed_version": installed, "scheduler_loaded": scheduler,
        "next_eligible_check": next, "scheduler_poll_seconds": TICK_SECONDS,
        "scheduling": "A due check runs within 60 seconds while the user scheduler is active; sleeping or logged-out hosts defer it"});
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!(
            "Automatic upgrades: {effective}\nInstalled: {}\nInterval: {} hours\nLast attempt: {}\nLast result: {}\nNext eligible check: {}\nNode policy: defer until manual restart",
            installed.as_deref().unwrap_or("unsupported installation"),
            policy.interval_hours,
            policy
                .last_attempt
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "never".into()),
            policy.last_result.as_deref().unwrap_or("none"),
            next.map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "none".into())
        );
        println!("Scheduler checks eligibility every 60 seconds while the user session is active.");
        if let Some(version) = &policy.held_version {
            println!(
                "Held version: {version}\nHold reason: {}",
                policy.hold_reason.as_deref().unwrap_or("manual")
            );
        }
        if let Some(version) = &policy.last_activated_version {
            println!("Last activated version: {version}");
        }
        if let Some(version) = &policy.pending_skills_version {
            println!("Skills refresh pending: {version}");
        }
        if let Some(version) = &policy.pending_controller_version {
            println!("Controller refresh pending: {version}");
        }
        if let Some(retention) = &policy.retention_result {
            println!("Retention: {retention}");
        }
    }
    Ok(())
}

fn scheduler_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot locate home directory")?;
    if cfg!(target_os = "macos") {
        Ok(home.join("Library/LaunchAgents"))
    } else if cfg!(target_os = "linux") {
        Ok(std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("systemd/user"))
    } else {
        anyhow::bail!("Automatic scheduling requires macOS launchd or Linux systemd")
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn unit_quote(value: &str, exec: bool) -> String {
    let value = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!(
        "\"{}\"",
        if exec {
            value.replace('$', "$$")
        } else {
            value
        }
    )
}

fn launchd_plist(root: &Path, binary: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{controller}</string><string>update</string><string>auto</string><string>run</string></array>
<key>StartInterval</key><integer>{TICK_SECONDS}</integer><key>RunAtLoad</key><true/>
<key>ProcessType</key><string>Background</string>
<key>EnvironmentVariables</key><dict><key>NYXID_INSTALL_ROOT</key><string>{root}</string><key>NYXID_ACTIVE_SYMLINK</key><string>{binary}</string><key>NYXID_NO_UPDATE_CHECK</key><string>1</string><key>NYXID_TELEMETRY</key><string>0</string></dict>
</dict></plist>
"#,
        binary = xml(&binary.to_string_lossy()),
        root = xml(&root.to_string_lossy()),
        controller = xml(&root.join(".update-controller").to_string_lossy())
    )
}

fn systemd_service(root: &Path, binary: &Path) -> String {
    format!(
        "[Unit]\nDescription=NyxID verified automatic upgrade\n[Service]\nType=oneshot\nExecStart={} update auto run\nEnvironment={} {} NYXID_NO_UPDATE_CHECK=1 NYXID_TELEMETRY=0\nTimeoutStartSec=16min\nKillMode=control-group\n",
        unit_quote(&root.join(".update-controller").to_string_lossy(), true),
        unit_quote(&format!("NYXID_INSTALL_ROOT={}", root.display()), false),
        unit_quote(&format!("NYXID_ACTIVE_SYMLINK={}", binary.display()), false)
    )
}

async fn control(program: &str, args: &[&str]) -> Result<bool> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().with_context(|| {
        format!("Cannot run {program}; enable a user session scheduler and retry")
    })?;
    Ok(tokio::time::timeout(Duration::from_secs(30), child.wait())
        .await
        .context("User scheduler command timed out")??
        .success())
}

fn launchd_domain() -> String {
    #[cfg(unix)]
    {
        format!("gui/{}", unsafe { libc::getuid() })
    }
    #[cfg(not(unix))]
    {
        String::new()
    }
}

async fn scheduler_loaded() -> bool {
    if cfg!(target_os = "macos") {
        control(
            "launchctl",
            &["print", &format!("{}/{LABEL}", launchd_domain())],
        )
        .await
        .unwrap_or(false)
    } else if cfg!(target_os = "linux") {
        control("systemctl", &["--user", "is-active", "--quiet", TIMER])
            .await
            .unwrap_or(false)
    } else {
        false
    }
}

fn write_schedule(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().context("Invalid scheduler path")?;
    fs::create_dir_all(dir)?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(contents.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

async fn install_scheduler(root: &Path, binary: &Path) -> Result<()> {
    let directory = scheduler_dir()?;
    if cfg!(target_os = "macos") {
        if scheduler_loaded().await {
            anyhow::ensure!(
                control(
                    "launchctl",
                    &["bootout", &format!("{}/{LABEL}", launchd_domain())]
                )
                .await?,
                "Failed to unload previous update scheduler"
            );
        }
        let path = directory.join(format!("{LABEL}.plist"));
        write_schedule(&path, &launchd_plist(root, binary))?;
        anyhow::ensure!(
            control(
                "launchctl",
                &["bootstrap", &launchd_domain(), &path.to_string_lossy()]
            )
            .await?,
            "launchd could not load the updater. Run this command in the signed-in desktop user session"
        );
    } else {
        write_schedule(&directory.join(SERVICE), &systemd_service(root, binary))?;
        write_schedule(
            &directory.join(TIMER),
            "[Unit]\nDescription=NyxID update eligibility check\n[Timer]\nOnCalendar=*-*-* *:*:00\nPersistent=true\nAccuracySec=1s\n[Install]\nWantedBy=timers.target\n",
        )?;
        anyhow::ensure!(
            control("systemctl", &["--user", "daemon-reload"]).await?,
            "systemd user manager unavailable"
        );
        anyhow::ensure!(
            control("systemctl", &["--user", "enable", "--now", TIMER]).await?,
            "Cannot enable update timer. Start a systemd user session; for logged-out servers enable lingering for this user"
        );
    }
    Ok(())
}

async fn remove_scheduler() -> Result<()> {
    let directory = scheduler_dir()?;
    if cfg!(target_os = "macos") {
        if scheduler_loaded().await {
            anyhow::ensure!(
                control(
                    "launchctl",
                    &["bootout", &format!("{}/{LABEL}", launchd_domain())]
                )
                .await?,
                "Policy is disabled but the launchd job could not be unloaded"
            );
        }
        remove_if_exists(&directory.join(format!("{LABEL}.plist")))?;
    } else {
        if directory.join(TIMER).exists() {
            anyhow::ensure!(
                control("systemctl", &["--user", "disable", "--now", TIMER]).await?,
                "Policy is disabled but the systemd timer could not be removed"
            );
        }
        remove_if_exists(&directory.join(TIMER))?;
        remove_if_exists(&directory.join(SERVICE))?;
        anyhow::ensure!(
            control("systemctl", &["--user", "daemon-reload"]).await?,
            "Policy is disabled; reload the systemd user manager when available"
        );
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn install_controller(root: &Path, source: &Path) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    let mut binary = fs::File::open(source)?;
    std::io::copy(&mut binary, &mut temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(root.join(".update-controller"))?;
    Ok(())
}
