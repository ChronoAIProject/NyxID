//! One command registers, enables local authority and starts the node service.
use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use nyxid_machine::context::{Choice, Mode};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{BufRead, IsTerminal, Read, Write},
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
    #[arg(long, value_enum, env = "NYXID_MACHINE_SEPARATION")]
    pub separation: Option<Separation>,
    /// Disposable Docker preflight: report real capabilities without pairing,
    /// registration, persistent volumes or service startup.
    #[arg(long, hide = true)]
    pub probe_separation: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
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

impl Separation {
    pub(crate) fn as_cli_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::RequireSeparated => "require-separated",
            Self::SharedLegacy => "shared-legacy",
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
        "{reason}: {} To explicitly accept shared legacy access, rerun this command with --separation shared-legacy after reviewing this warning: {}",
        nyxid_machine::context::unavailable_message(reason),
        nyxid_machine::context::SHARED_LEGACY_WARNING,
    )
}

/// Match the CLI's login/prompt convention: both input and diagnostic output
/// must be terminals. A required choice must never read input or offer fallback.
pub(crate) fn offer_shared_legacy(choice: Choice, reason: &str) -> Result<Choice> {
    let input = std::io::stdin();
    let output = std::io::stderr();
    offer_shared_legacy_with(
        choice,
        reason,
        input.is_terminal(),
        output.is_terminal(),
        &mut input.lock(),
        &mut output.lock(),
    )
}

fn offer_shared_legacy_with(
    choice: Choice,
    reason: &str,
    input_tty: bool,
    output_tty: bool,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Choice> {
    if choice != Choice::Automatic || !input_tty || !output_tty {
        return Err(selection_error(reason));
    }
    writeln!(
        output,
        "{reason}: {}",
        nyxid_machine::context::unavailable_message(reason)
    )?;
    writeln!(output, "{}", nyxid_machine::context::SHARED_LEGACY_WARNING)?;
    write!(output, "Continue with shared legacy mode? [y/N] ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if answer.trim().eq_ignore_ascii_case("y") {
        return Ok(Choice::SharedLegacy);
    }
    Err(selection_error(reason)
        .context("Shared legacy mode was not selected; automatic intent is retained"))
}

const PENDING_CHOICE: &str = "machine-separation-choice.json";

fn pending_choice(directory: &Path) -> Result<Option<Choice>> {
    use std::os::unix::{fs::MetadataExt, fs::OpenOptionsExt};
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(PENDING_CHOICE))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 128,
        "Invalid pending machine separation choice; expected a private owner-controlled file"
    );
    let mut encoded = String::new();
    file.take(129).read_to_string(&mut encoded)?;
    Ok(Some(serde_json::from_str(&encoded)?))
}

/// No registration-shaped config is created before owner consent. Retain a
/// failed or accepted pre-pairing choice in a small private, atomic sidecar.
fn persist_setup_choice(
    directory: &Path,
    existing: &mut Option<crate::node::config::NodeConfig>,
    choice: Option<Choice>,
) -> Result<()> {
    if let Some(config) = existing {
        if config.machine.separation_choice != choice {
            config.machine.separation_choice = choice;
            config.save(&directory.join("config.toml"))?;
        }
    } else if let Some(choice) = choice {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(&mut file, &choice)?;
        file.as_file().sync_all()?;
        file.persist(directory.join(PENDING_CHOICE))?;
    }
    Ok(())
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
) -> std::result::Result<(), &'static str> {
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
        return Err(reason);
    }
    Ok(())
}

async fn verify_selection(
    directory: &Path,
    config: &nyxid_machine::config::Config,
) -> std::result::Result<Option<Mode>, &'static str> {
    let Some(choice) = config.separation_choice else {
        return Ok(None);
    };
    if choice == Choice::SharedLegacy {
        return Ok(Some(Mode::SharedLegacy));
    }
    let identity = |name| {
        super::process::Identity::resolve(name).map_err(|_| "separated_requires_separate_users")
    };
    let command = identity(config.agent_user.as_deref())?;
    let secure = identity(config.browser_user.as_deref())?;
    let developer = config
        .effective_dev_browser_user()
        .map(|name| identity(Some(name)))
        .transpose()?;
    let custody = crate::node::custody::Custody::new(directory);
    custody
        .observe(&[
            Some(command.clone()),
            Some(secure.clone()),
            developer.clone(),
        ])
        .await;
    let mut evidence =
        super::separation::observe(config, &command, &secure, developer.as_ref()).await;
    evidence.credential_custody = Some(custody.status().all_denied());
    nyxid_machine::context::select_new(choice, Some(&evidence)).map(Some)
}

async fn container_evidence() -> Result<nyxid_machine::context::Evidence> {
    use super::process::Identity;
    let command = Identity::resolve(Some("agent")).or_else(|_| Identity::resolve(None))?;
    let secure = Identity::resolve(Some("browser")).or_else(|_| Identity::resolve(None))?;
    let developer = Identity::resolve(Some("devbrowser")).ok();
    let temporary = tempfile::tempdir()?;
    let config = nyxid_machine::config::Config {
        managed_browser: browser_binary().ok().map(|binary| {
            nyxid_machine::config::ManagedBrowserConfig {
                binary,
                data_dir: temporary.path().to_path_buf(),
                update_port: 32248,
                container: true,
            }
        }),
        ..Default::default()
    };
    let custody = crate::node::custody::preflight_files(&[
        Some(command.clone()),
        Some(secure.clone()),
        developer.clone(),
    ])
    .await;
    let mut evidence =
        super::separation::observe(&config, &command, &secure, developer.as_ref()).await;
    evidence.credential_custody = Some(custody);
    Ok(evidence)
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
    if args.probe_separation {
        // The Docker caller uses the same image, identities and seccomp policy
        // as startup. Setup will probe again after actual provisioning.
        let evidence = if args.container {
            container_evidence().await?
        } else {
            let directory = crate::node::config::resolve_config_dir_with_profile(
                args.config.as_deref(),
                args.profile.as_deref(),
            )?;
            let config = crate::node::config::NodeConfig::load(&directory.join("config.toml"))?;
            let command = super::process::Identity::resolve(config.machine.agent_user.as_deref())?;
            let secure = super::process::Identity::resolve(config.machine.browser_user.as_deref())?;
            let developer = config
                .machine
                .effective_dev_browser_user()
                .map(|name| super::process::Identity::resolve(Some(name)))
                .transpose()?;
            let custody = crate::node::custody::Custody::new(&directory);
            custody
                .observe(&[
                    Some(command.clone()),
                    Some(secure.clone()),
                    developer.clone(),
                ])
                .await;
            let mut evidence =
                super::separation::observe(&config.machine, &command, &secure, developer.as_ref())
                    .await;
            evidence.credential_custody = Some(custody.status().all_denied());
            evidence
        };
        println!("{}", serde_json::to_string(&evidence)?);
        return Ok(());
    }
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
    #[cfg(target_os = "linux")]
    if args.separate_users && !args.container && args.config.is_none() {
        super::context_runtime::prepare_native_profile(&directory)?;
    }
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
    let mut existing = config_path
        .exists()
        .then(|| crate::node::config::NodeConfig::load(&config_path))
        .transpose()?;
    let mut choice = setup_choice(
        args.separation,
        existing.as_ref().map(|config| &config.machine),
    );
    if existing.is_none() && args.separation.is_none() {
        choice = pending_choice(&directory)?.or(choice);
    }
    persist_setup_choice(&directory, &mut existing, choice)?;
    if let Err(reason) = preflight_selection(
        choice,
        cfg!(target_os = "linux"),
        unsafe { libc::geteuid() },
        separated,
        args.browser.unwrap_or(args.computer) && !args.skip_browser_policy,
        existing.as_ref().map(|config| &config.machine),
    ) {
        choice = Some(offer_shared_legacy(
            choice.context("missing separation choice")?,
            reason,
        )?);
        persist_setup_choice(&directory, &mut existing, choice)?;
    }
    #[cfg(target_os = "linux")]
    if args.browser.unwrap_or(args.computer) && !args.skip_browser_policy {
        super::browser_layout::Layout::detect(&browser_binary()?)?;
    }
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
    match std::fs::remove_file(directory.join(PENDING_CHOICE)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
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
            browser_installer(
                &std::env::current_exe()?,
                port,
                cfg!(target_os = "linux")
                    .then(browser_binary)
                    .transpose()?
                    .as_deref(),
            )
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
    let mut config = crate::node::config::NodeConfig::load(&config_path)?;
    let mode = match verify_selection(&directory, &config.machine).await {
        Ok(mode) => mode,
        Err(reason) => {
            let choice = offer_shared_legacy(
                config
                    .machine
                    .separation_choice
                    .context("missing separation choice")?,
                reason,
            )?;
            config.machine.separation_choice = Some(choice);
            config.save(&config_path)?;
            Some(Mode::SharedLegacy)
        }
    };
    match (config.machine.separation_choice, mode) {
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
    let explicit = if cfg!(target_os = "linux") {
        std::env::var_os("NYXID_MACHINE_BROWSER").map(PathBuf::from)
    } else {
        None
    };
    select_browser_binary(explicit.as_deref(), paths)
}

fn select_browser_binary(explicit: Option<&Path>, candidates: &[&str]) -> Result<PathBuf> {
    if let Some(binary) = explicit {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            binary.is_absolute(),
            "NYXID_MACHINE_BROWSER must be an absolute executable path"
        );
        let metadata = binary.metadata().context(
            "NYXID_MACHINE_BROWSER is unavailable; install a supported browser package or correct the selected path",
        )?;
        anyhow::ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            "NYXID_MACHINE_BROWSER must name an executable file"
        );
        // Linux setup still validates the resolved package and policy layout.
        // An invalid explicit choice must never fall back to another browser.
        return Ok(binary.to_owned());
    }
    candidates.iter().map(PathBuf::from).find(|p|p.is_file()).context("Install Chromium (or Google Chrome on macOS), then run setup again to enable saved-login filling")
}

fn browser_installer(
    executable: &Path,
    port: u16,
    browser: Option<&Path>,
) -> std::process::Command {
    let mut command = std::process::Command::new("sudo");
    if let Some(browser) = browser {
        // sudo normally strips custom environment variables. Forward only the
        // selected package path; the privileged installer validates it again.
        let mut selection = std::ffi::OsString::from("NYXID_MACHINE_BROWSER=");
        selection.push(browser);
        command.arg("env").arg(selection);
    }
    command.arg(executable).args([
        "node",
        "machine-browser-install",
        "--port",
        &port.to_string(),
    ]);
    command
}

pub fn install_browser(port: u16) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("Managed browser installation needs administrator access");
    }
    if port == 0 {
        bail!("Managed extension update port must be fixed");
    }
    let layout = if cfg!(target_os = "linux") {
        super::browser_layout::Layout::detect(&browser_binary()?)?
    } else {
        super::browser_layout::Layout::GoogleChrome
    };
    let installed = install_supervisor_binary()?;
    super::browser::install(
        Path::new("/"),
        &installed,
        &format!("http://127.0.0.1:{port}/update.xml"),
        cfg!(target_os = "macos"),
        layout,
    )?;
    if cfg!(target_os = "linux") && std::env::var_os("NYXID_MACHINE_CONTAINER").is_some() {
        let secure = super::process::Identity::resolve(Some("browser"))?;
        let dev = super::process::Identity::resolve(Some("devbrowser"))?;
        super::browser::install_container_policies(Path::new("/"), layout, secure.gid, dev.gid)?;
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

    #[test]
    fn privileged_browser_installer_receives_the_selected_package_only() {
        let executable = Path::new("/opt/nyxid/bin/nyxid");
        let command =
            browser_installer(executable, 32248, Some(Path::new("/usr/bin/google-chrome")));
        assert_eq!(command.get_program(), "sudo");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "env",
                "NYXID_MACHINE_BROWSER=/usr/bin/google-chrome",
                "/opt/nyxid/bin/nyxid",
                "node",
                "machine-browser-install",
                "--port",
                "32248",
            ]
        );
        let mac = browser_installer(executable, 32248, None);
        assert_eq!(mac.get_args().next().unwrap(), executable);
    }

    #[test]
    fn explicit_browser_selection_wins_over_installed_candidates() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let shim = root.path().join("chromium-browser");
        let chrome = root.path().join("google-chrome");
        for binary in [&shim, &chrome] {
            std::fs::write(binary, "browser").unwrap();
            std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let candidates = [shim.to_str().unwrap(), chrome.to_str().unwrap()];
        assert_eq!(select_browser_binary(None, &candidates).unwrap(), shim);
        assert_eq!(
            select_browser_binary(Some(&chrome), &candidates).unwrap(),
            chrome
        );
    }

    #[test]
    fn invalid_explicit_browser_fails_without_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("google-chrome");
        std::fs::write(&installed, "browser").unwrap();
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o644)).unwrap();
        let candidates = [installed.to_str().unwrap()];
        for selected in [
            Path::new(""),
            Path::new("google-chrome"),
            &root.path().join("missing"),
            root.path(),
            &installed,
        ] {
            assert!(select_browser_binary(Some(selected), &candidates).is_err());
        }
    }

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
                    .map_err(selection_error)
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
        assert_eq!(
            pending_choice(directory.path()).unwrap(),
            Some(Choice::Automatic)
        );
    }

    #[test]
    fn automatic_tty_yes_requires_the_full_warning_and_records_explicit_legacy() {
        let directory = tempfile::tempdir().unwrap();
        let mut existing = None;
        persist_setup_choice(directory.path(), &mut existing, Some(Choice::Automatic)).unwrap();
        let mut output = Vec::new();
        let selected = offer_shared_legacy_with(
            Choice::Automatic,
            "separated_requires_linux",
            true,
            true,
            &mut std::io::Cursor::new(b"y\n"),
            &mut output,
        )
        .unwrap();
        assert_eq!(selected, Choice::SharedLegacy);
        persist_setup_choice(directory.path(), &mut existing, Some(selected)).unwrap();
        assert_eq!(
            pending_choice(directory.path()).unwrap(),
            Some(Choice::SharedLegacy)
        );
        assert!(!directory.path().join("config.toml").exists());
        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("separated_requires_linux:"));
        assert!(output.contains(nyxid_machine::context::SHARED_LEGACY_WARNING));
        for text in [
            "node tokens",
            "signing secrets",
            "stored credentials",
            "share the host kernel",
            "container or VM per agent",
        ] {
            assert!(output.contains(text));
        }
        assert!(output.ends_with("Continue with shared legacy mode? [y/N] "));
    }

    #[test]
    fn automatic_tty_no_empty_or_eof_preserves_automatic() {
        for answer in ["n\n", "\n", "", "yes\n"] {
            let directory = tempfile::tempdir().unwrap();
            persist_setup_choice(directory.path(), &mut None, Some(Choice::Automatic)).unwrap();
            let mut output = Vec::new();
            assert!(
                offer_shared_legacy_with(
                    Choice::Automatic,
                    "separated_probe_timeout",
                    true,
                    true,
                    &mut std::io::Cursor::new(answer),
                    &mut output,
                )
                .is_err()
            );
            assert_eq!(
                pending_choice(directory.path()).unwrap(),
                Some(Choice::Automatic)
            );
            assert!(
                String::from_utf8(output)
                    .unwrap()
                    .contains("separated_probe_timeout")
            );
        }
    }

    struct MustNotRead;
    impl Read for MustNotRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("noninteractive or required selection must never read input")
        }
    }

    #[test]
    fn automatic_non_tty_never_prompts_and_names_exact_flag_and_reason() {
        for (input_tty, output_tty) in [(false, false), (true, false), (false, true)] {
            let mut output = Vec::new();
            let error = offer_shared_legacy_with(
                Choice::Automatic,
                "separated_requires_managed_browser",
                input_tty,
                output_tty,
                &mut std::io::BufReader::new(MustNotRead),
                &mut output,
            )
            .unwrap_err()
            .to_string();
            assert!(output.is_empty());
            assert!(error.contains("--separation shared-legacy"));
            assert!(error.contains("separated_requires_managed_browser"));
        }
    }

    #[test]
    fn required_never_prompts_even_with_both_terminals() {
        let mut output = Vec::new();
        let error = offer_shared_legacy_with(
            Choice::RequireSeparated,
            "separated_requires_linux",
            true,
            true,
            &mut std::io::BufReader::new(MustNotRead),
            &mut output,
        )
        .unwrap_err()
        .to_string();
        assert!(output.is_empty());
        assert!(error.starts_with("separated_requires_linux"));
    }

    #[test]
    fn pending_selection_rejects_symlinks_and_non_private_or_invalid_state() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(PENDING_CHOICE);
        let target = directory.path().join("target");
        std::fs::write(&target, "\"shared_legacy\"").unwrap();
        symlink(&target, &path).unwrap();
        assert!(pending_choice(directory.path()).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "\"shared_legacy\"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(pending_choice(directory.path()).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            pending_choice(directory.path()).unwrap(),
            Some(Choice::SharedLegacy)
        );
        std::fs::write(&path, "\"unknown\"").unwrap();
        assert!(pending_choice(directory.path()).is_err());
    }

    #[test]
    fn node_only_entrypoint_forwards_selection_and_preserves_existing_profile() {
        use std::os::unix::fs::PermissionsExt;
        for (choice, setup_status, existing) in [
            (None, 23, false),
            (Some("automatic"), 23, false),
            (Some("require-separated"), 23, false),
            (Some("shared-legacy"), 0, false),
            (Some("unknown"), 0, false),
            (Some(""), 0, false),
            (None, 0, true),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let state = directory.path().join("state");
            let node = state.join("node");
            std::fs::create_dir_all(&node).unwrap();
            let config = node.join("config.toml");
            if existing {
                std::fs::write(&config, "shell = true\n").unwrap();
            }
            let source = include_str!("../../../container/machine-node-only-entrypoint.sh")
                .replace("/var/lib/nyxid-machine", state.to_str().unwrap())
                .replace(
                    "/workspace",
                    directory.path().join("workspace").to_str().unwrap(),
                )
                .replace(
                    "/home/agent",
                    directory.path().join("home").to_str().unwrap(),
                );
            let script = directory.path().join("entrypoint.sh");
            std::fs::write(&script, source).unwrap();
            let bin = directory.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            for (name, source) in [
                ("chown", "#!/bin/sh\nexit 0\n"),
                (
                    "nyxid",
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$TEST_CALLS\"\nif [ \"$2\" = setup ]; then\n  [ \"$TEST_SETUP_STATUS\" = 0 ] || exit \"$TEST_SETUP_STATUS\"\n  printf 'shell = true\\n' > \"$TEST_CONFIG\"\nfi\n",
                ),
            ] {
                let path = bin.join(name);
                std::fs::write(&path, source).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let calls = directory.path().join("calls");
            let mut command = std::process::Command::new("/bin/sh");
            command
                .arg(script)
                .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
                .env("NYXID_MACHINE_MANAGED", "0")
                .env("NYXID_NODE_TOKEN", "fixture")
                .env("NYXID_NODE_URL", "wss://example.test/api/v1/nodes/ws")
                .env("TEST_CALLS", &calls)
                .env("TEST_CONFIG", &config)
                .env("TEST_SETUP_STATUS", setup_status.to_string())
                .env_remove("NYXID_MACHINE_SEPARATION");
            if let Some(choice) = choice {
                command.env("NYXID_MACHINE_SEPARATION", choice);
            }
            let output = command.output().unwrap();
            let calls = std::fs::read_to_string(calls).unwrap_or_default();
            if matches!(choice, Some("unknown" | "")) {
                assert_eq!(output.status.code(), Some(2));
                assert!(calls.is_empty());
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("NYXID_MACHINE_SEPARATION must be")
                );
            } else if existing {
                assert!(output.status.success());
                assert!(!calls.contains("setup"));
                assert_eq!(std::fs::read_to_string(config).unwrap(), "shell = true\n");
            } else {
                assert!(calls.contains(&format!(
                    "--separation\n{}\n",
                    choice.unwrap_or("automatic")
                )));
                assert_eq!(output.status.code(), Some(setup_status));
                assert_eq!(calls.contains("start"), setup_status == 0);
            }
        }
    }

    #[tokio::test]
    async fn legacy_and_unselected_existing_setup_do_not_infer_separation() {
        let mut config = nyxid_machine::config::Config::default();
        assert_eq!(
            verify_selection(Path::new("/unused"), &config)
                .await
                .unwrap(),
            None
        );
        config.separation_choice = Some(Choice::SharedLegacy);
        let encoded = toml::to_string(&config).unwrap();
        let config = toml::from_str(&encoded).unwrap();
        assert_eq!(
            verify_selection(Path::new("/unused"), &config)
                .await
                .unwrap(),
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
