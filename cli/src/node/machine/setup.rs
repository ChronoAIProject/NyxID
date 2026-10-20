//! One command registers, enables local authority and starts the node service.
use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use nyxid_machine::context::{Choice, Mode};
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
    #[arg(long, num_args=0..=1, default_missing_value="true")]
    pub browser: Option<bool>,
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
    /// Intent for new agent assignments. New setups default to automatic and
    /// require confirmed support; shared-legacy explicitly accepts shared access.
    #[arg(long, value_enum)]
    pub separation: Option<Separation>,
    /// Used by the machine image, which supplies the two users and display.
    #[arg(long, hide = true)]
    pub container: bool,
    /// Skip the admin policy installation; saved-login filling stays unavailable.
    #[arg(long)]
    pub skip_browser_policy: bool,
    /// Container entrypoints start the daemon in the foreground themselves.
    #[arg(long)]
    pub no_daemon: bool,
    /// Limited shell/files setup helper for the node-only image. Full hosted
    /// Machines instead use managed-start with sealed owner capabilities.
    #[arg(long, hide = true)]
    pub node_only_machine_preset: bool,
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, env = "NYXID_PROFILE")]
    pub profile: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Separation {
    Automatic,
    RequireSeparated,
    SharedLegacy,
}

impl From<Separation> for Choice {
    fn from(value: Separation) -> Self {
        match value {
            Separation::Automatic => Self::Automatic,
            Separation::RequireSeparated => Self::RequireSeparated,
            Separation::SharedLegacy => Self::SharedLegacy,
        }
    }
}

fn setup_choice(
    requested: Option<Separation>,
    existing: Option<&nyxid_machine::config::Config>,
) -> Option<Choice> {
    requested.map(Choice::from).or_else(|| {
        existing
            .and_then(|config| config.separation_choice)
            .or_else(|| {
                // Rerunning setup against an existing machine must not change its
                // prior intent, including profiles predating recorded selections.
                let already_enabled = existing.is_some_and(|config| {
                    config.shell || config.files || config.computer || config.browser_enabled()
                });
                (!already_enabled).then_some(Choice::Automatic)
            })
    })
}

fn selection_error(reason: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{reason}: {} To explicitly accept shared legacy access, rerun setup with --separation shared-legacy after reviewing this warning: {}",
        nyxid_machine::context::unavailable_message(reason),
        nyxid_machine::context::SHARED_LEGACY_WARNING,
    )
}

/// Reject known unsupported configurations before pairing/registration. The
/// real mechanism probes still decide support after provisioning completes.
fn preflight_selection(
    choice: Option<Choice>,
    linux: bool,
    supervisor: u32,
    separate_users_requested: bool,
    managed_browser_requested: bool,
    existing: Option<&nyxid_machine::config::Config>,
) -> Result<()> {
    if choice.is_none() || choice == Some(Choice::SharedLegacy) {
        return Ok(());
    }
    let reason = if !linux {
        Some("separated_requires_linux")
    } else if supervisor != 0
        || (!separate_users_requested
            && !existing.is_some_and(|config| {
                config.agent_user.is_some()
                    && config.browser_user.is_some()
                    && config.effective_dev_browser_user().is_some()
            }))
    {
        Some("separated_requires_separate_users")
    } else if !managed_browser_requested
        && !existing.is_some_and(|config| config.managed_browser.is_some())
    {
        Some("separated_requires_managed_browser")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(selection_error(reason));
    }
    Ok(())
}

async fn verify_selection(config: &nyxid_machine::config::Config) -> Result<Option<Mode>> {
    let Some(choice) = config.separation_choice else {
        return Ok(None);
    };
    if choice == Choice::SharedLegacy {
        return Ok(Some(Mode::SharedLegacy));
    }
    let identity = |name| {
        super::process::Identity::resolve(name)
            .map_err(|_| selection_error("separated_requires_separate_users"))
    };
    let command = identity(config.agent_user.as_deref())?;
    let secure = identity(config.browser_user.as_deref())?;
    let developer = config
        .effective_dev_browser_user()
        .map(|name| identity(Some(name)))
        .transpose()?;
    let evidence = super::separation::observe(config, &command, &secure, developer.as_ref()).await;
    nyxid_machine::context::select_new(choice, Some(&evidence))
        .map(Some)
        .map_err(selection_error)
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
    if args.node_only_machine_preset {
        args.machine = true;
        args.shell = true;
        args.files = true;
        args.computer = false;
        args.browser = Some(false);
        args.computer_mode = super::commands::Mode::Standard;
        args.roots = vec![PathBuf::from("/workspace")];
        args.allow_root = false;
    }
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
    crate::node::managed_runtime::require_self_hosted_mutation(&directory)?;
    let token = args.token.take().map(Zeroizing::new);
    let shell = args.machine || args.shell;
    let files = args.machine || args.files;
    if !shell && !files && !args.computer && args.browser != Some(true) {
        bail!("Choose --machine (commands and files), --browser or --computer");
    }
    if config_path.exists() && token.is_some() {
        bail!(
            "This profile is already registered. Use machine enable or choose another --profile."
        );
    }
    let existing = config_path
        .exists()
        .then(|| crate::node::config::NodeConfig::load(&config_path))
        .transpose()?;
    let choice = setup_choice(
        args.separation,
        existing.as_ref().map(|config| &config.machine),
    );
    preflight_selection(
        choice,
        cfg!(target_os = "linux"),
        unsafe { libc::geteuid() },
        separated,
        args.browser.unwrap_or(args.computer) && !args.skip_browser_policy,
        existing.as_ref().map(|config| &config.machine),
    )?;
    eprintln!(
        "Agents act with their OS user's access; prompt injection is possible. Use a separate machine container or VM per agent for full isolation."
    );
    if choice == Some(Choice::SharedLegacy) || (choice.is_none() && shell && !separated) {
        eprintln!("{}", nyxid_machine::context::SHARED_LEGACY_WARNING);
    }
    if !config_path.exists() {
        let token = match token {
            Some(token) => token,
            None => {
                pair(
                    &api,
                    shell,
                    files,
                    args.computer,
                    args.browser.unwrap_or(args.computer),
                )
                .await?
            }
        };
        crate::node::agent::cmd_register(&token, Some(&args.url), directory.to_str(), false)
            .await?;
    }
    let mut config = crate::node::config::NodeConfig::load(&config_path)?;
    if config.machine.separation_choice != choice {
        // Retain attempted automatic/required intent even if later provisioning
        // or probes fail, so retrying cannot reinterpret it as legacy consent.
        config.machine.separation_choice = choice;
        config.save(&config_path)?;
    }
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
        config.machine.dev_browser_user = Some(if args.container {
            "devbrowser".into()
        } else {
            format!("nyxdev-{}", profile.unwrap_or("default"))
        });
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
    let cua_driver = if (args.computer || args.browser == Some(true))
        && separated
        && args.cua_driver.is_none()
    {
        Some(super::cua::install(Path::new("/opt/nyxid/cua")).await?)
    } else {
        args.cua_driver
    };
    super::commands::run(
        super::commands::Commands::Enable(super::commands::Enable {
            shell,
            files,
            computer: args.computer,
            browser: args.browser,
            roots: args.roots,
            computer_mode: args.computer_mode,
            allow_root: args.allow_root,
            cua_driver,
        }),
        directory.to_str(),
        None,
    )
    .await?;
    if args.node_only_machine_preset {
        let mut config = crate::node::config::NodeConfig::load(&config_path)?;
        config.machine.shell = true;
        config.machine.files = true;
        config.machine.computer = false;
        config.machine.browser = Some(false);
        config.machine.computer_mode = nyxid_machine::ComputerMode::Standard;
        config.machine.roots = vec![PathBuf::from("/workspace")];
        config.machine.max_jobs = 2;
        config.machine.max_timeout_secs = 3600;
        config.machine.output_bytes = 1024 * 1024;
        config.machine.allow_root = false;
        config.machine.cua_driver = None;
        config.machine.managed_browser = None;
        config.machine.validate().map_err(anyhow::Error::msg)?;
        config.save(&config_path)?;
    }
    if args.browser.unwrap_or(args.computer) && !args.skip_browser_policy {
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
    let config = crate::node::config::NodeConfig::load(&config_path)?;
    match (
        config.machine.separation_choice,
        verify_selection(&config.machine).await?,
    ) {
        (Some(choice), Some(mode)) => eprintln!(
            "New-assignment selection: {} ({}). Existing assignments retain their stored mode; owner authorization is still required.",
            choice.as_str(),
            mode.as_str(),
        ),
        _ => eprintln!(
            "Existing machine retained without a new-assignment selection. Existing assignments keep their stored mode. Use --separation automatic, require-separated or shared-legacy to record intent for new assignments."
        ),
    }
    if !separated {
        if args.computer && cfg!(target_os = "linux") {
            eprintln!(
                "Linux computer use needs at-spi2-core, dbus-x11 and a session D-Bus: start the node inside your desktop session (or dbus-run-session). Install bubblewrap, Xvfb and openbox for the developer browser, which starts its own display and cookie. Chromium is launched with renderer accessibility enabled."
            );
        }
        if args.computer && cfg!(target_os = "macos") {
            eprintln!(
                "The secure browser uses Google Chrome. Install Google Chrome Beta for the separate developer browser; its managed policy permits DevTools and forbids saved-login filling. macOS browsers share your desktop and OS user, so they are not isolated from each other; use a separated Linux machine for sensitive accounts. Owner takeover locks both views."
            );
        }
        eprintln!(
            "Saved-login typing is off until the owner allows it in Assistant → Machines settings. Agent commands share the browser user's access and could read typed values. The machine container or separated VM is recommended."
        );
    }
    if !args.no_daemon {
        if !args.container {
            crate::commands::machine_native_update::prepare(
                &directory,
                profile,
                args.separate_users,
            )?;
        }
        if args.separate_users {
            install_supervisor(
                &directory,
                profile,
                args.computer || args.browser == Some(true),
            )?;
        } else {
            crate::node::daemon::install(directory.to_str(), profile, None, false)?;
            crate::node::daemon::start(directory.to_str(), profile)?;
        }
    }
    if !args.no_daemon && !args.container {
        crate::commands::machine_native_update::install(&directory, profile, args.separate_users)?;
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
    browser: bool,
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
    if browser && !computer {
        capabilities.push("browser");
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
    )?;
    if cfg!(target_os = "linux") && std::env::var_os("NYXID_MACHINE_CONTAINER").is_some() {
        let secure = super::process::Identity::resolve(Some("browser"))?;
        let dev = super::process::Identity::resolve(Some("devbrowser"))?;
        super::browser::install_container_policies(Path::new("/"), secure.gid, dev.gid)?;
    }
    Ok(())
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
    let dev = format!("nyxdev-{suffix}");
    for name in [&agent, &browser, &dev] {
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
    let binary = crate::commands::machine_native_update::prepare(directory, profile, true)?;
    let suffix = profile.unwrap_or("default");
    let unit = format!("nyxid-machine-{suffix}.service");
    let mut display = String::new();
    if computer {
        for binary in [
            "Xvfb",
            "xauth",
            "openbox",
            "dbus-run-session",
            "dbus-send",
            "bwrap",
        ] {
            if !std::process::Command::new("sh")
                .args(["-c", &format!("command -v {binary}")])
                .stdout(std::process::Stdio::null())
                .status()?
                .success()
            {
                bail!(
                    "Install xvfb, xauth, openbox, dbus-x11, at-spi2-core, bubblewrap and Chromium on this VM, then run setup --separate-users again"
                );
            }
        }
        if !Path::new("/usr/share/dbus-1/services/org.a11y.Bus.service").is_file() {
            bail!("Install at-spi2-core on this VM, then run setup --separate-users again");
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
        let session_script = Path::new("/opt/nyxid/bin/desktop-session");
        std::fs::write(
            session_script,
            include_str!("../../../container/desktop-session.sh"),
        )?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(session_script, std::fs::Permissions::from_mode(0o755))?;
        }
        let session_unit = format!("nyxid-desktop-{suffix}.service");
        std::fs::write(
            Path::new("/etc/systemd/system").join(&session_unit),
            format!(
                "[Unit]\nDescription=NyxID browser accessibility session\nRequires={display_unit}\nAfter={display_unit}\n[Service]\nUser={browser}\n{display}ExecStart={}\nRestart=on-failure\n[Install]\nWantedBy=multi-user.target\n",
                session_script.display()
            ),
        )?;
        let dev = config
            .machine
            .dev_browser_user
            .as_deref()
            .context("Developer browser user missing")?;
        let dev_number = number + 1000;
        let dev_identity = super::process::Identity::resolve(Some(dev))?;
        let dev_authority = dev_identity.home.join(".Xauthority");
        super::browser::create_xauthority(&dev_authority, dev, dev_number)?;
        let dev_display_unit = format!("nyxid-dev-display-{suffix}.service");
        let dev_session_unit = format!("nyxid-dev-desktop-{suffix}.service");
        std::fs::write(
            Path::new("/etc/systemd/system").join(&dev_display_unit),
            format!(
                "[Unit]\nDescription=NyxID developer display\n[Service]\nUser={dev}\nExecStart=/usr/bin/Xvfb :{dev_number} -screen 0 1280x800x24 -nolisten tcp -auth {}\nRestart=on-failure\n[Install]\nWantedBy=multi-user.target\n",
                dev_authority.display()
            ),
        )?;
        std::fs::write(
            Path::new("/etc/systemd/system").join(&dev_session_unit),
            format!(
                "[Unit]\nDescription=NyxID developer window manager\nRequires={dev_display_unit}\nAfter={dev_display_unit}\n[Service]\nUser={dev}\nEnvironment=DISPLAY=:{dev_number}\nEnvironment=XAUTHORITY={}\nExecStart=/usr/bin/openbox\nRestart=on-failure\n[Install]\nWantedBy=multi-user.target\n",
                dev_authority.display()
            ),
        )?;
        display.push_str(&format!(
            "Environment=NYXID_DEV_DISPLAY=:{dev_number}\nEnvironment=NYXID_DEV_XAUTHORITY={}\n",
            dev_authority.display()
        ));
        let status = std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status()?;
        if !status.success() {
            bail!("Could not reload machine display service");
        }
        let status = std::process::Command::new("systemctl")
            .args([
                "enable",
                "--now",
                &display_unit,
                &session_unit,
                &dev_display_unit,
                &dev_session_unit,
            ])
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

    fn setup_args(extra: &[&str]) -> Setup {
        use crate::cli::{Cli, Commands, NodeCommands};
        use clap::Parser;
        let mut args = vec!["nyxid", "node", "setup", "--machine"];
        args.extend_from_slice(extra);
        match Cli::try_parse_from(args).unwrap().command {
            Commands::Node {
                command: NodeCommands::Setup(args),
            } => args,
            _ => panic!("expected node setup"),
        }
    }

    #[test]
    fn new_setup_defaults_to_automatic_but_existing_profiles_preserve_intent() {
        use nyxid_machine::config::Config;
        assert_eq!(
            setup_choice(setup_args(&[]).separation, None),
            Some(Choice::Automatic)
        );
        assert_eq!(
            setup_choice(None, Some(&Config::default())),
            Some(Choice::Automatic)
        );
        let mut existing = Config {
            shell: true,
            ..Default::default()
        };
        assert_eq!(setup_choice(None, Some(&existing)), None);
        for stored in [
            Choice::Automatic,
            Choice::RequireSeparated,
            Choice::SharedLegacy,
        ] {
            existing.separation_choice = Some(stored);
            assert_eq!(setup_choice(None, Some(&existing)), Some(stored));
            for (flag, wanted) in [
                ("automatic", Choice::Automatic),
                ("require-separated", Choice::RequireSeparated),
                ("shared-legacy", Choice::SharedLegacy),
            ] {
                let args = setup_args(&["--separation", flag]);
                assert_eq!(setup_choice(args.separation, Some(&existing)), Some(wanted));
            }
        }
    }

    #[test]
    fn unsupported_setup_requires_explicit_legacy_choice_before_registration() {
        for choice in [Some(Choice::Automatic), Some(Choice::RequireSeparated)] {
            for (linux, uid, users, browser, reason) in [
                (false, 0, true, true, "separated_requires_linux"),
                (true, 501, true, true, "separated_requires_separate_users"),
                (true, 0, false, true, "separated_requires_separate_users"),
                (true, 0, true, false, "separated_requires_managed_browser"),
            ] {
                let error = preflight_selection(choice, linux, uid, users, browser, None)
                    .unwrap_err()
                    .to_string();
                assert!(error.starts_with(reason));
                assert!(error.contains("--separation shared-legacy"));
                for word in ["node tokens", "signing secrets", "stored credentials"] {
                    assert!(error.contains(word));
                }
            }
            assert!(preflight_selection(choice, true, 0, true, true, None).is_ok());
        }
        for choice in [None, Some(Choice::SharedLegacy)] {
            assert!(preflight_selection(choice, false, 501, false, false, None).is_ok());
        }
    }

    #[tokio::test]
    async fn unsupported_new_setup_fails_before_creating_registration_state() {
        let directory = tempfile::tempdir().unwrap();
        let mut args = setup_args(&[]);
        args.config = Some(directory.path().to_string_lossy().into_owned());
        args.url = "ws://127.0.0.1:9/api/v1/nodes/ws".into();
        args.token = Some("nyx_nreg_validation_fixture_only".into());
        args.no_daemon = true;
        let error = run(args).await.unwrap_err().to_string();
        assert!(
            error.starts_with(if cfg!(target_os = "linux") {
                "separated_requires_separate_users"
            } else {
                "separated_requires_linux"
            }),
            "{error}"
        );
        assert!(!directory.path().join("config.toml").exists());
    }

    #[tokio::test]
    async fn legacy_and_unselected_existing_setup_do_not_infer_separation() {
        let mut config = nyxid_machine::config::Config::default();
        assert_eq!(verify_selection(&config).await.unwrap(), None);
        config.separation_choice = Some(Choice::SharedLegacy);
        let encoded = toml::to_string(&config).unwrap();
        let config = toml::from_str(&encoded).unwrap();
        assert_eq!(
            verify_selection(&config).await.unwrap(),
            Some(Mode::SharedLegacy)
        );
    }

    #[test]
    fn node_only_machine_preset_has_no_old_alias() {
        use crate::cli::Cli;
        use clap::Parser;
        assert!(
            Cli::try_parse_from(["nyxid", "node", "setup", "--node-only-machine-preset"]).is_ok()
        );
        assert!(Cli::try_parse_from(["nyxid", "node", "setup", "--fixed-shell-files"]).is_err());
    }

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
