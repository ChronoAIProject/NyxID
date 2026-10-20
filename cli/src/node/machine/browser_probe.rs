//! Observe the managed browser's real renderer without enabling DevTools.
use super::process::{self, Identity};
use anyhow::{Context, Result, ensure};
use nyxid_machine::config::ManagedBrowserConfig;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

// Pending hosted calibration: retain the existing bound until samples from the
// actual CI image establish whether additional cold-start headroom is needed.
const BOUND: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Process {
    pid: u32,
    parent: u32,
    group: u32,
    start: u64,
    uid: [u32; 4],
    gid: [u32; 4],
    nnp: u32,
    seccomp: u32,
    filters: u32,
    nested: Vec<u32>,
    exe: PathBuf,
    namespaces: [PathBuf; 3],
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
fn namespaces(path: &Path) -> Result<[PathBuf; 3]> {
    Ok([
        fs::read_link(path.join("ns/user"))?,
        fs::read_link(path.join("ns/pid"))?,
        fs::read_link(path.join("ns/net"))?,
    ])
}
impl Process {
    fn read(pid: u32) -> Result<Self> {
        let path = PathBuf::from(format!("/proc/{pid}"));
        let before = fs::read_to_string(path.join("stat"))?;
        let (parent, group, start) = stat(&before).context("browser_proc_stat_incomplete")?;
        let status = fs::read_to_string(path.join("status"))?;
        let parse = |name| field(&status, name).context("browser_proc_status_incomplete");
        let process = Self {
            pid,
            parent,
            group,
            start,
            uid: numbers(parse("Uid:")?).context("browser_proc_uid_incomplete")?,
            gid: numbers(parse("Gid:")?).context("browser_proc_gid_incomplete")?,
            nnp: parse("NoNewPrivs:")?.parse()?,
            seccomp: parse("Seccomp:")?.parse()?,
            filters: parse("Seccomp_filters:")?.parse()?,
            nested: parse("NSpid:")?
                .split_whitespace()
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()?,
            exe: fs::read_link(path.join("exe"))?,
            namespaces: namespaces(&path)?,
        };
        ensure!(
            stat(&fs::read_to_string(path.join("stat"))?) == Some((parent, group, start)),
            "browser_proc_changed"
        );
        Ok(process)
    }
    fn identity(&self, identity: &Identity) -> bool {
        self.uid == [identity.uid; 4] && self.gid == [identity.gid; 4] && self.nnp == 1
    }
    fn sandboxed(&self, browser: &Self, supervisor: &[PathBuf; 3], identity: &Identity) -> bool {
        self.identity(identity)
            && browser.identity(identity)
            && self.exe == browser.exe
            && self.group == browser.pid
            && self.seccomp == 2
            && self.filters > browser.filters
            && self.nested.len() >= 2
            && self.nested.iter().all(|pid| *pid > 0)
            && self
                .namespaces
                .iter()
                .zip(&browser.namespaces)
                .zip(supervisor)
                .all(|((child, parent), supervisor)| child != parent && child != supervisor)
    }
    fn descends_from(&self, browser: &Self) -> Result<bool> {
        let mut parent = self.parent;
        let mut visited = BTreeSet::new();
        while parent != browser.pid {
            if parent == 0 || !visited.insert(parent) || visited.len() > 64 {
                return Ok(false);
            }
            let ancestor = Self::read(parent)?;
            if ancestor.group != browser.pid {
                return Ok(false);
            }
            parent = ancestor.parent;
        }
        Ok(true)
    }
    fn unchanged(&self) -> bool {
        Self::read(self.pid).is_ok_and(|current| current == *self)
    }
}
fn renderer(pid: u32, identity: &Identity, supervisor: &[PathBuf; 3]) -> Result<bool> {
    let browser = Process::read(pid)?;
    ensure!(browser.identity(identity), "browser_probe_identity_changed");
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(candidate) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if unsafe { libc::getpgid(candidate as i32) } != pid as i32 {
            continue;
        }
        let Ok(args) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        // Chromium rewrites its argv to one space-separated process title.
        if !args
            .split(|byte| *byte == 0 || byte.is_ascii_whitespace())
            .any(|arg| arg == b"--type=renderer")
        {
            continue;
        }
        let Ok(child) = Process::read(candidate) else {
            continue;
        };
        if child.sandboxed(&browser, supervisor, identity)
            && child.descends_from(&browser)?
            && child.unchanged()
            && browser.unchanged()
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn no_tcp_listener(pid: u32) -> Result<bool> {
    let mut sockets = BTreeSet::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(candidate) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if unsafe { libc::getpgid(candidate as i32) } != pid as i32 {
            continue;
        }
        for fd in fs::read_dir(entry.path().join("fd"))? {
            let target = match fs::read_link(fd?.path()) {
                Ok(target) => target,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let target = target.to_string_lossy();
            if let Some(inode) = target
                .strip_prefix("socket:[")
                .and_then(|s| s.strip_suffix(']'))
            {
                sockets.insert(inode.to_owned());
            }
        }
    }
    for file in ["tcp", "tcp6"] {
        let rows = fs::read_to_string(format!("/proc/{pid}/net/{file}"))?;
        for row in rows.lines().skip(1) {
            let fields: Vec<_> = row.split_whitespace().collect();
            ensure!(fields.len() >= 10, "browser_socket_table_incomplete");
            if fields[3] == "0A" && sockets.contains(fields[9]) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub async fn observe(
    browser: &ManagedBrowserConfig,
    identity: &Identity,
    policy: bool,
) -> Result<()> {
    use std::os::fd::AsRawFd;
    ensure!(identity.uid != 0, "separated_browser_requires_non_root");
    super::browser_layout::Layout::detect(&browser.binary)?;
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
    let supervisor = namespaces(Path::new("/proc/self"))?;
    let mut refused_at = None;
    process::probe_running(command, BOUND, |pid, stderr| {
        if !policy {
            return renderer(pid, identity, &supervisor);
        }
        ensure!(
            !profile.path().join("DevToolsActivePort").exists()
                && !stderr
                    .windows(b"DevTools listening on".len())
                    .any(|window| window == b"DevTools listening on")
                && no_tcp_listener(pid)?,
            "separated_browser_policy_not_effective"
        );
        let refusal = b"DevTools remote debugging is disallowed by the system admin";
        if stderr
            .windows(refusal.len())
            .any(|window| window == refusal)
        {
            let since = refused_at.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(200) {
                let parent = Process::read(pid)?;
                return Ok(parent.identity(identity) && parent.unchanged());
            }
        }
        Ok(false)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn browser() -> Process {
        Process {
            pid: 10,
            parent: 1,
            group: 10,
            start: 100,
            uid: [1001; 4],
            gid: [1002; 4],
            nnp: 1,
            seccomp: 2,
            filters: 1,
            nested: vec![10],
            exe: "/usr/lib/chromium/chromium".into(),
            namespaces: ["user:outer".into(), "pid:outer".into(), "net:outer".into()],
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
    #[test]
    fn chromium_diagnostics_require_every_active_sandbox_layer() {
        let parent = browser();
        let child = Process {
            pid: 20,
            parent: 10,
            start: 101,
            filters: 2,
            nested: vec![20, 1],
            namespaces: ["user:inner".into(), "pid:inner".into(), "net:inner".into()],
            ..parent.clone()
        };
        assert!(child.sandboxed(&parent, &parent.namespaces, &identity()));
        for index in 0..3 {
            let mut bad = child.clone();
            bad.namespaces[index] = parent.namespaces[index].clone();
            assert!(!bad.sandboxed(&parent, &parent.namespaces, &identity()));
        }
        for index in 0..4 {
            let mut bad = child.clone();
            bad.uid[index] = 0;
            assert!(!bad.sandboxed(&parent, &parent.namespaces, &identity()));
            let mut bad = child.clone();
            bad.gid[index] = 0;
            assert!(!bad.sandboxed(&parent, &parent.namespaces, &identity()));
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
                group: 11,
                ..child.clone()
            },
            Process {
                exe: "/tmp/forged-browser".into(),
                ..child.clone()
            },
        ] {
            assert!(!bad.sandboxed(&parent, &parent.namespaces, &identity()));
        }
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
