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
        let directory = identity.home.join(".nyxid-dev-display");
        browser::runtime_directory(&directory, identity.uid, identity.gid, 0o700)?;
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
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&authority)?;
        std::fs::File::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        std::io::Write::write_all(&mut file, &record)?;
        browser::chown(&authority, identity.uid, identity.gid)?;
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
