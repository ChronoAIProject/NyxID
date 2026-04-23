use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::api::CLI_USER_AGENT;
use crate::cli::{AiSetupCommands, AiToolTarget};

/// GitHub raw base URL for the NyxID repository.
const GITHUB_RAW: &str = "https://raw.githubusercontent.com/ChronoAIProject/NyxID/main";

/// Path within the repo to the canonical skill files.
const SKILL_DIR: &str = "skills/nyxid";

/// The default hosted NyxID URL used in the repo's SKILL.md.
/// Replaced with the user's actual server URL at install time.
const DEFAULT_HOSTED_URL: &str = "https://nyx-api.chrono-ai.fun";

/// The default hosted dashboard URL.
const DEFAULT_HOSTED_DASHBOARD: &str = "https://nyx.chrono-ai.fun";

pub async fn run(command: AiSetupCommands) -> Result<()> {
    match command {
        AiSetupCommands::Install { tool, base_url } => install(tool, &base_url).await,
        AiSetupCommands::Update { tool, base_url } => update(tool, &base_url).await,
        AiSetupCommands::Status => status(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_base_url(base_url: &Option<String>) -> Result<String> {
    let url = if let Some(url) = base_url {
        url.trim_end_matches('/').to_string()
    } else if let Some(url) = crate::auth::read_saved_base_url() {
        url.trim_end_matches('/').to_string()
    } else {
        bail!(
            "No base URL configured. Run `nyxid login --base-url <URL>` first, \
             or pass --base-url, or set NYXID_URL"
        )
    };

    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("Base URL must use http:// or https:// scheme, got: {url}");
    }

    Ok(url)
}

/// Max response size for fetched files (2 MiB).
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(CLI_USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")
}

/// Fetch a URL, validating status code and response size.
async fn fetch_url(url: &str) -> Result<String> {
    let resp = http_client()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch {url}"))?;

    if !resp.status().is_success() {
        bail!("HTTP {} from {url}", resp.status());
    }
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_RESPONSE_BYTES)
    {
        bail!("Response too large (max {MAX_RESPONSE_BYTES} bytes) from {url}");
    }

    resp.text()
        .await
        .with_context(|| format!("Failed to read body from {url}"))
}

/// Fetch a file from the GitHub repository.
async fn fetch_github(path: &str) -> Result<String> {
    fetch_url(&format!("{GITHUB_RAW}/{path}")).await
}

/// Fetch the playbook from the user's NyxID server (already has URL substitution).
async fn fetch_playbook(base_url: &str) -> Result<String> {
    fetch_url(&format!("{base_url}/llms.txt")).await
}

/// Replace default hosted URLs in content with the user's actual server URL.
fn substitute_urls(content: &str, base_url: &str, dashboard_url: &str) -> String {
    content
        .replace(DEFAULT_HOSTED_URL, base_url)
        .replace(DEFAULT_HOSTED_DASHBOARD, dashboard_url)
        .replace("http://localhost:3001", base_url)
        .replace("http://localhost:3000", dashboard_url)
}

async fn resolve_dashboard_url(base_url: &str) -> String {
    // ai-setup is a pre-flight informational command — no profile is
    // in scope here. Using `None` falls back to the default profile's
    // consent, which is the right behavior for a non-profile command.
    crate::auth::fetch_frontend_url(base_url, None)
        .await
        .unwrap_or_else(|_| base_url.to_string())
}

fn write_file(path: &PathBuf, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    std::fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    eprintln!("  Wrote {}", path.display());
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &PathBuf) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if path.exists() {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

fn cargo_home_dir(home: &Path) -> PathBuf {
    std::env::var("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".cargo"))
}

fn current_shell_name() -> String {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    PathBuf::from(shell)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "sh".into())
}

fn shell_rc_path(home: &Path, shell_name: &str) -> PathBuf {
    match shell_name {
        "zsh" => home.join(".zshrc"),
        "bash" => {
            if cfg!(target_os = "macos") {
                home.join(".bash_profile")
            } else {
                home.join(".bashrc")
            }
        }
        "fish" => std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".config"))
            .join("fish/config.fish"),
        _ => home.join(".profile"),
    }
}

fn shell_escape_double_quoted(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn cargo_path_is_configured(contents: &str, cargo_bin: &Path, cargo_env: &Path) -> bool {
    let cargo_bin = cargo_bin.to_string_lossy();
    let cargo_env = cargo_env.to_string_lossy();

    [
        cargo_bin.as_ref(),
        cargo_env.as_ref(),
        "$HOME/.cargo/bin",
        "$HOME/.cargo/env",
        "${HOME}/.cargo/bin",
        "${HOME}/.cargo/env",
        ".cargo/bin",
        ".cargo/env",
        "fish_add_path",
    ]
    .iter()
    .any(|needle| contents.contains(needle))
}

fn cargo_setup_command(shell_name: &str, cargo_bin: &Path, cargo_env: &Path) -> String {
    if shell_name == "fish" {
        format!(
            "fish_add_path \"{}\"",
            shell_escape_double_quoted(cargo_bin)
        )
    } else if cargo_env.exists() {
        format!(". \"{}\"", shell_escape_double_quoted(cargo_env))
    } else {
        format!(
            "export PATH=\"{}:$PATH\"",
            shell_escape_double_quoted(cargo_bin)
        )
    }
}

fn cargo_setup_rc_snippet(shell_name: &str, cargo_bin: &Path, cargo_env: &Path) -> String {
    format!(
        "\n# Cargo (Rust package manager) -- added by NyxID installer\n{}\n",
        cargo_setup_command(shell_name, cargo_bin, cargo_env)
    )
}

fn skill_paths(tool: AiToolTarget) -> Result<Vec<(String, PathBuf)>> {
    let home = home_dir()?;
    match tool {
        AiToolTarget::ClaudeCode => Ok(vec![(
            "skill".into(),
            home.join(".claude/skills/nyxid/SKILL.md"),
        )]),
        AiToolTarget::Cursor => Ok(vec![(
            "rule".into(),
            PathBuf::from(".cursor/rules/nyxid.mdc"),
        )]),
        AiToolTarget::Codex => Ok(vec![(
            "skill".into(),
            home.join(".codex/skills/nyxid/SKILL.md"),
        )]),
        AiToolTarget::Openclaw => Ok(vec![(
            "skill".into(),
            home.join(".openclaw/skills/nyxid/SKILL.md"),
        )]),
        AiToolTarget::Generic => Ok(vec![]),
    }
}

// ---------------------------------------------------------------------------
// Shared fetch: get SKILL.md from GitHub, playbook from server
// ---------------------------------------------------------------------------

struct SkillContent {
    skill_md: String,
    playbook: String,
    post_install: String,
    install_sh: String,
    services_sh: String,
    proxy_sh: String,
}

async fn fetch_skill_content(base_url: &str) -> Result<SkillContent> {
    let dashboard = resolve_dashboard_url(base_url).await;

    eprintln!("  Fetching skill from GitHub...");
    let skill_md_raw = fetch_github(&format!("{SKILL_DIR}/SKILL.md")).await?;
    let skill_md = substitute_urls(&skill_md_raw, base_url, &dashboard);

    let post_install = fetch_github(&format!("{SKILL_DIR}/POST_INSTALL.md"))
        .await
        .unwrap_or_default();

    eprintln!("  Fetching playbook from {base_url}/llms.txt...");
    let playbook = fetch_playbook(base_url).await?;

    let install_sh = fetch_github(&format!("{SKILL_DIR}/tools/install.sh")).await?;
    let services_sh = fetch_github(&format!("{SKILL_DIR}/tools/services.sh")).await?;
    let proxy_sh = fetch_github(&format!("{SKILL_DIR}/tools/proxy.sh")).await?;

    Ok(SkillContent {
        skill_md,
        playbook,
        post_install,
        install_sh,
        services_sh,
        proxy_sh,
    })
}

// ---------------------------------------------------------------------------
// PATH setup
// ---------------------------------------------------------------------------

/// Detect the user's shell RC file and ensure the active cargo bin directory is
/// in PATH. This is critical for non-technical users whose shell does not
/// source cargo env by default (e.g. after a fresh `cargo install`).
fn ensure_cargo_in_path() {
    let home = match home_dir() {
        Ok(h) => h,
        Err(_) => return,
    };

    let cargo_home = cargo_home_dir(&home);
    let cargo_bin = cargo_home.join("bin");
    let cargo_env = cargo_home.join("env");

    if let Some(path_var) = std::env::var_os("PATH")
        && std::env::split_paths(&path_var).any(|path| path == cargo_bin)
    {
        return;
    }

    // If the binary doesn't exist at all, skip PATH setup.
    if !cargo_bin.join("nyxid").exists() {
        return;
    }

    let shell_name = current_shell_name();
    let rc_file = shell_rc_path(&home, &shell_name);

    if let Ok(contents) = std::fs::read_to_string(&rc_file)
        && cargo_path_is_configured(&contents, &cargo_bin, &cargo_env)
    {
        return;
    }

    let line = cargo_setup_rc_snippet(&shell_name, &cargo_bin, &cargo_env);

    if let Some(parent) = rc_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&rc_file)
    {
        Ok(mut f) => {
            use std::io::Write;
            if f.write_all(line.as_bytes()).is_ok() {
                eprintln!(
                    "  Added {} to PATH in {}",
                    cargo_bin.display(),
                    rc_file.display()
                );
                eprintln!("  Open a new terminal or run: source {}", rc_file.display());
            }
        }
        Err(e) => {
            eprintln!("  [warn] Could not update {}: {e}", rc_file.display());
            eprintln!(
                "  Add this to your shell config manually: {}",
                cargo_setup_command(&shell_name, &cargo_bin, &cargo_env)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

async fn install(tool: AiToolTarget, base_url: &Option<String>) -> Result<()> {
    let base = resolve_base_url(base_url)?;
    eprintln!("Installing NyxID skill for {tool}...");

    let content = fetch_skill_content(&base).await?;

    let result = match tool {
        AiToolTarget::ClaudeCode => install_claude_code(&content).await,
        AiToolTarget::Cursor => install_cursor(&content),
        AiToolTarget::Codex => install_codex(&content),
        AiToolTarget::Openclaw => install_openclaw(&content),
        AiToolTarget::Generic => {
            bail!(
                "Generic tool has no skill files. Use `nyxid ai-setup agent create` to create an agent identity instead."
            )
        }
    };

    if result.is_ok() {
        // After installing the skill, ensure the active cargo bin directory is
        // in PATH so that `skills check` can find the nyxid binary.
        ensure_cargo_in_path();
    }

    result
}

/// Print the post-install summary plus tool-specific notes.
fn print_post_install(tool: AiToolTarget, content: &SkillContent) {
    // Tool-specific restart / activation notes
    match tool {
        AiToolTarget::ClaudeCode => {
            eprintln!(
                "Use /nyxid in Claude Code, or just ask about NyxID and it activates automatically."
            );
        }
        AiToolTarget::Cursor => {
            eprintln!(
                "Cursor rules are project-level. Run this command in each project you want NyxID in."
            );
        }
        AiToolTarget::Codex => {
            eprintln!("Start a new Codex session to load the skill.");
        }
        AiToolTarget::Openclaw => {
            eprintln!("To activate, start a new chat in OpenClaw.");
            eprintln!();

            // Check if the gateway is installed as a service
            let gateway_installed = std::process::Command::new("openclaw")
                .args(["gateway", "status"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());

            if !gateway_installed {
                eprintln!("Tip: Install the OpenClaw gateway as a background service so it");
                eprintln!("stays running and restarts automatically:");
                eprintln!();
                eprintln!("  openclaw gateway install");
                eprintln!("  openclaw gateway start");
                eprintln!();
                eprintln!("You can verify it's running with: openclaw gateway status");
                eprintln!();
            }

            eprintln!("Verify skill: `openclaw skills check` (should show NyxID as ready).");
        }
        AiToolTarget::Generic => {}
    }

    eprintln!();
    eprintln!("To update the skill later: nyxid ai-setup update");
    eprintln!(
        "To update the CLI itself: cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli"
    );

    if !content.post_install.is_empty() {
        eprintln!();
        for line in content.post_install.lines() {
            eprintln!("{line}");
        }
    }
}

async fn install_claude_code(content: &SkillContent) -> Result<()> {
    let dir = home_dir()?.join(".claude/skills/nyxid");

    write_file(&dir.join("SKILL.md"), &content.skill_md)?;
    write_file(&dir.join("references/playbook.md"), &content.playbook)?;

    eprintln!();
    print_post_install(AiToolTarget::ClaudeCode, content);
    Ok(())
}

fn install_cursor(content: &SkillContent) -> Result<()> {
    let mdc = format!(
        "---\ndescription: NyxID auth/SSO platform -- credential brokering, services, API keys, nodes, SSH, MCP\nglobs:\nalwaysApply: true\n---\n\n{}\n",
        content.skill_md
    );
    write_file(&PathBuf::from(".cursor/rules/nyxid.mdc"), &mdc)?;

    eprintln!();
    print_post_install(AiToolTarget::Cursor, content);
    Ok(())
}

fn install_codex(content: &SkillContent) -> Result<()> {
    let dir = home_dir()?.join(".codex/skills/nyxid");

    write_file(&dir.join("SKILL.md"), &content.skill_md)?;
    write_file(&dir.join("references/playbook.md"), &content.playbook)?;

    eprintln!();
    print_post_install(AiToolTarget::Codex, content);
    Ok(())
}

fn install_openclaw(content: &SkillContent) -> Result<()> {
    let dir = home_dir()?.join(".openclaw/skills/nyxid");

    write_file(&dir.join("SKILL.md"), &content.skill_md)?;
    write_file(&dir.join("references/playbook.md"), &content.playbook)?;
    write_file(&dir.join("tools/install.sh"), &content.install_sh)?;
    write_file(&dir.join("tools/services.sh"), &content.services_sh)?;
    write_file(&dir.join("tools/proxy.sh"), &content.proxy_sh)?;

    #[cfg(unix)]
    {
        make_executable(&dir.join("tools/install.sh"))?;
        make_executable(&dir.join("tools/services.sh"))?;
        make_executable(&dir.join("tools/proxy.sh"))?;
    }

    eprintln!();
    print_post_install(AiToolTarget::Openclaw, content);
    Ok(())
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

async fn update(tool: Option<AiToolTarget>, base_url: &Option<String>) -> Result<()> {
    let all_tools = [
        AiToolTarget::ClaudeCode,
        AiToolTarget::Cursor,
        AiToolTarget::Codex,
        AiToolTarget::Openclaw,
    ];

    let tools: Vec<AiToolTarget> = match tool {
        Some(AiToolTarget::Generic) => {
            bail!("Generic tool has no skill files to update.")
        }
        Some(t) => vec![t],
        None => all_tools.to_vec(),
    };

    let base = resolve_base_url(base_url)?;
    let mut installed_tools = Vec::new();

    // Check which tools are installed before fetching
    for &t in &tools {
        let paths = skill_paths(t)?;
        let installed = paths.iter().any(|(_, p)| p.exists());
        if installed {
            installed_tools.push(t);
        } else if tool.is_some() {
            bail!(
                "No NyxID skill installed for {t}. Run `nyxid ai-setup install --tool {t}` first."
            );
        }
    }

    if installed_tools.is_empty() {
        eprintln!("No installed skills found. Run `nyxid ai-setup install --tool <tool>` first.");
        return Ok(());
    }

    // Fetch once, install to all
    let content = fetch_skill_content(&base).await?;

    for t in installed_tools {
        eprintln!("Updating {t}...");
        match t {
            AiToolTarget::ClaudeCode => install_claude_code(&content).await?,
            AiToolTarget::Cursor => install_cursor(&content)?,
            AiToolTarget::Codex => install_codex(&content)?,
            AiToolTarget::Openclaw => install_openclaw(&content)?,
            AiToolTarget::Generic => unreachable!(),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

fn status() -> Result<()> {
    use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};

    let all_tools = [
        AiToolTarget::ClaudeCode,
        AiToolTarget::Cursor,
        AiToolTarget::Codex,
        AiToolTarget::Openclaw,
    ];

    let home = home_dir().unwrap_or_default();
    let home_str = home.to_string_lossy();

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["Agent", "Type", "Path", "Status"]);

    for tool in all_tools {
        let paths = skill_paths(tool)?;

        for (label, path) in &paths {
            let status_str = if path.exists() {
                let date = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<Utc> = t.into();
                        dt.format("%Y-%m-%d").to_string()
                    })
                    .unwrap_or_else(|| "unknown".into());
                format!("installed ({date})")
            } else {
                "not installed".into()
            };

            let display_path = path.to_string_lossy().replace(&*home_str, "~");

            table.add_row([
                &tool.to_string(),
                label.as_str(),
                &display_path,
                &status_str,
            ]);
        }
    }

    eprintln!("{table}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        cargo_path_is_configured, cargo_setup_command, shell_escape_double_quoted, shell_rc_path,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn cargo_path_detection_matches_default_home_entries() {
        let cargo_bin = Path::new("/tmp/example/.cargo/bin");
        let cargo_env = Path::new("/tmp/example/.cargo/env");

        assert!(cargo_path_is_configured(
            ". \"$HOME/.cargo/env\"\n",
            cargo_bin,
            cargo_env
        ));
        assert!(cargo_path_is_configured(
            "export PATH=\"$HOME/.cargo/bin:$PATH\"\n",
            cargo_bin,
            cargo_env
        ));
    }

    #[test]
    fn cargo_path_detection_matches_custom_cargo_home_entries() {
        let cargo_bin = Path::new("/opt/nyx cargo/bin");
        let cargo_env = Path::new("/opt/nyx cargo/env");

        assert!(cargo_path_is_configured(
            "export PATH=\"/opt/nyx cargo/bin:$PATH\"\n",
            cargo_bin,
            cargo_env
        ));
        assert!(cargo_path_is_configured(
            ". \"/opt/nyx cargo/env\"\n",
            cargo_bin,
            cargo_env
        ));
    }

    #[test]
    fn bash_rc_path_matches_platform_convention() {
        let home = Path::new("/tmp/home");
        let expected = if cfg!(target_os = "macos") {
            home.join(".bash_profile")
        } else {
            home.join(".bashrc")
        };

        assert_eq!(shell_rc_path(home, "bash"), expected);
    }

    #[test]
    fn cargo_setup_command_uses_absolute_custom_cargo_home() {
        let test_root = temp_test_dir();
        let cargo_home = test_root.join("custom cargo home");
        let cargo_bin = cargo_home.join("bin");
        let cargo_env = cargo_home.join("env");

        fs::create_dir_all(&cargo_bin).unwrap();
        fs::write(&cargo_env, "").unwrap();

        let command = cargo_setup_command("bash", &cargo_bin, &cargo_env);
        let expected_env = shell_escape_double_quoted(&cargo_env);

        assert_eq!(command, format!(". \"{expected_env}\""));

        fs::remove_dir_all(test_root).unwrap();
    }

    fn temp_test_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nyxid-ai-setup-tests-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn ai_tool_target_display_generic() {
        assert_eq!(super::AiToolTarget::Generic.to_string(), "generic");
    }

    #[test]
    fn skill_paths_generic_returns_empty() {
        let paths = super::skill_paths(super::AiToolTarget::Generic).unwrap();
        assert!(paths.is_empty());
    }
}
