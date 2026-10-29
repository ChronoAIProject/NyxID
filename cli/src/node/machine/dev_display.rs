//! Private X server for single-user Linux installs without a service-managed one.
use super::{browser, process::Identity};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::{io::AsyncBufReadExt, process::Command, sync::OnceCell};

pub struct Server {
    pub name: String,
    pub authority: PathBuf,
    _xvfb: tokio::process::Child,
    _wm: tokio::process::Child,
}
static SERVER: OnceCell<Server> = OnceCell::const_new();

pub fn endpoint() -> Result<(String, PathBuf)> {
    if let (Ok(name), Ok(authority)) = (
        std::env::var("NYXID_DEV_DISPLAY"),
        std::env::var("NYXID_DEV_XAUTHORITY"),
    ) {
        anyhow::ensure!(
            Some(&name) != std::env::var("DISPLAY").ok().as_ref(),
            "Developer display must be separate"
        );
        return Ok((name, authority.into()));
    }
    let server = SERVER.get().context("Developer display unavailable")?;
    Ok((server.name.clone(), server.authority.clone()))
}

pub async fn ensure(identity: &Identity) -> Result<()> {
    if endpoint().is_ok() {
        return Ok(());
    }
    SERVER
        .get_or_try_init(|| async { Server::launch(identity).await })
        .await?;
    Ok(())
}

impl Server {
    pub async fn launch(identity: &Identity) -> Result<Self> {
        // A separated HOME is 0700 and its owner is a worker. The hosted
        // supervisor has CHOWN but no DAC override: keep display authority in
        // a supervisor-owned sibling behind the protected role gate instead.
        let separated = identity.sandbox.is_some();
        let supervisor = unsafe { libc::geteuid() };
        let (directory, owner, directory_mode, authority_mode) =
            if separated || supervisor != identity.uid {
                let parent = identity.home.parent().context("context_role_missing")?;
                let directory = if separated {
                    parent.join("display")
                } else {
                    // A root supervisor must not repair through a worker-owned
                    // HOME. Keep the private display under its protected parent.
                    parent.join(format!(".nyxid-dev-display-{}", identity.uid))
                };
                browser::protected_runtime_parent(&directory)?;
                (directory, supervisor, 0o750, 0o640)
            } else {
                (
                    identity.home.join(".nyxid-dev-display"),
                    identity.uid,
                    0o700,
                    0o600,
                )
            };
        browser::runtime_directory(&directory, owner, identity.gid, directory_mode)?;
        let authority = directory.join("Xauthority");
        // A wildcard record lets Xvfb choose an unused display atomically through
        // -displayfd. Neither the cookie nor the display is an agent environment.
        let cookie = uuid::Uuid::new_v4();
        let mut record = vec![255, 255];
        for field in [
            &b""[..],
            &b""[..],
            &b"MIT-MAGIC-COOKIE-1"[..],
            cookie.as_bytes(),
        ] {
            record.extend_from_slice(&(field.len() as u16).to_be_bytes());
            record.extend_from_slice(field);
        }
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(authority_mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&authority)?;
        std::fs::File::set_permissions(
            &file,
            std::os::unix::fs::PermissionsExt::from_mode(authority_mode),
        )?;
        std::io::Write::write_all(&mut file, &record)?;
        browser::chown(&authority, owner, identity.gid)?;
        let mut command = Command::new("Xvfb");
        identity.prepare(&mut command)?;
        parent_bound(&mut command);
        command
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1280x800x24",
                "-nolisten",
                "tcp",
                // X clients can use the authenticated pathname socket. An
                // abstract listener outside the client's Landlock domain is
                // refused with EPERM, which libxcb does not fall back from.
                "-nolisten",
                "local",
                "-auth",
            ])
            .arg(&authority)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut xvfb = command
            .spawn()
            .context("Install Xvfb and openbox for the separate developer display")?;
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::io::BufReader::new(
                xvfb.stdout
                    .take()
                    .context("Xvfb display pipe unavailable")?,
            )
            .read_line(&mut line),
        )
        .await??;
        let number: u16 = line
            .trim()
            .parse()
            .context("Xvfb did not publish a display")?;
        let name = format!(":{number}");
        let mut command = Command::new("openbox");
        identity.prepare(&mut command)?;
        parent_bound(&mut command);
        command
            .env("DISPLAY", &name)
            .env("XAUTHORITY", &authority)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let wm = command.spawn()?;
        Ok::<_, anyhow::Error>(Server {
            name,
            authority,
            _xvfb: xvfb,
            _wm: wm,
        })
    }
}

fn parent_bound(command: &mut Command) {
    let parent = unsafe { libc::getpid() };
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 || libc::getppid() != parent
            {
                return Err(std::io::Error::other("Supervisor exited"));
            }
            Ok(())
        });
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        sync::Arc,
    };

    #[tokio::test]
    #[ignore = "requires a complete disposable hosted image with the fixed four capabilities and Localhost profile"]
    async fn legacy_display_uses_the_protected_parent_of_worker_home() {
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let parent = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o711)).unwrap();
        let mut identity = Identity::resolve(Some("devbrowser")).unwrap();
        identity.home = parent.path().join("home");
        fs::create_dir(&identity.home).unwrap();
        fs::set_permissions(&identity.home, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::chown(&identity.home, Some(identity.uid), Some(identity.gid)).unwrap();
        assert_eq!(
            fs::File::open(&identity.home).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let display = Server::launch(&identity).await.unwrap();
        assert!(display.authority.starts_with(parent.path()));
        assert!(!display.authority.starts_with(&identity.home));
        let authority = fs::metadata(&display.authority).unwrap();
        assert_eq!(
            (authority.uid(), authority.gid(), authority.mode() & 0o777),
            (0, identity.gid, 0o640)
        );
        let mut check = Command::new("xdpyinfo");
        identity.prepare(&mut check).unwrap();
        let status = check
            .env("DISPLAY", &display.name)
            .env("XAUTHORITY", &display.authority)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let home = fs::metadata(&identity.home).unwrap();
        assert_eq!((home.uid(), home.mode() & 0o777), (identity.uid, 0o700));
        std::os::unix::fs::chown(&identity.home, Some(0), Some(0)).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a complete disposable hosted image with the fixed four capabilities and Localhost profile"]
    async fn separated_display_does_not_require_supervisor_access_to_worker_home() {
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let status = fs::read_to_string("/proc/self/status").unwrap();
        for field in ["CapEff", "CapPrm", "CapBnd"] {
            let actual = status
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{field}:")))
                .unwrap();
            assert_eq!(u64::from_str_radix(actual.trim(), 16).unwrap(), 0xe1);
        }
        assert!(status.lines().any(|line| line == "NoNewPrivs:\t1"));
        let parent = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o711)).unwrap();
        let mut displays = Vec::new();
        for uid in [20001, 20004] {
            let root = parent.path().join(uid.to_string());
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o710)).unwrap();
            std::os::unix::fs::chown(&root, Some(0), Some(uid)).unwrap();
            for name in ["workspace", "home", "tmp"] {
                let path = root.join(name);
                fs::create_dir(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                std::os::unix::fs::chown(&path, Some(uid), Some(uid)).unwrap();
            }
            let mut identity = Identity::resolve(Some(&format!("nyxc{uid}"))).unwrap();
            identity.home = root.join("home");
            identity.sandbox = Some(Arc::new(
                super::super::process::context_sandbox::Sandbox::browser(&root, uid).unwrap(),
            ));
            assert_eq!(
                fs::File::open(&identity.home).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            let display = Server::launch(&identity).await.unwrap();
            assert!(!display.authority.starts_with(&identity.home));
            let authority = fs::metadata(&display.authority).unwrap();
            assert_eq!(
                (authority.uid(), authority.gid(), authority.mode() & 0o777),
                (0, uid, 0o640)
            );
            let mut check = Command::new("xdpyinfo");
            identity.prepare(&mut check).unwrap();
            let result = check
                .env("DISPLAY", &display.name)
                .env("XAUTHORITY", &display.authority)
                .output()
                .await
                .unwrap();
            assert!(
                result.status.success(),
                "private X server is not usable: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let other = if uid == 20001 { 20004 } else { 20001 };
            let denied = std::process::Command::new("setpriv")
                .args([
                    "--reuid",
                    &other.to_string(),
                    "--regid",
                    &other.to_string(),
                    "--clear-groups",
                    "test",
                    "-r",
                ])
                .arg(&display.authority)
                .status()
                .unwrap();
            assert!(
                !denied.success(),
                "sibling browser may not read display authority"
            );
            assert_eq!(fs::metadata(&identity.home).unwrap().uid(), uid);
            assert_eq!(fs::metadata(&identity.home).unwrap().mode() & 0o777, 0o700);
            displays.push(display);
        }
        assert_ne!(displays[0].name, displays[1].name);
        assert_ne!(displays[0].authority, displays[1].authority);
    }
}
