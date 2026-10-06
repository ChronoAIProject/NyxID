//! Linux-only feasibility harness. Never called by the installed node.
//! Reuse the actual production identity, NNP and seccomp implementation.
#[allow(dead_code)]
#[path = "../../../src/node/machine/process.rs"]
mod production_process;

use anyhow::{Context, Result, bail, ensure};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use tokio::process::Command;

use production_process::context_sandbox::{abi, context_directories, ruleset};
const MIN_ABI: i64 = 6;
fn check(value: libc::c_long) -> std::io::Result<libc::c_long> {
    if value < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(value)
    }
}

async fn run() -> Result<i32> {
    let mut args = std::env::args().skip(1);
    let mode = args
        .next()
        .context("usage: spike abi | MODE ROOT UID COMMAND [ARG...]")?;
    if mode == "abi" {
        println!("landlock_abi={} minimum={MIN_ABI}", abi()?);
        return Ok(0);
    }
    ensure!(
        matches!(
            mode.as_str(),
            "confined" | "uid-only-control" | "leaked-fd-control"
        ),
        "unknown mode"
    );
    let root = PathBuf::from(args.next().context("root missing")?);
    let uid: u32 = args.next().context("uid missing")?.parse()?;
    ensure!(
        uid >= 10000 && unsafe { libc::geteuid() } == 0,
        "spike_requires_root_and_test_uid"
    );
    let program = args.next().context("command missing")?;
    let directories = context_directories(&root, uid)?;
    let sandbox = (mode != "uid-only-control")
        .then(|| ruleset(&directories))
        .transpose()?;
    let mut command = Command::new(program);
    command.args(args);
    let identity = production_process::Identity {
        uid,
        gid: uid,
        name: format!("spike-{uid}"),
        home: root.join("home"),
        sandbox: None,
        policy_groups: Vec::new(),
        desktop: None,
    };
    identity.prepare_agent(&mut command)?;
    command
        .env("PATH", "/usr/bin:/bin")
        .env("TMPDIR", root.join("tmp"))
        .env("XDG_CACHE_HOME", root.join("home/.cache"))
        .env("XDG_CONFIG_HOME", root.join("home/.config"))
        .env("CARGO_HOME", root.join("home/.cargo"))
        .env("NYX_WORKSPACE", root.join("workspace"));
    let cwd_fd = directories[0].as_raw_fd();
    let sandbox_fd = sandbox.as_ref().map(AsRawFd::as_raw_fd);
    let scrub_fds = mode != "leaked-fd-control";
    unsafe {
        command.pre_exec(move || {
            check(libc::fchdir(cwd_fd).into())?;
            libc::umask(0o077);
            if let Some(fd) = sandbox_fd {
                check(libc::syscall(libc::SYS_landlock_restrict_self, fd, 0))?;
            }
            if scrub_fds {
                // CLOEXEC preserves Rust's internal spawn-error pipe until exec.
                check(libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32))?;
            }
            Ok(())
        });
    }
    let status = command.status().await.context("separated_spawn_failed")?;
    if let Some(code) = status.code() {
        Ok(code)
    } else {
        bail!("child_killed_by_signal")
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    match run().await {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("spike: {error:#}");
            std::process::exit(125);
        }
    }
}
