use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use nyxid_machine::ComputerMode;
use std::path::PathBuf;

#[derive(Clone, Copy, ValueEnum)]
pub enum Mode {
    Standard,
    Unrestricted,
}

#[derive(Args)]
pub struct Enable {
    #[arg(long)]
    pub shell: bool,
    #[arg(long)]
    pub files: bool,
    #[arg(long)]
    pub computer: bool,
    /// Explicit browser ceiling; omitted inherits the computer setting.
    #[arg(long, num_args=0..=1, default_missing_value="true")]
    pub browser: Option<bool>,
    #[arg(long = "root")]
    pub roots: Vec<PathBuf>,
    #[arg(long, value_enum, default_value = "standard")]
    pub computer_mode: Mode,
    #[arg(long)]
    pub allow_root: bool,
    #[arg(long)]
    pub cua_driver: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Give agents machine access; with no capability flags, enable shell and files.
    Enable(Enable),
    /// Disable capabilities locally. Restart the daemon to apply.
    Disable {
        #[arg(long)]
        shell: bool,
        #[arg(long)]
        files: bool,
        #[arg(long)]
        computer: bool,
        #[arg(long)]
        browser: bool,
        #[arg(long)]
        all: bool,
    },
    /// Show capabilities, roots, driver readiness and machine safety guidance.
    Status,
}

pub async fn run(command: Commands, config: Option<&str>, profile: Option<&str>) -> Result<()> {
    let directory = crate::node::config::resolve_config_dir_with_profile(config, profile)?;
    let path = directory.join("config.toml");
    let mut config = crate::node::config::NodeConfig::load(&path)?;
    match command {
        Commands::Enable(args) => {
            let defaults = !args.shell && !args.files && !args.computer && args.browser.is_none();
            let identity = super::process::Identity::resolve(config.machine.agent_user.as_deref())?;
            let browser =
                super::process::Identity::resolve(config.machine.browser_user.as_deref())?;
            if !args.allow_root
                && (((args.shell || defaults) && identity.uid == 0)
                    || ((args.computer || args.browser == Some(true)) && browser.uid == 0))
            {
                bail!(
                    "Commands or computer input would run as root. Prefer the machine container or a separated VM; pass --allow-root to explicitly accept full root access."
                );
            }
            config.machine.shell |= args.shell || defaults;
            config.machine.files |= args.files || defaults;
            config.machine.computer |= args.computer;
            if let Some(browser) = args.browser {
                config.machine.browser = Some(browser);
            }
            config.machine.allow_root |= args.allow_root;
            if !args.roots.is_empty() {
                config.machine.roots = args.roots;
            }
            if config.machine.roots.is_empty() {
                config
                    .machine
                    .roots
                    .push(identity.home.join("nyxid-workspace"));
            }
            for root in &mut config.machine.roots {
                std::fs::create_dir_all(&*root)?;
                *root = root.canonicalize()?;
            }
            if args.computer || args.browser == Some(true) {
                config.machine.computer_mode = match args.computer_mode {
                    Mode::Standard => ComputerMode::Standard,
                    Mode::Unrestricted => ComputerMode::Unrestricted,
                };
                if matches!(args.computer_mode, Mode::Unrestricted) {
                    eprintln!(
                        "WARNING: unrestricted cua mode bypasses driver approval prompts and permits full desktop control. Use a disposable machine."
                    );
                }
                let binary = if let Some(path) = args.cua_driver {
                    super::cua::verify_version(&path).await?;
                    path.canonicalize()?
                } else {
                    super::cua::install(&directory).await?
                };
                config.machine.cua_driver = Some(binary);
            }
            config.machine.validate().map_err(anyhow::Error::msg)?;
            config.save(&path)?;
            eprintln!(
                "Machine access enabled. Agents have the command user's full permissions; roots constrain file tools and working directories, not the shell. A VM or container is recommended because prompt injection is possible. Restart the node daemon to apply."
            );
        }
        Commands::Disable {
            shell,
            files,
            computer,
            browser,
            all,
        } => {
            if !shell && !files && !computer && !browser && !all {
                bail!("choose --shell, --files, --browser, --computer or --all");
            }
            if shell || all {
                config.machine.shell = false;
            }
            if files || all {
                config.machine.files = false;
            }
            if browser || all {
                config.machine.browser = Some(false);
            }
            if computer || all {
                config.machine.computer = false;
            }
            config.save(&path)?;
            eprintln!("Capabilities disabled locally. Restart the node daemon to apply.");
        }
        Commands::Status => {
            // Never construct a Runtime or probe drivers here: a separate
            // process must not bind the daemon's socket or launch Chromium.
            let cached = read_status(&directory);
            let identity = super::process::Identity::resolve(config.machine.agent_user.as_deref())?;
            let commands_isolated = identity
                .commands_isolated(&[
                    directory.clone(),
                    dirs::home_dir().unwrap_or_default().join(".nyxid-node"),
                ])
                .await;
            let mut status = match cached.as_ref() {
                Some(row) => serde_json::to_value(row)?,
                None => serde_json::json!({
                    "shell": config.machine.shell, "files": config.machine.files,
                    "computer": config.machine.computer, "browser":config.machine.browser_enabled(), "roots": config.machine.roots,
                    "readiness": "unknown; start the node daemon to publish live status",
                }),
            };
            status["commands_isolated"] = serde_json::json!(commands_isolated);
            println!("{}", serde_json::to_string_pretty(&status)?);
            eprintln!(
                "Read-only status: cached daemon observation; no browsers or live probes started."
            );
            if config.machine.agent_user.is_none() {
                eprintln!(
                    "Saved logins require managed browser policies and owner opt-in on the Assistant → Machines page. Commands run as the browser user, so a misbehaving or prompt-injected agent could read typed values. Prefer the machine container or a separated VM."
                );
            }
            if config.machine.shell && !commands_isolated {
                eprintln!(
                    "Not isolated: agent commands can read this node's stored credentials, signing secret and node token, including its config and local credential store. Prefer the container or --separate-users; you may continue on this machine."
                );
            }
            if config.machine.shell {
                eprintln!(
                    "Shell commands have this OS user's full access. Workspace roots constrain only file tools and working directories."
                );
            }
            if config.machine.allow_root {
                eprintln!("WARNING: root access is explicitly allowed.");
            }
            if (config.machine.computer || config.machine.browser_enabled())
                && cfg!(target_os = "macos")
            {
                eprintln!(
                    "The computer_permissions fields report Screen Recording and Accessibility for the running driver. With direct MCP, macOS attributes these grants to the app launching the node (for example Terminal), so enable that app in System Settings and restart the node. Saved-login filling also needs admin-installed managed browser policies."
                );
            }
        }
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedStatus {
    observed_at_ms: i64,
    #[serde(flatten)]
    profile: nyxid_machine::MachineProfile,
}
fn read_status(directory: &std::path::Path) -> Option<CachedStatus> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut bytes = Vec::new();
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("machine-status.json"))
        .ok()?
        .take(128 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}
