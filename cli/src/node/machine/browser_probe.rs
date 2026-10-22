//! Observe the managed browser's real renderer without enabling DevTools.
use super::{
    browser_layout::Layout,
    process::{self, Identity},
};
use anyhow::{Context, Result, ensure};
use nyxid_machine::config::ManagedBrowserConfig;
use std::{
    collections::BTreeSet,
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    path::PathBuf,
    time::{Duration, Instant},
};

// Pending hosted calibration: retain the existing bound until samples from the
// actual CI image establish whether additional cold-start headroom is needed.
const BOUND: Duration = Duration::from_secs(5);

// Only reviewed labels reach diagnostics. No browser output or private paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum Observation {
    #[error("browser_pidfd_unavailable")]
    Pidfd,
    #[error("browser_proc_stat_unreadable")]
    Stat,
    #[error("browser_proc_status_unreadable")]
    Status,
    #[error("browser_proc_status_incomplete")]
    StatusIncomplete,
    #[error("browser_proc_exe_unreadable")]
    Exe,
    #[error("browser_executable_package_mismatch")]
    BrowserExe,
    #[error("browser_proc_uid_map_unreadable")]
    UidMap,
    #[error("browser_proc_gid_map_unreadable")]
    GidMap,
    #[error("browser_proc_identity_map_incomplete")]
    MapIncomplete,
    #[error("browser_proc_network_identity_unreadable")]
    NetworkRead,
    #[error("browser_proc_socket_table_unreadable")]
    Sockets,
    #[error("browser_proc_socket_table_incomplete")]
    SocketsIncomplete,
    #[error("browser_identity_mismatch")]
    BrowserIdentity,
    #[error("browser_renderer_identity_mismatch")]
    RendererIdentity,
    #[error("browser_renderer_process_group_mismatch")]
    RendererGroup,
    #[error("browser_renderer_seccomp_mode_unconfirmed")]
    SeccompMode,
    #[error("browser_renderer_additional_filter_unconfirmed")]
    SeccompFilter,
    #[error("browser_renderer_pid_nesting_unconfirmed")]
    PidNesting,
    #[error("browser_renderer_user_namespace_not_distinct")]
    UserNamespace,
    #[error("browser_renderer_network_namespace_not_distinct")]
    NetworkNamespace,
    #[error("browser_renderer_ancestry_unconfirmed")]
    Ancestry,
    #[error("browser_renderer_live_recheck_failed")]
    RendererChanged,
    #[error("browser_parent_live_recheck_failed")]
    BrowserChanged,
    #[error("browser_renderer_not_observed")]
    NoRenderer,
    #[error("browser_policy_refusal_not_observed")]
    NoPolicyRefusal,
    #[error("browser_policy_devtools_listener_observed")]
    PolicyListener,
}
impl Observation {
    fn unobservable(self) -> bool {
        matches!(
            self,
            Self::Pidfd
                | Self::Stat
                | Self::Status
                | Self::StatusIncomplete
                | Self::Exe
                | Self::UidMap
                | Self::GidMap
                | Self::MapIncomplete
                | Self::NetworkRead
                | Self::Sockets
                | Self::SocketsIncomplete
        )
    }
}
fn failure_reason(error: &anyhow::Error) -> String {
    error.downcast_ref::<Observation>().map_or_else(
        || "browser_probe_unavailable".to_owned(),
        ToString::to_string,
    )
}
fn failure_diagnostic(error: &anyhow::Error) -> String {
    let errno = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>()?.raw_os_error())
        .map_or_else(|| "unknown".to_owned(), |errno| errno.to_string());
    format!(
        "browser_probe_failure reason={} errno={errno}",
        failure_reason(error)
    )
}

struct PinnedProcess(OwnedFd);
impl PinnedProcess {
    fn open(pid: u32) -> Result<Self> {
        // No ptrace capability is required. The handle binds liveness to this
        // process, rather than a numeric PID that might later be recycled.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context(Observation::Pidfd);
        }
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
    }
    fn live(&self) -> Result<bool> {
        let mut descriptor = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if ready < 0 {
            return Err(std::io::Error::last_os_error()).context(Observation::Pidfd);
        }
        Ok(ready == 0)
    }
}

fn numbers<const N: usize>(text: &str) -> Option<[u32; N]> {
    text.split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?
        .try_into()
        .ok()
}
fn stat(text: &str) -> Option<(u32, u32, u64)> {
    let fields: Vec<_> = text.rsplit_once(") ")?.1.split_whitespace().collect();
    Some((
        fields.get(1)?.parse().ok()?,
        fields.get(2)?.parse().ok()?,
        fields.get(19)?.parse().ok()?,
    ))
}
fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let mut found = text.lines().filter_map(|line| line.strip_prefix(name));
    let value = found.next()?;
    found.next().is_none().then_some(value.trim())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Link {
    pid: u32,
    parent: u32,
    group: u32,
    start: u64,
}
impl Link {
    fn read_status(pid: u32) -> Result<(Self, String)> {
        let path = PathBuf::from(format!("/proc/{pid}"));
        let before = fs::read_to_string(path.join("stat")).context(Observation::Stat)?;
        let (parent, group, start) = stat(&before).context(Observation::Stat)?;
        let status = fs::read_to_string(path.join("status")).context(Observation::Status)?;
        // PPid is kernel-supplied, not a Name/cmdline claim. Require the status
        // and stat snapshots to agree and retain starttime for every recheck.
        ensure!(
            field(&status, "Pid:").and_then(|s| s.parse().ok()) == Some(pid)
                && field(&status, "PPid:").and_then(|s| s.parse().ok()) == Some(parent),
            Observation::StatusIncomplete
        );
        ensure!(
            stat(&fs::read_to_string(path.join("stat")).context(Observation::Stat)?)
                == Some((parent, group, start)),
            Observation::RendererChanged
        );
        Ok((
            Self {
                pid,
                parent,
                group,
                start,
            },
            status,
        ))
    }
    fn read(pid: u32) -> Result<Self> {
        Self::read_status(pid).map(|(link, _)| link)
    }
    fn unchanged(&self) -> bool {
        Self::read(self.pid).is_ok_and(|now| now == *self)
    }
}

// Canonicalize the kernel's maps, never compare arbitrary whitespace. Missing,
// overlapping, overflowing and partial rows are not namespace evidence.
fn identity_map(text: &str) -> Option<Vec<[u32; 3]>> {
    let mut map: Vec<_> = text.lines().map(numbers::<3>).collect::<Option<_>>()?;
    if map.is_empty() || map.len() > 340 {
        return None;
    }
    for [inside, outside, length] in &map {
        if *length == 0
            || inside.checked_add(*length).is_none()
            || outside.checked_add(*length).is_none()
        {
            return None;
        }
    }
    for dimension in [0, 1] {
        map.sort_unstable_by_key(|row| row[dimension]);
        if map
            .windows(2)
            .any(|rows| rows[0][dimension] + rows[0][2] > rows[1][dimension])
        {
            return None;
        }
    }
    map.sort_unstable();
    Some(map)
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Process {
    link: Link,
    uid: [u32; 4],
    gid: [u32; 4],
    nnp: u32,
    seccomp: u32,
    filters: u32,
    nested: Vec<u32>,
    uid_map: Vec<[u32; 3]>,
    gid_map: Vec<[u32; 3]>,
    network: (u64, u64),
}
impl Process {
    fn read(pid: u32) -> Result<Self> {
        let (link, status) = Link::read_status(pid)?;
        let path = PathBuf::from(format!("/proc/{pid}"));
        let parse = |name| field(&status, name).context(Observation::StatusIncomplete);
        let network = fs::File::open(path.join("net/dev"))
            .context(Observation::NetworkRead)?
            .metadata()
            .context(Observation::NetworkRead)?;
        let process = Self {
            link,
            uid: numbers(parse("Uid:")?).context(Observation::StatusIncomplete)?,
            gid: numbers(parse("Gid:")?).context(Observation::StatusIncomplete)?,
            nnp: parse("NoNewPrivs:")?
                .parse()
                .context(Observation::StatusIncomplete)?,
            seccomp: parse("Seccomp:")?
                .parse()
                .context(Observation::StatusIncomplete)?,
            filters: parse("Seccomp_filters:")?
                .parse()
                .context(Observation::StatusIncomplete)?,
            nested: parse("NSpid:")?
                .split_whitespace()
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()
                .context(Observation::StatusIncomplete)?,
            uid_map: identity_map(
                &fs::read_to_string(path.join("uid_map")).context(Observation::UidMap)?,
            )
            .context(Observation::MapIncomplete)?,
            gid_map: identity_map(
                &fs::read_to_string(path.join("gid_map")).context(Observation::GidMap)?,
            )
            .context(Observation::MapIncomplete)?,
            // net/dev is registered once in each network namespace's proc tree;
            // the generic /proc/PID/net directory has a per-process inode and
            // MUST NOT be compared as network namespace identity.
            network: (network.dev(), network.ino()),
        };
        ensure!(process.link.unchanged(), Observation::RendererChanged);
        Ok(process)
    }
    fn identity(&self, identity: &Identity) -> bool {
        self.uid == [identity.uid; 4] && self.gid == [identity.gid; 4] && self.nnp == 1
    }
    fn sandboxed(
        &self,
        browser: &Self,
        identity: &Identity,
    ) -> std::result::Result<(), Observation> {
        for (confirmed, failure) in [
            (self.identity(identity), Observation::RendererIdentity),
            (browser.identity(identity), Observation::BrowserIdentity),
            (
                self.link.group == browser.link.pid && browser.link.group == browser.link.pid,
                Observation::RendererGroup,
            ),
            (self.seccomp == 2, Observation::SeccompMode),
            (self.filters > browser.filters, Observation::SeccompFilter),
            (
                self.nested.len() > browser.nested.len()
                    && !browser.nested.is_empty()
                    && browser.nested.first() == Some(&browser.link.pid)
                    && self.nested.first() == Some(&self.link.pid)
                    && self
                        .nested
                        .iter()
                        .chain(&browser.nested)
                        .all(|pid| *pid > 0),
                Observation::PidNesting,
            ),
            (
                self.uid_map != browser.uid_map && self.gid_map != browser.gid_map,
                Observation::UserNamespace,
            ),
            (
                self.network != browser.network,
                Observation::NetworkNamespace,
            ),
        ] {
            if !confirmed {
                return Err(failure);
            }
        }
        Ok(())
    }
    fn unchanged(&self) -> bool {
        Self::read(self.link.pid).is_ok_and(|now| now == *self)
    }
}

fn ancestry(
    child: &Link,
    browser: &Link,
    mut read: impl FnMut(u32) -> Result<Link>,
) -> Result<Option<Vec<Link>>> {
    let mut links = vec![child.clone()];
    let mut visited = BTreeSet::from([child.pid]);
    let mut parent = child.parent;
    while parent != browser.pid {
        if parent == 0 || !visited.insert(parent) || links.len() >= 64 {
            return Ok(None);
        }
        let ancestor = read(parent)?;
        if ancestor.pid != parent
            || ancestor.group != browser.pid
            || ancestor.start < browser.start
            || ancestor.start > child.start
        {
            return Ok(None);
        }
        parent = ancestor.parent;
        links.push(ancestor);
    }
    Ok(
        (child.pid != browser.pid && child.group == browser.pid && child.start >= browser.start)
            .then_some(links),
    )
}

// Only the dumpable parent needs a protected exe read. Match filesystem IDs in
// a dedicated short-lived OS thread: never change a Tokio worker's credentials,
// and never add CAP_SYS_PTRACE or read a renderer's memory/fds/exe/ns links.
fn browser_executable(pid: u32, identity: &Identity, layout: Layout) -> Result<PathBuf> {
    let (uid, gid) = (identity.uid, identity.gid);
    let executable = std::thread::spawn(move || -> Result<PathBuf> {
        unsafe {
            libc::syscall(libc::SYS_setfsgid, gid);
            libc::syscall(libc::SYS_setfsuid, uid);
            ensure!(
                libc::syscall(libc::SYS_setfsuid, u32::MAX) == uid as libc::c_long
                    && libc::syscall(libc::SYS_setfsgid, u32::MAX) == gid as libc::c_long,
                Observation::Exe
            );
        }
        fs::read_link(format!("/proc/{pid}/exe")).context(Observation::Exe)
    })
    .join()
    .map_err(|_| anyhow::anyhow!(Observation::Exe))??;
    ensure!(
        Layout::detect(&executable).is_ok_and(|observed| observed == layout),
        Observation::BrowserExe
    );
    Ok(executable)
}
fn remember(last: &mut Observation, reason: Observation) {
    if !last.unobservable() || reason.unobservable() {
        *last = reason;
    }
}
fn remember_candidate_failure(last: &mut Observation, error: &anyhow::Error) {
    // A child can exit between any two proc reads. That is an incomplete live
    // sample, not evidence that the container hides a required observation.
    let exited = error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)))
    });
    remember(
        last,
        if exited {
            Observation::RendererChanged
        } else {
            error
                .downcast_ref::<Observation>()
                .copied()
                .unwrap_or(Observation::StatusIncomplete)
        },
    );
}
fn renderer(browser: &Process, identity: &Identity, last: &mut Observation) -> Result<bool> {
    for entry in fs::read_dir("/proc").context(Observation::Status)? {
        let entry = entry.context(Observation::Status)?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        // A cheap kernel group filter limits proc I/O; ancestry below remains
        // mandatory even for a same-group process with forged browser labels.
        if pid == browser.link.pid
            || unsafe { libc::getpgid(pid as i32) } != browser.link.pid as i32
        {
            continue;
        }
        let link = match Link::read(pid) {
            Ok(link) => link,
            Err(error) => {
                remember_candidate_failure(last, &error);
                continue;
            }
        };
        if link.group != browser.link.pid {
            continue;
        }
        let links = match ancestry(&link, &browser.link, Link::read) {
            Ok(Some(links)) => links,
            Ok(None) => {
                remember(last, Observation::Ancestry);
                continue;
            }
            Err(error) => {
                remember_candidate_failure(last, &error);
                continue;
            }
        };
        // Name and argv are user-controlled. Neither is read to discover a
        // renderer. A candidate must be a live, sandboxed kernel descendant.
        let candidate = match PinnedProcess::open(pid) {
            Ok(candidate) => candidate,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.raw_os_error() == Some(libc::ESRCH)) =>
            {
                remember(last, Observation::RendererChanged);
                continue;
            }
            Err(error) => return Err(error),
        };
        let child = match Process::read(pid) {
            Ok(child) => child,
            Err(error) => {
                remember_candidate_failure(last, &error);
                continue;
            }
        };
        if let Err(reason) = child.sandboxed(browser, identity) {
            remember(last, reason);
            continue;
        }
        if child.link != link
            || !links.iter().all(Link::unchanged)
            || !candidate.live()?
            || !child.unchanged()
        {
            remember(last, Observation::RendererChanged);
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}
fn no_uid_listener(table: &str, uid: u32) -> Result<bool> {
    let mut lines = table.lines();
    let header = lines.next().context(Observation::SocketsIncomplete)?;
    let columns: Vec<_> = header.split_whitespace().collect();
    ensure!(
        columns.len() == 12
            && columns[0] == "sl"
            && columns[1] == "local_address"
            && matches!(columns[2], "rem_address" | "remote_address")
            && columns[3..]
                == [
                    "st", "tx_queue", "rx_queue", "tr", "tm->when", "retrnsmt", "uid", "timeout",
                    "inode"
                ],
        Observation::SocketsIncomplete
    );
    let mut clear = true;
    for row in lines {
        let fields: Vec<_> = row.split_whitespace().collect();
        ensure!(
            fields.len() >= 10 && fields[3].len() == 2,
            Observation::SocketsIncomplete
        );
        let state = u8::from_str_radix(fields[3], 16).context(Observation::SocketsIncomplete)?;
        let owner = fields[7]
            .parse::<u32>()
            .context(Observation::SocketsIncomplete)?;
        if state == 0x0a && owner == uid {
            clear = false;
        }
    }
    Ok(clear)
}
fn no_tcp_listener(pid: u32, uid: u32) -> Result<bool> {
    // The browser namespace's kernel tables include every socket for this UID,
    // including helper-owned listeners. No ptrace-gated fd traversal is needed.
    for file in ["tcp", "tcp6"] {
        let rows =
            fs::read_to_string(format!("/proc/{pid}/net/{file}")).context(Observation::Sockets)?;
        if !no_uid_listener(&rows, uid)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub async fn observe(
    browser: &ManagedBrowserConfig,
    identity: &Identity,
    policy: bool,
) -> Result<()> {
    let result = observe_browser(browser, identity, policy)
        .await
        .map_err(|error| {
            if browser.container
                && error
                    .downcast_ref::<Observation>()
                    .is_some_and(|reason| reason.unobservable())
            {
                error.context(super::separation::BrowserSandboxUnobservableInContainer)
            } else {
                error
            }
        });
    if let Err(error) = &result {
        eprintln!("{}", failure_diagnostic(error));
    }
    result
}

async fn observe_browser(
    browser: &ManagedBrowserConfig,
    identity: &Identity,
    policy: bool,
) -> Result<()> {
    ensure!(identity.uid != 0, "separated_browser_requires_non_root");
    let layout = Layout::detect(&browser.binary)?;
    let profile = tempfile::Builder::new()
        .prefix(".nyxid-sandbox-probe-")
        .tempdir()?;
    let directory = fs::File::open(profile.path())?;
    ensure!(
        unsafe { libc::fchown(directory.as_raw_fd(), identity.uid, identity.gid) } == 0,
        "separated_browser_probe_profile_owner"
    );
    let mut command = tokio::process::Command::new(&browser.binary);
    identity.prepare(&mut command)?;
    command
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .args([
            "--headless=new",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-breakpad",
            "--disable-crash-reporter",
            "--password-store=basic",
        ]);
    if policy {
        command.arg("--remote-debugging-port=0");
    }
    command.arg("about:blank");
    let mut pinned = None;
    let mut refused_at = None;
    let mut last = if policy {
        Observation::NoPolicyRefusal
    } else {
        Observation::NoRenderer
    };
    let result = process::probe_running(command, BOUND, |pid, stderr| {
        if pinned.is_none() {
            pinned = Some(PinnedProcess::open(pid)?);
        }
        let pinned = pinned.as_ref().context(Observation::Pidfd)?;
        ensure!(pinned.live()?, Observation::BrowserChanged);
        let parent = Process::read(pid)?;
        ensure!(parent.identity(identity), Observation::BrowserIdentity);
        let executable = match browser_executable(pid, identity, layout) {
            Ok(executable) => executable,
            Err(error) => {
                // Package launchers can briefly still be a shell before exec.
                // Nothing is accepted until the package browser itself is live.
                remember(
                    &mut last,
                    error
                        .downcast_ref::<Observation>()
                        .copied()
                        .unwrap_or(Observation::BrowserExe),
                );
                return Ok(false);
            }
        };
        let observed = if !policy {
            renderer(&parent, identity, &mut last)?
        } else {
            ensure!(
                !profile.path().join("DevToolsActivePort").exists()
                    && !stderr
                        .windows(b"DevTools listening on".len())
                        .any(|window| window == b"DevTools listening on")
                    && no_tcp_listener(pid, identity.uid)?,
                Observation::PolicyListener
            );
            let refusal = b"DevTools remote debugging is disallowed by the system admin";
            if stderr
                .windows(refusal.len())
                .any(|window| window == refusal)
            {
                refused_at.get_or_insert_with(Instant::now).elapsed() >= Duration::from_millis(200)
            } else {
                false
            }
        };
        if !observed {
            return Ok(false);
        }
        ensure!(
            pinned.live()?
                && parent.unchanged()
                && browser_executable(pid, identity, layout)? == executable,
            Observation::BrowserChanged
        );
        Ok(true)
    })
    .await;
    result.map_err(|error| {
        let reason = error.downcast_ref::<Observation>().copied().unwrap_or(last);
        error.context(reason)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn browser() -> Process {
        Process {
            link: Link {
                pid: 10,
                parent: 1,
                group: 10,
                start: 100,
            },
            uid: [1001; 4],
            gid: [1002; 4],
            nnp: 1,
            seccomp: 2,
            filters: 1,
            nested: vec![10],
            uid_map: vec![[0, 0, u32::MAX]],
            gid_map: vec![[0, 0, u32::MAX]],
            network: (1, 100),
        }
    }
    fn identity() -> Identity {
        Identity {
            uid: 1001,
            gid: 1002,
            name: "browser".into(),
            home: "/home/browser".into(),
            policy_groups: vec![],
            desktop: None,
            sandbox: None,
        }
    }
    fn descendant() -> Process {
        Process {
            link: Link {
                pid: 20,
                parent: 11,
                group: 10,
                start: 102,
            },
            filters: 2,
            nested: vec![20, 12, 1],
            uid_map: vec![[1001, 1001, 1]],
            gid_map: vec![[1002, 1002, 1]],
            network: (1, 101),
            ..browser()
        }
    }
    #[test]
    fn chromium_diagnostics_require_every_active_sandbox_layer() {
        let parent = browser();
        let child = descendant();
        assert!(child.sandboxed(&parent, &identity()).is_ok());
        for index in 0..4 {
            let mut bad = child.clone();
            bad.uid[index] = 0;
            assert_eq!(
                bad.sandboxed(&parent, &identity()),
                Err(Observation::RendererIdentity)
            );
            let mut bad = child.clone();
            bad.gid[index] = 0;
            assert_eq!(
                bad.sandboxed(&parent, &identity()),
                Err(Observation::RendererIdentity)
            );
        }
        for bad in [
            Process {
                nnp: 0,
                ..child.clone()
            },
            Process {
                seccomp: 0,
                ..child.clone()
            },
            Process {
                filters: 1,
                ..child.clone()
            },
            Process {
                nested: vec![20],
                ..child.clone()
            },
            Process {
                nested: vec![20, 0],
                ..child.clone()
            },
            Process {
                nested: vec![21, 1],
                ..child.clone()
            },
            Process {
                link: Link {
                    group: 11,
                    ..child.link.clone()
                },
                ..child.clone()
            },
            Process {
                uid_map: parent.uid_map.clone(),
                ..child.clone()
            },
            Process {
                gid_map: parent.gid_map.clone(),
                ..child.clone()
            },
            Process {
                network: parent.network,
                ..child.clone()
            },
        ] {
            assert!(bad.sandboxed(&parent, &identity()).is_err());
        }
        let mut outer = parent.clone();
        outer.nnp = 0;
        assert_eq!(
            child.sandboxed(&outer, &identity()),
            Err(Observation::BrowserIdentity)
        );
    }
    #[test]
    fn counterfeit_status_never_substitutes_for_kernel_ancestry() {
        let browser = browser();
        let child = descendant();
        // Even all sandbox predicates cannot admit an unrelated fake renderer.
        assert!(child.sandboxed(&browser, &identity()).is_ok());
        let ancestor = Link {
            pid: 11,
            parent: 10,
            group: 10,
            start: 101,
        };
        assert!(
            ancestry(&child.link, &browser.link, |_| Ok(ancestor.clone()))
                .unwrap()
                .is_some()
        );
        for bad in [
            Link {
                parent: 0,
                ..ancestor.clone()
            },
            Link {
                parent: 11,
                ..ancestor.clone()
            },
            Link {
                group: 12,
                ..ancestor.clone()
            },
            Link {
                start: 99,
                ..ancestor.clone()
            },
            Link {
                start: 103,
                ..ancestor.clone()
            },
            Link {
                pid: 12,
                ..ancestor.clone()
            },
        ] {
            assert!(
                ancestry(&child.link, &browser.link, |_| Ok(bad.clone()))
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            ancestry(&child.link, &browser.link, |_| Err(
                Observation::Status.into()
            ))
            .is_err()
        );
    }
    #[test]
    fn real_same_group_fake_renderer_is_not_a_browser_descendant() {
        use std::{
            os::unix::process::CommandExt,
            process::{Child, Command},
        };
        struct Reap(Child);
        impl Drop for Reap {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let browser = Reap(
            Command::new("/bin/sleep")
                .arg("30")
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        let pin = PinnedProcess::open(browser.0.id()).unwrap();
        assert!(pin.live().unwrap());
        let directory = tempfile::tempdir().unwrap();
        let fake = directory.path().join("chromium");
        std::os::unix::fs::symlink("/bin/sleep", &fake).unwrap();
        let fake = Reap(
            Command::new(&fake)
                .arg0("chromium --type=renderer")
                .arg("30")
                .process_group(browser.0.id() as i32)
                .spawn()
                .unwrap(),
        );
        let parent = Link::read(browser.0.id()).unwrap();
        let child = Link::read(fake.0.id()).unwrap();
        assert_eq!(child.group, parent.pid);
        assert_eq!(child.parent, std::process::id());
        assert!(ancestry(&child, &parent, Link::read).unwrap().is_none());
        // spawn() can return before the child's exec completes; until then
        // /proc reports the forking test thread's name, not the executable's.
        let comm = || {
            fs::read_to_string(format!("/proc/{}/comm", fake.0.id()))
                .unwrap()
                .trim()
                .to_owned()
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while comm() != "chromium" && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(comm(), "chromium");
        // The exec'ed process is still not a browser descendant.
        let child = Link::read(fake.0.id()).unwrap();
        assert!(ancestry(&child, &parent, Link::read).unwrap().is_none());
        drop(browser);
        assert!(
            !pin.live().unwrap(),
            "a reaped browser handle must never authorize another PID"
        );
    }
    #[test]
    fn identity_maps_and_socket_tables_fail_closed_on_unknown_or_partial_rows() {
        assert_eq!(identity_map("1001 1001 1\n"), Some(vec![[1001, 1001, 1]]));
        assert_eq!(
            identity_map("0 0 4294967295\n"),
            Some(vec![[0, 0, u32::MAX]])
        );
        for text in [
            "",
            "unknown",
            "1 1",
            "1 1 0",
            "1 1 4294967295",
            "0 0 2\n1 3 1",
            "0 0 1\n2 0 1",
            "1 1 1\n\n",
        ] {
            assert!(identity_map(text).is_none(), "{text:?}");
        }
        let header = "sl local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode\n";
        let row =
            "0: 00000000:1234 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1001 0 42\n";
        assert!(no_uid_listener(header, 1001).unwrap());
        assert!(!no_uid_listener(&format!("{header}{row}"), 1001).unwrap());
        assert!(no_uid_listener(&format!("{header}{row}"), 1002).unwrap());
        assert!(
            no_uid_listener(&format!("{header}{}", row.replace(" 0A ", " 01 ")), 1001).unwrap()
        );
        for table in [
            String::new(),
            "unknown\n".into(),
            format!("{header}partial\n"),
            format!("{header}{}", row.replace(" 1001 ", " unknown ")),
            header.replace("uid timeout", "timeout uid"),
        ] {
            assert!(no_uid_listener(&table, 1001).is_err());
        }
    }
    #[test]
    fn failure_diagnostics_use_fixed_labels_and_preserve_timeout_classification() {
        let error =
            anyhow::anyhow!("private browser output and profile path").context(Observation::Exe);
        assert_eq!(failure_reason(&error), "browser_proc_exe_unreadable");
        assert_eq!(
            failure_diagnostic(&error),
            "browser_probe_failure reason=browser_proc_exe_unreadable errno=unknown"
        );
        let denied = anyhow::Error::new(std::io::Error::from_raw_os_error(libc::EACCES))
            .context(Observation::UidMap);
        assert_eq!(
            failure_diagnostic(&denied),
            "browser_probe_failure reason=browser_proc_uid_map_unreadable errno=13"
        );
        assert_eq!(
            failure_reason(&anyhow::anyhow!("private browser output")),
            "browser_probe_unavailable"
        );
        let timeout = anyhow::Error::new(process::ProbeTimeout).context(Observation::NoRenderer);
        assert!(timeout.is::<process::ProbeTimeout>());
        assert_eq!(failure_reason(&timeout), "browser_renderer_not_observed");
    }
    #[test]
    fn inaccessible_candidates_retain_the_reason_but_exiting_children_do_not() {
        for observation in [Observation::Stat, Observation::Status, Observation::UidMap] {
            let mut last = Observation::NoRenderer;
            let denied = anyhow::Error::new(std::io::Error::from_raw_os_error(libc::EACCES))
                .context(observation);
            remember_candidate_failure(&mut last, &denied);
            assert_eq!(last, observation);
            assert!(last.unobservable());
            // Later process churn must not erase an actual inaccessible read.
            for errno in [libc::ENOENT, libc::ESRCH] {
                let exited = anyhow::Error::new(std::io::Error::from_raw_os_error(errno))
                    .context(Observation::Stat);
                remember_candidate_failure(&mut last, &exited);
                assert_eq!(last, observation);
                let mut otherwise_readable = Observation::NoRenderer;
                remember_candidate_failure(&mut otherwise_readable, &exited);
                assert_eq!(otherwise_readable, Observation::RendererChanged);
                assert!(!otherwise_readable.unobservable());
            }
        }
        let mut last = Observation::NoRenderer;
        remember_candidate_failure(&mut last, &Observation::StatusIncomplete.into());
        assert_eq!(last, Observation::StatusIncomplete);
        assert!(last.unobservable());
    }
    #[test]
    fn partial_unknown_and_duplicate_proc_fields_never_establish_support() {
        assert_eq!(numbers::<4>("1 1 1 1"), Some([1; 4]));
        for value in ["", "1 1 1", "1 1 1 1 1", "1 1 1 unknown", "-1 1 1 1"] {
            assert!(numbers::<4>(value).is_none());
        }
        assert_eq!(field("NoNewPrivs:\t1\n", "NoNewPrivs:"), Some("1"));
        assert_eq!(
            field("NoNewPrivs:\t1\nNoNewPrivs:\t0\n", "NoNewPrivs:"),
            None
        );
        assert!(stat("unknown").is_none());
        assert!(stat("20 (chrome (renderer)) R 10 10").is_none());
    }
}
