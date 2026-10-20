//! Explicit maintenance boundary for binaries that predate runtime leases.
//! Acknowledgement is fresh per invocation: it is never a durable assertion
//! that an old executable cannot be restarted by an external supervisor.
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

use super::processes;
use crate::node::error::{Error, Result};
use crate::node::storage::Directory;

const WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(50);

pub(super) fn refuse(reason: &str) -> Error {
    Error::Config(format!(
        "node_migration_{reason}: stop all node runtimes and disable external supervisors; retain custody and recovery files, then retry with the original profile/config"
    ))
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Cutover<'a> {
    pub(crate) profile: Option<&'a str>,
    pub(crate) confirm_legacy_cutover: bool,
}

struct State {
    stopped: bool,
    pids: Vec<u32>,
}
trait Environment {
    fn status(&mut self) -> Result<State>;
    fn stop(&mut self) -> Result<()>;
    fn processes_stopped(&mut self) -> Result<()>;
    fn alive(&mut self, pid: u32) -> Result<bool>;
}

/// Retained across the whole migration, including uncertain publication/errors.
/// Revalidation precedes every mutation phase; new runtimes cannot start while
/// this exclusive lease is held. Legacy restart prevention is the explicit
/// operator maintenance boundary, not something this new lock can establish.
pub(crate) struct Quiescence {
    _lease: File,
    directory: Directory,
    config_dir: PathBuf,
    environment: Native,
}
impl Quiescence {
    pub(crate) fn check(&mut self) -> Result<()> {
        self.directory.check_path(&self.config_dir)?;
        if !self.environment.status()?.stopped {
            return Err(refuse("service_restarted"));
        }
        self.environment.processes_stopped()
    }
}

pub(crate) fn require_migration_quiescence(
    directory: &Directory,
    config_dir: &Path,
    options: Cutover<'_>,
) -> Result<Quiescence> {
    if !options.confirm_legacy_cutover {
        return Err(Error::Config("node_migration_quiescence_required: first stop foreground nodes and disable every external supervisor, including containers and nodes under other OS identities; keep them stopped throughout migration and do not restart old binaries afterward. Pass --confirm-legacy-cutover to acknowledge this maintenance boundary; the CLI then stops and verifies the selected launchd/systemd service and checks OS processes".into()));
    }
    directory.check_path(config_dir)?;
    let mut environment = Native::new(directory, options.profile)?;
    stop_and_wait(&mut environment, WAIT)?;
    let lease = directory.runtime_lease(true)?;
    // Close startup/lease races: inventory again *after* excluding new agents.
    if !environment.status()?.stopped {
        return Err(refuse("service_restarted"));
    }
    environment.processes_stopped()?;
    directory.check_path(config_dir)?;
    Ok(Quiescence {
        _lease: lease,
        directory: directory.try_clone()?,
        config_dir: config_dir.to_owned(),
        environment,
    })
}

fn stop_and_wait(environment: &mut impl Environment, budget: Duration) -> Result<()> {
    let initial = environment.status()?;
    if !initial.stopped {
        environment.stop()?;
    }
    let deadline = Instant::now() + budget;
    loop {
        let state = environment.status()?;
        let mut any_alive = false;
        for pid in &initial.pids {
            any_alive |= environment.alive(*pid)?;
        }
        if state.stopped && !any_alive {
            environment.processes_stopped()?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(refuse("stop_timeout"));
        }
        std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

#[derive(Clone, Copy)]
enum Platform {
    Launchd,
    Systemd,
}
struct Native {
    platform: Platform,
    program: PathBuf,
    target: String,
    domain: String,
    definition: Option<(PathBuf, Zeroizing<Vec<u8>>)>,
    recorded_pid: Option<u32>,
}
impl Native {
    fn new(directory: &Directory, profile: Option<&str>) -> Result<Self> {
        let (platform, program, target, domain, path) = if cfg!(target_os = "macos") {
            (
                Platform::Launchd,
                PathBuf::from("/bin/launchctl"),
                super::launchd_target_for(profile)?,
                super::launchd_domain(),
                super::plist_path_for(profile)?,
            )
        } else if cfg!(target_os = "linux") {
            (
                Platform::Systemd,
                PathBuf::from("/usr/bin/systemctl"),
                super::systemd_unit(profile)?,
                String::new(),
                super::unit_path_for(profile)?,
            )
        } else {
            return Err(refuse("platform_unsupported"));
        };
        let definition = match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                use std::os::unix::fs::MetadataExt;
                if !meta.is_file()
                    || meta.uid() != unsafe { libc::geteuid() }
                    || meta.mode() & 0o022 != 0
                {
                    return Err(refuse("service_definition_invalid"));
                }
                // Read the protected definition without following final links.
                let bytes = read_definition(&path)?;
                let arguments = match platform {
                    Platform::Launchd => {
                        let value = plist::Value::from_reader_xml(bytes.as_slice())
                            .map_err(|_| refuse("service_definition_invalid"))?;
                        let args = value
                            .as_dictionary()
                            .and_then(|v| v.get("ProgramArguments"))
                            .and_then(plist::Value::as_array)
                            .ok_or_else(|| refuse("service_definition_invalid"))?;
                        args.iter()
                            .map(|s| {
                                s.as_string()
                                    .map(str::to_owned)
                                    .ok_or_else(|| refuse("service_definition_invalid"))
                            })
                            .collect::<Result<Vec<_>>>()?
                    }
                    Platform::Systemd => {
                        let text = std::str::from_utf8(&bytes)
                            .map_err(|_| refuse("service_definition_invalid"))?;
                        super::systemd_pinned_binary(text)
                            .map_err(|_| refuse("service_definition_invalid"))?;
                        let line = text
                            .lines()
                            .find_map(|s| s.trim().strip_prefix("ExecStart="))
                            .ok_or_else(|| refuse("service_definition_invalid"))?;
                        shlex::split(line).ok_or_else(|| refuse("service_definition_invalid"))?
                    }
                };
                validate_arguments(directory, &arguments)?;
                Some((path, bytes))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(refuse("service_definition_invalid")),
        };
        let recorded_pid = match directory.read(std::ffi::OsStr::new("nyxid-node.pid"), 32) {
            Ok(bytes) => {
                let pid = std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .filter(|v| *v > 0 && *v <= i32::MAX as u32)
                    .ok_or_else(|| refuse("pid_file_invalid"))?;
                Some(pid)
            }
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(refuse("pid_file_invalid")),
        };
        Ok(Self {
            platform,
            program,
            target,
            domain,
            definition,
            recorded_pid,
        })
    }
    fn call(&self, args: &[&str]) -> Result<Output> {
        call(&self.program, args, WAIT)
    }
}

fn read_definition(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let directory = Directory::open(
        path.parent()
            .ok_or_else(|| refuse("service_definition_invalid"))?,
    )?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| refuse("service_definition_invalid"))?;
    let meta = file
        .metadata()
        .map_err(|_| refuse("service_definition_invalid"))?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o022 != 0
    {
        return Err(refuse("service_definition_invalid"));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| refuse("service_definition_invalid"))?;
    if bytes.len() > 65536 {
        return Err(refuse("service_definition_invalid"));
    }
    directory.check_path(path.parent().unwrap())?;
    Ok(bytes)
}

fn validate_arguments(directory: &Directory, arguments: &[String]) -> Result<()> {
    if arguments.len() < 5
        || !Path::new(&arguments[0]).is_absolute()
        || arguments[1..3] != ["node", "start"]
    {
        return Err(refuse("service_definition_invalid"));
    }
    let mut config = None;
    let (options, remainder) = arguments[3..].as_chunks::<2>();
    for pair in options {
        match pair[0].as_str() {
            "--config" if config.is_none() => config = Some(Path::new(&pair[1])),
            "--log-level"
                if ["trace", "debug", "info", "warn", "error"].contains(&pair[1].as_str()) => {}
            _ => return Err(refuse("service_definition_invalid")),
        }
    }
    if !remainder.is_empty() {
        return Err(refuse("service_definition_invalid"));
    }
    directory
        .check_path(
            config
                .filter(|p| p.is_absolute())
                .ok_or_else(|| refuse("service_definition_invalid"))?,
        )
        .map_err(|_| refuse("service_config_mismatch"))
}

struct Output {
    success: bool,
    code: Option<i32>,
    stdout: Zeroizing<Vec<u8>>,
    stderr: Zeroizing<Vec<u8>>,
}
fn call(program: &Path, args: &[&str], budget: Duration) -> Result<Output> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    struct Capture<R> {
        pipe: R,
        bytes: Zeroizing<Vec<u8>>,
        closed: bool,
    }
    impl<R: Read + AsRawFd> Capture<R> {
        fn new(pipe: R) -> Result<Self> {
            let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                    < 0
            {
                return Err(refuse("service_manager_unavailable"));
            }
            Ok(Self {
                pipe,
                bytes: Zeroizing::new(Vec::new()),
                closed: false,
            })
        }
        fn drain(&mut self) -> Result<()> {
            let mut buffer = Zeroizing::new([0u8; 8192]);
            // Bound work even if a broken helper continuously writes output.
            for _ in 0..16 {
                match self.pipe.read(buffer.as_mut()) {
                    Ok(0) => {
                        self.closed = true;
                        break;
                    }
                    Ok(count) => {
                        if self.bytes.len() + count > 4 * 1024 * 1024 {
                            return Err(refuse("service_manager_invalid"));
                        }
                        self.bytes.extend_from_slice(&buffer[..count]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(refuse("service_manager_unavailable")),
                }
            }
            Ok(())
        }
    }
    struct OwnedCommand {
        child: std::process::Child,
        armed: bool,
    }
    impl Drop for OwnedCommand {
        fn drop(&mut self) {
            if self.armed {
                // Only this command's new process group, never a node PID or
                // another lane's process. Also closes inherited pipe handles.
                unsafe {
                    libc::kill(-(self.child.id() as i32), libc::SIGKILL);
                }
                let _ = self.child.wait();
            }
        }
    }
    let child = Command::new(program)
        .process_group(0)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| refuse("service_manager_unavailable"))?;
    let mut command = OwnedCommand { child, armed: true };
    let mut stdout = Capture::new(command.child.stdout.take().unwrap())?;
    let mut stderr = Capture::new(command.child.stderr.take().unwrap())?;
    let deadline = Instant::now() + budget;
    let mut status = None;
    loop {
        stdout.drain()?;
        stderr.drain()?;
        if status.is_none() {
            status = command
                .child
                .try_wait()
                .map_err(|_| refuse("service_manager_unavailable"))?;
        }
        if let Some(status) = status
            && stdout.closed
            && stderr.closed
        {
            command.armed = false;
            return Ok(Output {
                success: status.success(),
                code: status.code(),
                stdout: stdout.bytes,
                stderr: stderr.bytes,
            });
        }
        // Deadline includes EOF: a helper that exits but leaves a child holding
        // stdout/stderr cannot turn a bounded status check into an infinite join.
        if Instant::now() >= deadline {
            return Err(refuse("service_manager_timeout"));
        }
        std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

impl Environment for Native {
    fn status(&mut self) -> Result<State> {
        let mut state = self.manager_status()?;
        if let Some(pid) = self.recorded_pid {
            state.pids.push(pid);
        }
        Ok(state)
    }
    fn stop(&mut self) -> Result<()> {
        let args = match self.platform {
            Platform::Launchd => vec!["bootout", self.target.as_str()],
            Platform::Systemd => vec!["--user", "stop", self.target.as_str()],
        };
        if !self.call(&args)?.success {
            return Err(refuse("service_stop_failed"));
        }
        Ok(())
    }
    fn processes_stopped(&mut self) -> Result<()> {
        if let Some(pid) = self.recorded_pid
            && processes::exists(pid)?
        {
            return Err(refuse("node_process_running"));
        }
        processes::require_no_runtimes()
    }
    fn alive(&mut self, pid: u32) -> Result<bool> {
        processes::exists(pid)
    }
}
impl Native {
    fn manager_status(&self) -> Result<State> {
        if let Some((path, original)) = &self.definition
            && read_definition(path)?.as_slice() != original.as_slice()
        {
            return Err(refuse("service_definition_changed"));
        }
        match self.platform {
            Platform::Launchd => {
                // First prove the user domain is reachable. An arbitrary error
                // from `print target` is never interpreted as stopped.
                if !self.call(&["print", &self.domain])?.success {
                    return Err(refuse("service_manager_unavailable"));
                }
                let output = self.call(&["print", &self.target])?;
                if !output.success {
                    let error = std::str::from_utf8(&output.stderr)
                        .map_err(|_| refuse("service_manager_invalid"))?;
                    let label = self.target.rsplit('/').next().unwrap();
                    if output.code == Some(113)
                        && error.contains(&format!("Could not find service \"{label}\" in domain"))
                    {
                        return Ok(State {
                            stopped: true,
                            pids: vec![],
                        });
                    }
                    return Err(refuse("service_manager_unavailable"));
                }
                if self.definition.is_none() {
                    return Err(refuse("service_definition_missing"));
                }
                let text = std::str::from_utf8(&output.stdout)
                    .map_err(|_| refuse("service_manager_invalid"))?;
                let pids = text
                    .lines()
                    .filter_map(|s| s.trim().strip_prefix("pid = "))
                    .map(|s| {
                        s.parse::<u32>()
                            .map_err(|_| refuse("service_manager_invalid"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if pids.iter().any(|p| *p == 0 || *p > i32::MAX as u32) {
                    return Err(refuse("service_manager_invalid"));
                }
                Ok(State {
                    stopped: false,
                    pids,
                })
            }
            Platform::Systemd => {
                let output = self.call(&[
                    "--user",
                    "show",
                    &self.target,
                    "--property=LoadState,ActiveState,SubState,MainPID,ControlPID,FragmentPath",
                ])?;
                let text = std::str::from_utf8(&output.stdout)
                    .map_err(|_| refuse("service_manager_invalid"))?;
                let values = properties(text)?;
                let get = |name| {
                    values
                        .get(name)
                        .copied()
                        .ok_or_else(|| refuse("service_manager_invalid"))
                };
                let loaded = get("LoadState")?;
                let absent = output.code == Some(1) && loaded == "not-found";
                if !(output.success || absent) || !matches!(loaded, "loaded" | "not-found") {
                    return Err(refuse("service_manager_unavailable"));
                }
                let active = get("ActiveState")?;
                let sub = get("SubState")?;
                if !matches!(
                    (active, sub),
                    ("active", "running" | "exited" | "listening")
                        | ("inactive", "dead")
                        | ("failed", "failed")
                        | (
                            "activating",
                            "start-pre"
                                | "start"
                                | "start-post"
                                | "auto-restart"
                                | "auto-restart-queued"
                                | "condition"
                        )
                        | (
                            "deactivating",
                            "stop"
                                | "stop-sigterm"
                                | "stop-sigkill"
                                | "stop-post"
                                | "final-sigterm"
                                | "final-sigkill"
                        )
                        | ("reloading", "reload" | "reload-signal" | "reload-notify")
                ) {
                    return Err(refuse("service_manager_invalid"));
                }
                let main = get("MainPID")?
                    .parse::<u32>()
                    .map_err(|_| refuse("service_manager_invalid"))?;
                let control = get("ControlPID")?
                    .parse::<u32>()
                    .map_err(|_| refuse("service_manager_invalid"))?;
                let fragment = get("FragmentPath")?;
                if loaded == "loaded" {
                    let (expected, _) = self
                        .definition
                        .as_ref()
                        .ok_or_else(|| refuse("service_definition_missing"))?;
                    if Path::new(fragment) != expected {
                        return Err(refuse("service_definition_mismatch"));
                    }
                } else if !fragment.is_empty()
                    || main != 0
                    || control != 0
                    || active != "inactive"
                    || sub != "dead"
                {
                    return Err(refuse("service_manager_invalid"));
                }
                if main > i32::MAX as u32 || control > i32::MAX as u32 {
                    return Err(refuse("service_manager_invalid"));
                }
                Ok(State {
                    stopped: matches!((active, sub), ("inactive", "dead") | ("failed", "failed"))
                        && main == 0
                        && control == 0,
                    pids: [main, control].into_iter().filter(|v| *v > 0).collect(),
                })
            }
        }
    }
}

fn properties(text: &str) -> Result<std::collections::BTreeMap<&str, &str>> {
    let mut result = std::collections::BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| refuse("service_manager_invalid"))?;
        if result.insert(key, value).is_some() {
            return Err(refuse("service_manager_invalid"));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::os::unix::fs::PermissionsExt;

    fn stopped() -> State {
        State {
            stopped: true,
            pids: vec![],
        }
    }
    fn running() -> State {
        State {
            stopped: false,
            pids: vec![123],
        }
    }
    struct Fake {
        states: VecDeque<State>,
        alive: VecDeque<bool>,
        calls: Vec<&'static str>,
        stop_error: bool,
        inventory_error: bool,
    }
    impl Fake {
        fn new(states: Vec<State>) -> Self {
            Self {
                states: states.into(),
                alive: vec![false].into(),
                calls: vec![],
                stop_error: false,
                inventory_error: false,
            }
        }
    }
    impl Environment for Fake {
        fn status(&mut self) -> Result<State> {
            self.calls.push("status");
            Ok(if self.states.len() > 1 {
                self.states.pop_front().unwrap()
            } else {
                let state = self.states.front().unwrap();
                State {
                    stopped: state.stopped,
                    pids: state.pids.clone(),
                }
            })
        }
        fn stop(&mut self) -> Result<()> {
            self.calls.push("stop");
            if self.stop_error {
                Err(refuse("service_stop_failed"))
            } else {
                Ok(())
            }
        }
        fn processes_stopped(&mut self) -> Result<()> {
            self.calls.push("inventory");
            if self.inventory_error {
                Err(refuse("node_process_running"))
            } else {
                Ok(())
            }
        }
        fn alive(&mut self, _: u32) -> Result<bool> {
            self.calls.push("pid");
            Ok(if self.alive.len() > 1 {
                self.alive.pop_front().unwrap()
            } else {
                *self.alive.front().unwrap()
            })
        }
    }
    #[test]
    fn waits_for_both_manager_and_old_pid_then_inventories_all_processes() {
        let mut fake = Fake::new(vec![running(), running(), stopped(), stopped()]);
        fake.alive = vec![true, true, false].into();
        stop_and_wait(&mut fake, Duration::from_secs(1)).unwrap();
        assert_eq!(fake.calls.iter().filter(|c| **c == "stop").count(), 1);
        assert_eq!(fake.calls.iter().filter(|c| **c == "pid").count(), 3);
        assert_eq!(fake.calls.last(), Some(&"inventory"));
    }
    #[test]
    fn stop_failure_live_pid_timeout_and_foreground_process_refuse() {
        let mut fake = Fake::new(vec![running()]);
        fake.stop_error = true;
        assert!(
            stop_and_wait(&mut fake, Duration::ZERO)
                .unwrap_err()
                .to_string()
                .contains("service_stop_failed")
        );
        assert!(!fake.calls.contains(&"inventory"));
        let mut fake = Fake::new(vec![running(), stopped()]);
        fake.alive = vec![true].into();
        assert!(
            stop_and_wait(&mut fake, Duration::ZERO)
                .unwrap_err()
                .to_string()
                .contains("stop_timeout")
        );
        assert!(!fake.calls.contains(&"inventory"));
        let mut fake = Fake::new(vec![stopped()]);
        fake.inventory_error = true;
        assert!(
            stop_and_wait(&mut fake, Duration::ZERO)
                .unwrap_err()
                .to_string()
                .contains("node_process_running")
        );
        assert!(!fake.calls.contains(&"stop"));
    }
    #[test]
    fn missing_acknowledgement_has_no_service_lock_key_or_journal_effect() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Directory::open(temp.path()).unwrap();
        let before = directory.names().unwrap();
        let error = match require_migration_quiescence(&directory, temp.path(), Cutover::default())
        {
            Ok(_) => panic!("legacy boundary was bypassed"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("--confirm-legacy-cutover"));
        assert!(error.contains("external supervisor"));
        assert_eq!(directory.names().unwrap(), before);
    }
    #[test]
    fn runtime_lease_excludes_migration_and_new_readers_until_drop() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Directory::open(temp.path()).unwrap();
        let a = directory.runtime_lease(false).unwrap();
        let b = directory.runtime_lease(false).unwrap();
        assert!(directory.runtime_lease(true).is_err());
        drop(a);
        assert!(directory.runtime_lease(true).is_err());
        drop(b);
        let migration = directory.runtime_lease(true).unwrap();
        assert!(directory.runtime_lease(false).is_err());
        assert!(directory.runtime_lease(true).is_err());
        drop(migration);
        assert!(directory.runtime_lease(false).is_ok());
        assert!(temp.path().join(".nyxid-storage-runtime.lock").exists());
    }
    #[test]
    fn runtime_lock_rejects_symlinks_hardlinks_and_public_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let directory = Directory::open(temp.path()).unwrap();
        let file = temp.path().join(".nyxid-storage-runtime.lock");
        let target = temp.path().join("other");
        std::fs::write(&target, b"not a lease").unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert!(directory.runtime_lease(false).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::hard_link(&target, &file).unwrap();
        assert!(directory.runtime_lease(true).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, b"").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(directory.runtime_lease(true).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"not a lease");
    }
    #[test]
    fn exact_service_config_binding_rejects_other_profiles_and_ambiguous_arguments() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let directory = Directory::open(a.path()).unwrap();
        let args = vec![
            "/path with spaces/nyxid".into(),
            "node".into(),
            "start".into(),
            "--config".into(),
            a.path().display().to_string(),
        ];
        validate_arguments(&directory, &args).unwrap();
        let mut wrong = args.clone();
        wrong[4] = b.path().display().to_string();
        assert!(validate_arguments(&directory, &wrong).is_err());
        let mut duplicate = args.clone();
        duplicate.extend_from_slice(&args[3..]);
        assert!(validate_arguments(&directory, &duplicate).is_err());
        let mut unknown = args.clone();
        unknown.extend(["--profile".into(), "other".into()]);
        assert!(validate_arguments(&directory, &unknown).is_err());
        std::fs::rename(a.path(), b.path().join("moved")).unwrap();
        std::fs::create_dir(a.path()).unwrap();
        assert!(directory.check_path(a.path()).is_err());
        assert!(validate_arguments(&directory, &args).is_err());
    }

    // Actual executable fake: the production command runner and native parsers
    // run without contacting or changing the host's service manager.
    struct ManagerFixture {
        directory: tempfile::TempDir,
        native: Native,
    }
    impl ManagerFixture {
        fn new(platform: Platform) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let unit = root.join("unit.service");
            std::fs::write(&unit, "protected definition").unwrap();
            let script = root.join("manager");
            std::fs::write(&script, r#"#!/bin/sh
base=$(dirname "$0")
printf '%s\n' "$*" >> "$base/calls"
case "$*" in
  'print gui/42') test ! -f "$base/domain-failed"; exit $? ;;
  'print gui/42/dev.nyxid.node.work')
    if test -f "$base/stopped"; then
      cat "$base/error" >&2
      exit 113
    fi
    cat "$base/current"
    exit 0 ;;
  'bootout gui/42/dev.nyxid.node.work'|'--user stop nyxid-node-work.service')
    if test -f "$base/stop-failed"; then echo 'provider-body-SENTINEL' >&2; exit 1; fi
    touch "$base/stopped"
    cp "$base/after" "$base/current"
    exit 0 ;;
  '--user show nyxid-node-work.service --property=LoadState,ActiveState,SubState,MainPID,ControlPID,FragmentPath')
    cat "$base/current"
    if test -f "$base/status-failed"; then echo 'provider-body-SENTINEL' >&2; exit 1; fi
    exit 0 ;;
esac
exit 3
"#).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(
                root.join("error"),
                "Could not find service \"dev.nyxid.node.work\" in domain for user gui: 42\n",
            )
            .unwrap();
            let (target, current, after) = match platform {
                Platform::Launchd => (
                    "gui/42/dev.nyxid.node.work",
                    "state = waiting\n".into(),
                    String::new(),
                ),
                Platform::Systemd => (
                    "nyxid-node-work.service",
                    format!(
                        "LoadState=loaded\nActiveState=active\nSubState=running\nMainPID=0\nControlPID=0\nFragmentPath={}\n",
                        unit.display()
                    ),
                    format!(
                        "LoadState=loaded\nActiveState=inactive\nSubState=dead\nMainPID=0\nControlPID=0\nFragmentPath={}\n",
                        unit.display()
                    ),
                ),
            };
            std::fs::write(root.join("current"), current).unwrap();
            std::fs::write(root.join("after"), after).unwrap();
            let bytes = read_definition(&unit).unwrap();
            let native = Native {
                platform,
                program: script,
                target: target.into(),
                domain: "gui/42".into(),
                definition: Some((unit, bytes)),
                recorded_pid: None,
            };
            Self { directory, native }
        }
    }
    #[test]
    fn fake_launchd_and_systemd_stop_verify_and_reject_manager_errors() {
        for platform in [Platform::Launchd, Platform::Systemd] {
            let mut fixture = ManagerFixture::new(platform);
            assert!(!fixture.native.status().unwrap().stopped);
            fixture.native.stop().unwrap();
            assert!(fixture.native.status().unwrap().stopped);
            let calls = std::fs::read_to_string(fixture.directory.path().join("calls")).unwrap();
            assert!(calls.contains("work"));
            std::fs::write(fixture.directory.path().join("stop-failed"), b"").unwrap();
            let error = fixture.native.stop().unwrap_err().to_string();
            assert!(error.contains("service_stop_failed"));
            assert!(!error.contains("SENTINEL"));
            match platform {
                Platform::Launchd => {
                    std::fs::write(fixture.directory.path().join("domain-failed"), b"").unwrap()
                }
                Platform::Systemd => {
                    std::fs::write(fixture.directory.path().join("status-failed"), b"").unwrap()
                }
            }
            assert!(fixture.native.status().is_err());
        }
    }
    #[test]
    fn fake_launchd_requires_exact_missing_service_and_revalidates_definition() {
        let mut fixture = ManagerFixture::new(Platform::Launchd);
        fixture.native.stop().unwrap();
        std::fs::write(
            fixture.directory.path().join("error"),
            b"other error SENTINEL",
        )
        .unwrap();
        assert!(fixture.native.status().is_err());
        let definition = fixture.native.definition.as_ref().unwrap().0.clone();
        std::fs::write(definition, "changed").unwrap();
        assert!(fixture.native.status().is_err());
    }
    #[test]
    fn fake_systemd_rejects_duplicate_missing_and_mismatched_status() {
        let mut fixture = ManagerFixture::new(Platform::Systemd);
        let original = std::fs::read_to_string(fixture.directory.path().join("current")).unwrap();
        for changed in [original.clone() + "MainPID=0\n", original.replace("ControlPID=0\n", ""), original.replace("FragmentPath=", "FragmentPath=/different"), "LoadState=not-found\nActiveState=active\nSubState=running\nMainPID=3\nControlPID=0\nFragmentPath=\n".into()] {
            std::fs::write(fixture.directory.path().join("current"), changed).unwrap();
            assert!(fixture.native.status().is_err());
        }
    }
    #[test]
    fn restarted_service_refuses_with_exclusive_lease_still_held() {
        let fixture = ManagerFixture::new(Platform::Systemd);
        let directory = Directory::open(fixture.directory.path()).unwrap();
        let lease = directory.runtime_lease(true).unwrap();
        let mut proof = Quiescence {
            _lease: lease,
            directory: directory.try_clone().unwrap(),
            config_dir: fixture.directory.path().to_owned(),
            environment: fixture.native,
        };
        let error = proof.check().unwrap_err().to_string();
        assert!(error.contains("service_restarted"));
        assert!(directory.runtime_lease(false).is_err());
        drop(proof);
        assert!(directory.runtime_lease(false).is_ok());
    }
    #[test]
    fn directory_retarget_refuses_before_another_manager_action() {
        let fixture = ManagerFixture::new(Platform::Systemd);
        let config = tempfile::tempdir().unwrap();
        let relocated = tempfile::tempdir().unwrap();
        let directory = Directory::open(config.path()).unwrap();
        let mut proof = Quiescence {
            _lease: directory.runtime_lease(true).unwrap(),
            directory: directory.try_clone().unwrap(),
            config_dir: config.path().to_owned(),
            environment: fixture.native,
        };
        std::fs::rename(config.path(), relocated.path().join("original")).unwrap();
        std::fs::create_dir(config.path()).unwrap();
        assert!(proof.check().is_err());
        assert!(!fixture.directory.path().join("calls").exists());
        assert!(directory.runtime_lease(false).is_err());
    }
    #[test]
    fn historical_pid_remains_a_barrier_even_when_manager_reports_stopped() {
        let mut fixture = ManagerFixture::new(Platform::Systemd);
        fixture.native.stop().unwrap();
        fixture.native.recorded_pid = Some(std::process::id());
        let state = fixture.native.status().unwrap();
        assert!(state.stopped);
        assert!(state.pids.contains(&std::process::id()));
        assert!(
            fixture
                .native
                .processes_stopped()
                .unwrap_err()
                .to_string()
                .contains("node_process_running")
        );
    }
    #[test]
    fn metadata_definition_allows_normal_read_modes_but_refuses_untrusted_custody() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("unit.service");
        std::fs::write(&path, "non-secret metadata").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_definition(&path).is_ok());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(read_definition(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_definition(&link).is_err());
        std::fs::remove_file(&link).unwrap();
        std::fs::hard_link(&path, &link).unwrap();
        assert!(read_definition(&path).is_err());
    }

    #[test]
    fn manager_deadline_kills_only_its_owned_command_group_and_redacts_output() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("blocked-manager");
        std::fs::write(&script, "#!/bin/sh\necho body-SENTINEL >&2\nsleep 5\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        for body in [
            "#!/bin/sh\necho body-SENTINEL >&2\nsleep 5\n",
            "#!/bin/sh\necho body-SENTINEL >&2\nsleep 5 &\nexit 0\n",
        ] {
            std::fs::write(&script, body).unwrap();
            let start = Instant::now();
            let error = match call(&script, &[], Duration::from_millis(100)) {
                Ok(_) => panic!("manager timeout was ignored"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains("service_manager_timeout"));
            assert!(!error.contains("SENTINEL"));
            assert!(start.elapsed() < Duration::from_secs(2));
        }
    }
}
