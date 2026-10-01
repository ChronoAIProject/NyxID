//! One command registers, enables local authority and starts the node service.
use anyhow::{Context, Result, bail};
use clap::Args;
use serde::Deserialize;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Args)]
pub struct Setup {
    #[arg(long, env = "NYXID_NODE_TOKEN", hide_env_values = true)]
    pub token: Option<String>,
    #[arg(
        long,
        env = "NYXID_NODE_URL",
        default_value = "wss://nyx-api.chrono-ai.fun/api/v1/nodes/ws"
    )]
    pub url: String,
    /// Enable commands and files with dedicated workspace defaults.
    #[arg(long)]
    pub machine: bool,
    #[arg(long)]
    pub shell: bool,
    #[arg(long)]
    pub files: bool,
    #[arg(long)]
    pub computer: bool,
    #[arg(long = "root")]
    pub roots: Vec<PathBuf>,
    #[arg(long)]
    pub allow_root: bool,
    #[arg(long, value_enum, default_value = "standard")]
    pub computer_mode: super::commands::Mode,
    #[arg(long)]
    pub cua_driver: Option<PathBuf>,
    /// Linux VM: create separate browser/agent users and a system supervisor (run with sudo).
    #[arg(long)]
    pub separate_users: bool,
    /// Used by the machine image, which supplies the two users and display.
    #[arg(long, hide = true)]
    pub container: bool,
    /// Skip the admin policy installation; saved-login filling stays unavailable.
    #[arg(long)]
    pub skip_browser_policy: bool,
    /// Container entrypoints start the daemon in the foreground themselves.
    #[arg(long)]
    pub no_daemon: bool,
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, env = "NYXID_PROFILE")]
    pub profile: Option<String>,
}

#[derive(Deserialize)]
struct PairResponse {
    code: String,
    device: Zeroizing<String>,
    url: String,
    expires_in: u64,
    interval: u64,
}
#[derive(Deserialize)]
struct PollResponse {
    status: String,
    token: Option<Zeroizing<String>>,
}

fn http_base(ws: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(ws)?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => bail!("Node URL must use wss:// or ws://"),
    };
    let localhost = url
        .host_str()
        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"));
    if scheme == "http" && !localhost {
        bail!("Use wss:// for a remote machine setup");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Node URL must not contain credentials, query or fragment");
    }
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("invalid node URL"))?;
    url.set_path("/api/v1/machines/pair/");
    Ok(url)
}

pub async fn run(mut args: Setup) -> Result<()> {
    let api = http_base(&args.url)?;
    let profile = args.profile.as_deref();
    if let Some(profile) = profile {
        crate::auth::validate_profile_name(profile)?;
    }
    let separated = args.separate_users || args.container;
    if separated && (!cfg!(target_os = "linux") || unsafe { libc::geteuid() } != 0) {
        bail!(
            "Separate users require a Linux VM and a supervisor installed with sudo. Run this setup with sudo --separate-users, or use the machine container."
        );
    }
    let directory = if separated && args.config.is_none() {
        PathBuf::from("/var/lib/nyxid-machine")
            .join(profile.unwrap_or("default"))
            .join("node")
    } else {
        crate::node::config::resolve_config_dir_with_profile(args.config.as_deref(), profile)?
    };
    let config_path = directory.join("config.toml");
    let token = args.token.take().map(Zeroizing::new);
    let shell = args.machine || args.shell;
    let files = args.machine || args.files;
    if !shell && !files && !args.computer {
        bail!("Choose --machine (commands and files) or --computer");
    }
    eprintln!(
        "Recommended: use the machine container or set up a VM with --separate-users. Agents act with their OS user's full access; prompt injection is possible."
    );
    if shell && !separated {
        eprintln!(
            "Not isolated: agent commands can read this node's stored credentials, signing secret and node token, including its config and local credential store. File-tool workspace limits do not constrain the shell. You may proceed, but prefer the container or --separate-users."
        );
    }
    if config_path.exists() && token.is_some() {
        bail!(
            "This profile is already registered. Use machine enable or choose another --profile."
        );
    }
    if !config_path.exists() {
        let token = match token {
            Some(token) => token,
            None => pair(&api, shell, files, args.computer).await?,
        };
        crate::node::agent::cmd_register(&token, Some(&args.url), directory.to_str(), false)
            .await?;
    }
    let mut config = crate::node::config::NodeConfig::load(&config_path)?;
    let data_dir = if separated {
        let (agent, browser) = if args.container {
            (
                super::process::Identity::resolve(Some("agent"))?,
                super::process::Identity::resolve(Some("browser"))?,
            )
        } else {
            provision_users(profile)?
        };
        config.machine.agent_user = Some(agent.name.clone());
        config.machine.browser_user = Some(browser.name.clone());
        let data = directory
            .parent()
            .context("invalid node directory")?
            .join("desktop");
        std::fs::create_dir_all(&data)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o711))?;
        if args.roots.is_empty() {
            args.roots.push(if args.container {
                PathBuf::from("/workspace")
            } else {
                agent.home.join("workspace")
            });
        }
        for root in &args.roots {
            std::fs::create_dir_all(root)?;
            super::browser::chown(root, agent.uid, agent.gid)?;
        }
        config.save(&config_path)?;
        data
    } else {
        directory.join("managed-desktop")
    };
    let cua_driver = if args.computer && separated && args.cua_driver.is_none() {
        Some(super::cua::install(Path::new("/opt/nyxid/cua")).await?)
    } else {
        args.cua_driver
    };
    super::commands::run(
        super::commands::Commands::Enable(super::commands::Enable {
            shell,
            files,
            computer: args.computer,
            roots: args.roots,
            computer_mode: args.computer_mode,
            allow_root: args.allow_root,
            cua_driver,
        }),
        directory.to_str(),
        None,
    )
    .await?;
    if args.computer && !args.skip_browser_policy {
        let port = browser_port(profile.unwrap_or("default"));
        let installed = if unsafe { libc::geteuid() } == 0 {
            install_browser(port)?;
            true
        } else {
            eprintln!(
                "Saved-login filling needs admin-installed Chromium policies and the protected NyxID extension. Your system may ask for an administrator password. If declined, computer use still works and filling remains unavailable."
            );
            std::process::Command::new("sudo")
                .arg(std::env::current_exe()?)
                .args([
                    "node",
                    "machine-browser-install",
                    "--port",
                    &port.to_string(),
                ])
                .status()
                .is_ok_and(|s| s.success())
        };
        if installed {
            let binary = browser_binary()?;
            let mut config = crate::node::config::NodeConfig::load(&config_path)?;
            config.machine.managed_browser = Some(nyxid_machine::config::ManagedBrowserConfig {
                binary,
                data_dir,
                update_port: port,
                container: args.container,
            });
            config.save(&config_path)?;
        } else {
            eprintln!(
                "Saved-login filling is unavailable because managed policies were not installed. Run setup again when ready to approve the admin installation."
            );
        }
    }
    if !separated {
        eprintln!(
            "Saved-login typing is off until the owner allows it in Assistant → Machines settings. Agent commands share the browser user's access and could read typed values. The machine container or separated VM is recommended."
        );
    }
    if !args.no_daemon {
        if args.separate_users {
            install_supervisor(&directory, profile, args.computer)?;
        } else {
            crate::node::daemon::install(directory.to_str(), profile, None, false)?;
            crate::node::daemon::start(directory.to_str(), profile)?;
        }
    }
    eprintln!(
        "Machine setup complete. The Assistant → Machines page shows connection progress; NyxBot resumes when capabilities are reported."
    );
    Ok(())
}

async fn pair(
    api: &url::Url,
    shell: bool,
    files: bool,
    computer: bool,
) -> Result<Zeroizing<String>> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?;
    let mut capabilities = Vec::new();
    if shell {
        capabilities.push("shell");
    }
    if files {
        capabilities.push("files");
    }
    if computer {
        capabilities.push("computer");
    }
    let mut hostname_bytes = [0u8; 256];
    if unsafe { libc::gethostname(hostname_bytes.as_mut_ptr().cast(), hostname_bytes.len()) } != 0 {
        bail!("Could not read machine hostname");
    }
    let end = hostname_bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(hostname_bytes.len());
    let hostname = String::from_utf8_lossy(&hostname_bytes[..end]);
    let response = client
        .post(api.join("request")?)
        .json(&json!({"hostname":hostname,"os":std::env::consts::OS,"capabilities":capabilities}))
        .send()
        .await?
        .error_for_status()?;
    let pair: PairResponse = response.json().await?;
    eprintln!(
        "Pairing code: {}\nOpen {} or tell NyxBot this code. Confirm only the machine you are setting up.",
        pair.code, pair.url
    );
    let deadline = Instant::now() + Duration::from_secs(pair.expires_in.min(900));
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(pair.interval.clamp(2, 10))).await;
        let response = client
            .post(api.join("poll")?)
            .json(&json!({"device":pair.device.as_str()}))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let code = body["error"]["code"]
                .as_u64()
                .or_else(|| body["error_code"].as_u64())
                .unwrap_or(0);
            if matches!(code, 11203 | 11206) {
                continue;
            }
            bail!(
                "Machine pairing expired, was declined or could not be delivered (NyxID code {code}). Run setup again for a fresh code."
            );
        }
        let result: PollResponse = response.json().await?;
        if result.status == "approved" {
            return result
                .token
                .context("Approved pairing did not deliver a setup credential");
        }
    }
    bail!("Machine pairing expired. Run setup again for a fresh code.")
}

pub fn browser_port(profile: &str) -> u16 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(profile.as_bytes());
    28000 + u16::from_be_bytes([digest[0], digest[1]]) % 10000
}

fn browser_binary() -> Result<PathBuf> {
    let paths: &[&str] = if cfg!(target_os = "macos") {
        &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
    } else {
        &[
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome",
        ]
    };
    paths.iter().map(PathBuf::from).find(|p|p.is_file()).context("Install Chromium (or Google Chrome on macOS), then run setup again to enable saved-login filling")
}

pub fn install_browser(port: u16) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("Managed browser installation needs administrator access");
    }
    if port == 0 {
        bail!("Managed extension update port must be fixed");
    }
    let installed = install_supervisor_binary()?;
    super::browser::install(
        Path::new("/"),
        &installed,
        &format!("http://127.0.0.1:{port}/update.xml"),
        cfg!(target_os = "macos"),
    )
}

fn install_supervisor_binary() -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let directory = Path::new("/opt/nyxid/bin");
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755))?;
    let path = directory.join("nyxid");
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    std::io::copy(
        &mut std::fs::File::open(std::env::current_exe()?)?,
        &mut temp,
    )?;
    temp.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    temp.persist(&path)?;
    Ok(path)
}

fn provision_users(
    profile: Option<&str>,
) -> Result<(super::process::Identity, super::process::Identity)> {
    let suffix = profile.unwrap_or("default");
    if suffix.len() > 16 {
        bail!("Separated VM profiles must be at most 16 characters");
    }
    let agent = format!("nyxagent-{suffix}");
    let browser = format!("nyxbrowser-{suffix}");
    for name in [&agent, &browser] {
        if super::process::Identity::resolve(Some(name)).is_err() {
            let status = std::process::Command::new("useradd")
                .args([
                    "--system",
                    "--create-home",
                    "--shell",
                    "/usr/sbin/nologin",
                    name,
                ])
                .status()?;
            if !status.success() {
                bail!("Could not create isolated machine users");
            }
        }
    }
    let agent = super::process::Identity::resolve(Some(&agent))?;
    let browser = super::process::Identity::resolve(Some(&browser))?;
    if agent.uid == 0 || browser.uid == 0 || agent.uid == browser.uid || agent.gid == browser.gid {
        bail!("Machine browser and agent need separate non-root UIDs and groups");
    }
    Ok((agent, browser))
}

fn install_supervisor(directory: &Path, profile: Option<&str>, computer: bool) -> Result<()> {
    let binary = install_supervisor_binary()?;
    let suffix = profile.unwrap_or("default");
    let unit = format!("nyxid-machine-{suffix}.service");
    let mut display = String::new();
    if computer {
        for binary in ["Xvfb", "xauth", "openbox"] {
            if !std::process::Command::new("sh")
                .args(["-c", &format!("command -v {binary}")])
                .stdout(std::process::Stdio::null())
                .status()?
                .success()
            {
                bail!(
                    "Install xvfb, xauth, openbox and Chromium on this VM, then run setup --separate-users again"
                );
            }
        }
        let config = crate::node::config::NodeConfig::load(&directory.join("config.toml"))?;
        let browser = config
            .machine
            .browser_user
            .context("browser user missing")?;
        let number = 100 + browser_port(suffix) % 500;
        let authority = directory
            .parent()
            .context("invalid node directory")?
            .join("desktop/Xauthority");
        super::browser::create_xauthority(&authority, &browser, number)?;
        let display_unit = format!("nyxid-display-{suffix}.service");
        let contents = format!(
            "[Unit]\nDescription=NyxID isolated display\n[Service]\nUser={browser}\nExecStart=/usr/bin/Xvfb :{number} -screen 0 1280x800x24 -nolisten tcp -auth {}\nRestart=on-failure\n[Install]\nWantedBy=multi-user.target\n",
            authority.display()
        );
        std::fs::write(
            Path::new("/etc/systemd/system").join(&display_unit),
            contents,
        )?;
        display = format!(
            "Environment=DISPLAY=:{number}\nEnvironment=XAUTHORITY={}\n",
            authority.display()
        );
        let status = std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status()?;
        if !status.success() {
            bail!("Could not reload machine display service");
        }
        let status = std::process::Command::new("systemctl")
            .args(["enable", "--now", &display_unit])
            .status()?;
        if !status.success() {
            bail!("Could not start machine display service");
        }
    }
    let contents = format!(
        "[Unit]\nDescription=NyxID machine supervisor\nAfter=network-online.target\n[Service]\nType=simple\nExecStart={} node start --config {}\n{display}Restart=on-failure\nUMask=0077\nLimitCORE=0\n[Install]\nWantedBy=multi-user.target\n",
        systemd_quote(&binary)?,
        systemd_quote(directory)?
    );
    std::fs::write(Path::new("/etc/systemd/system").join(&unit), contents)?;
    for args in [
        vec!["daemon-reload"],
        vec!["enable", "--now", unit.as_str()],
    ] {
        if !std::process::Command::new("systemctl")
            .args(args)
            .status()?
            .success()
        {
            bail!("Could not start machine supervisor");
        }
    }
    Ok(())
}
fn systemd_quote(path: &Path) -> Result<String> {
    let path = path.to_str().context("Invalid system service path")?;
    if path.chars().any(|c| matches!(c, '\n' | '\r' | '%' | '$')) {
        bail!("Unsupported system service path");
    }
    Ok(format!(
        "\"{}\"",
        path.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_urls_reject_credentials_and_clear_remote_plaintext() {
        assert!(http_base("ws://example.com/api/v1/nodes/ws").is_err());
        assert!(http_base("wss://user:secret@example.com/api/v1/nodes/ws").is_err());
        assert_eq!(
            http_base("ws://localhost:3001/api/v1/nodes/ws")
                .unwrap()
                .as_str(),
            "http://localhost:3001/api/v1/machines/pair/"
        );
        assert_ne!(browser_port("one"), browser_port("two"));
    }
}
