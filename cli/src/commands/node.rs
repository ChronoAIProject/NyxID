use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{NodeCommands, NodeDaemonCommands, NodeDockerCommands, OutputFormat};

pub async fn run(command: NodeCommands) -> Result<()> {
    match command {
        // --- User-side commands (API calls) ---
        NodeCommands::List { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let nodes: Value = api.get("/nodes").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&nodes)?);
                }
                OutputFormat::Table => {
                    let items = nodes
                        .get("nodes")
                        .and_then(|v| v.as_array())
                        .or_else(|| nodes.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No nodes registered.");
                            eprintln!(
                                "Use `nyxid node register-token` to create a registration token."
                            );
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["ID", "Name", "Status", "Last Seen"]);

                        for node in items {
                            let id = node["id"].as_str().or(node["_id"].as_str()).unwrap_or("-");
                            let name = node["name"].as_str().unwrap_or("-");
                            let status = node["status"].as_str().unwrap_or("-");
                            let last_seen = node["last_heartbeat_at"].as_str().unwrap_or("-");
                            table.add_row([id, name, status, last_seen]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        NodeCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;

            // Try direct ID first; if 404, resolve by name
            let node: Value = match api.get_value(&format!("/nodes/{id}")).await {
                Ok(n) => n,
                Err(_) => {
                    // Try to find by name
                    let nodes: Value = api.get("/nodes").await?;
                    let items = nodes
                        .get("nodes")
                        .and_then(|v| v.as_array())
                        .or_else(|| nodes.as_array());
                    let found =
                        items.and_then(|arr| arr.iter().find(|n| n["name"].as_str() == Some(&id)));
                    match found {
                        Some(n) => {
                            let node_id =
                                n["id"].as_str().or(n["_id"].as_str()).ok_or_else(|| {
                                    anyhow::anyhow!("Node '{id}' found but has no ID")
                                })?;
                            api.get(&format!("/nodes/{node_id}")).await?
                        }
                        None => anyhow::bail!("Node '{id}' not found (tried as ID and name)"),
                    }
                }
            };

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&node)?);
                }
                OutputFormat::Table => {
                    let name = node["name"].as_str().unwrap_or("-");
                    let node_id = node["id"].as_str().or(node["_id"].as_str()).unwrap_or(&id);
                    let status = node["status"].as_str().unwrap_or("-");
                    let last_seen = node["last_heartbeat_at"].as_str().unwrap_or("-");
                    let version = node
                        .get("metadata")
                        .and_then(|m| m["agent_version"].as_str())
                        .unwrap_or("-");

                    eprintln!("Name:       {name}");
                    eprintln!("ID:         {node_id}");
                    eprintln!("Status:     {status}");
                    eprintln!("Last Seen:  {last_seen}");
                    eprintln!("Version:    {version}");

                    if let Some(metrics) = node.get("metrics") {
                        let total = metrics["total_requests"].as_u64().unwrap_or(0);
                        let success = metrics["success_count"].as_u64().unwrap_or(0);
                        let errors = metrics["error_count"].as_u64().unwrap_or(0);
                        eprintln!();
                        eprintln!("Metrics:");
                        eprintln!("  Total Requests: {total}");
                        eprintln!("  Success:        {success}");
                        eprintln!("  Errors:         {errors}");
                    }
                }
            }
            Ok(())
        }

        NodeCommands::RegisterToken { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;

            eprint!("Node name: ");
            std::io::stderr().flush()?;
            let mut name_input = String::new();
            std::io::stdin().read_line(&mut name_input)?;
            let node_name = name_input.trim();
            let node_name = if node_name.is_empty() {
                "my-node"
            } else {
                node_name
            };

            let result: Value = api
                .post(
                    "/nodes/register-token",
                    &serde_json::json!({ "name": node_name }),
                )
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let token = result["token"].as_str().unwrap_or("-");
                    let expires = result["expires_at"].as_str().unwrap_or("in 1 hour");

                    eprintln!("Registration token created.");
                    eprintln!();
                    eprintln!("Token:   {token}");
                    eprintln!("Expires: {expires}");
                    eprintln!();
                    eprintln!("Register a node:");
                    eprintln!(
                        "  nyxid node register --token {token} --url ws://<server>/api/v1/nodes/ws"
                    );
                }
            }
            Ok(())
        }

        NodeCommands::Delete { id, yes, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let resolved_id = resolve_node_id(&mut api, &id).await?;

            if !yes {
                eprint!("Delete node {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            api.delete_empty(&format!("/nodes/{resolved_id}")).await?;
            eprintln!("Node deleted.");
            Ok(())
        }

        NodeCommands::RotateToken { id, terminal, auth } => {
            use std::io::IsTerminal;
            // Wizard mode (v3 DisplayOnce) when output is interactive,
            // stdout is a TTY, and the environment can open a local
            // browser. Mirrors the v2 `service add` gate. Anything else
            // (--terminal, --output json, piped, SSH, NYXID_NO_WIZARD)
            // falls through to the scripted path BELOW, byte-identical
            // to pre-wizard behavior.
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && interactive_output
                && std::io::stdout().is_terminal()
                && crate::wizard::is_wizard_eligible();

            if wizard_eligible {
                let mut api = ApiClient::from_auth(&auth)?;
                let resolved_id = resolve_node_id(&mut api, &id).await?;
                // Best-effort fetch of the display name for the confirm
                // panel. Falls back to id if the GET fails.
                let display_name = match api.get::<Value>(&format!("/nodes/{resolved_id}")).await {
                    Ok(node) => node["name"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| resolved_id.clone()),
                    Err(_) => resolved_id.clone(),
                };
                let prefill = crate::wizard::RotatePrefill {
                    resource_id: resolved_id,
                    display_name,
                };
                return crate::wizard::run_node_rotate_token_wizard(&auth, prefill).await;
            }

            // Scripted / headless path — UNCHANGED from pre-wizard behavior.
            let mut api = ApiClient::from_auth(&auth)?;
            let resolved_id = resolve_node_id(&mut api, &id).await?;
            let result: Value = api
                .post(
                    &format!("/nodes/{resolved_id}/rotate-token"),
                    &serde_json::json!({}),
                )
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let token = result["token"]
                        .as_str()
                        .or(result["auth_token"].as_str())
                        .unwrap_or("-");
                    eprintln!("Node token rotated.");
                    eprintln!("New Token: {token}  (save this -- shown only once)");
                    eprintln!();
                    eprintln!("Update the node agent configuration:");
                    eprintln!("  nyxid node rekey --auth-token {token} --signing-secret <HEX>");
                }
            }
            Ok(())
        }

        // --- Agent-side commands (local node operations) ---
        NodeCommands::Register {
            token,
            url,
            config,
            keychain,
            profile,
        } => {
            let effective_config = resolve_effective_config(config.as_deref(), profile.as_deref())?;
            crate::node::agent::cmd_register(
                &token,
                url.as_deref(),
                effective_config.as_deref(),
                keychain,
            )
            .await
            .map_err(anyhow::Error::from)
        }

        NodeCommands::Start {
            config,
            log_level,
            profile,
        } => {
            let effective_config = resolve_effective_config(config.as_deref(), profile.as_deref())?;
            crate::node::agent::cmd_start(effective_config.as_deref(), log_level.as_deref())
                .await
                .map_err(anyhow::Error::from)
        }

        NodeCommands::AgentStatus { config, profile } => {
            let effective_config = resolve_effective_config(config.as_deref(), profile.as_deref())?;
            crate::node::agent::cmd_status(effective_config.as_deref()).map_err(anyhow::Error::from)
        }

        NodeCommands::Rekey {
            auth_token,
            signing_secret,
            config,
            profile,
        } => {
            let effective_config = resolve_effective_config(config.as_deref(), profile.as_deref())?;
            crate::node::agent::cmd_rekey(&auth_token, &signing_secret, effective_config.as_deref())
                .map_err(anyhow::Error::from)
        }

        NodeCommands::Credentials {
            command,
            config,
            profile,
        } => {
            let effective_config = resolve_effective_config(config.as_deref(), profile.as_deref())?;
            crate::node::agent::cmd_credentials(command, effective_config.as_deref())
                .await
                .map_err(anyhow::Error::from)
        }

        NodeCommands::Migrate {
            to,
            config,
            profile,
        } => {
            let effective_config = resolve_effective_config(config.as_deref(), profile.as_deref())?;
            crate::node::agent::cmd_migrate(&to, effective_config.as_deref())
                .map_err(anyhow::Error::from)
        }

        NodeCommands::NodeOpenclaw {
            command,
            config,
            profile,
        } => {
            let effective_config = resolve_effective_config(config.as_deref(), profile.as_deref())?;
            crate::node::agent::cmd_openclaw(command, effective_config.as_deref())
                .await
                .map_err(anyhow::Error::from)
        }

        NodeCommands::AgentVersion => {
            crate::node::agent::cmd_version();
            Ok(())
        }

        NodeCommands::Docker { command } => run_docker_command(command),

        NodeCommands::Daemon { command } => match command {
            NodeDaemonCommands::Install {
                args,
                log_level,
                force,
            } => crate::node::daemon::install(
                args.config.as_deref(),
                args.profile.as_deref(),
                log_level.as_deref(),
                force,
            )
            .map_err(anyhow::Error::from),
            NodeDaemonCommands::Uninstall { args } => {
                crate::node::daemon::uninstall(args.config.as_deref(), args.profile.as_deref())
                    .map_err(anyhow::Error::from)
            }
            NodeDaemonCommands::Start { args } => {
                crate::node::daemon::start(args.config.as_deref(), args.profile.as_deref())
                    .map_err(anyhow::Error::from)
            }
            NodeDaemonCommands::Stop { args } => {
                crate::node::daemon::stop(args.config.as_deref(), args.profile.as_deref())
                    .map_err(anyhow::Error::from)
            }
            NodeDaemonCommands::Restart { args } => {
                crate::node::daemon::restart(args.config.as_deref(), args.profile.as_deref())
                    .map_err(anyhow::Error::from)
            }
            NodeDaemonCommands::Status { args } => {
                crate::node::daemon::status(args.config.as_deref(), args.profile.as_deref())
                    .map_err(anyhow::Error::from)
            }
            NodeDaemonCommands::Logs {
                args,
                follow,
                lines,
            } => crate::node::daemon::logs(
                args.config.as_deref(),
                args.profile.as_deref(),
                follow,
                lines,
            )
            .map_err(anyhow::Error::from),
        },
    }
}

/// Resolve the effective config path, applying profile if no explicit config was given.
fn resolve_effective_config(
    config: Option<&str>,
    profile: Option<&str>,
) -> anyhow::Result<Option<String>> {
    if config.is_some() {
        return Ok(config.map(String::from));
    }
    match profile {
        Some(p) => Ok(Some(
            crate::node::config::resolve_config_dir_with_profile(None, Some(p))
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .to_string_lossy()
                .to_string(),
        )),
        None => Ok(None),
    }
}

/// Resolve a node identifier (ID or name) to a node ID.
async fn resolve_node_id(api: &mut ApiClient, id_or_name: &str) -> Result<String> {
    // Try as UUID first (quick check)
    if id_or_name.len() == 36 && id_or_name.contains('-') {
        return Ok(id_or_name.to_string());
    }

    // List nodes and find by name
    let nodes: Value = api.get("/nodes").await?;
    let items = nodes
        .get("nodes")
        .and_then(|v| v.as_array())
        .or_else(|| nodes.as_array());

    if let Some(arr) = items
        && let Some(node) = arr.iter().find(|n| n["name"].as_str() == Some(id_or_name))
        && let Some(nid) = node["id"].as_str().or(node["_id"].as_str())
    {
        return Ok(nid.to_string());
    }

    // Fall back to treating it as an ID (let the server decide)
    Ok(id_or_name.to_string())
}

// ---- Docker subcommands ----

const DOCKER_IMAGE: &str = "nyxid-node:latest";
const DOCKER_CONFIG_DIR: &str = "/app/config";

fn docker_container_name(profile: Option<&str>) -> String {
    match profile {
        None | Some("default") => "nyxid-node".to_string(),
        Some(name) => format!("nyxid-node-{name}"),
    }
}

fn docker_config_dir(profile: Option<&str>) -> Result<std::path::PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let base = home.join(".nyxid-node");
    match profile {
        None | Some("default") => Ok(base),
        Some(name) => {
            crate::auth::validate_profile_name(name)?;
            Ok(base.join("profiles").join(name))
        }
    }
}

fn run_docker_command(command: NodeDockerCommands) -> Result<()> {
    // Verify docker is available
    let docker_check = std::process::Command::new("docker")
        .arg("--version")
        .output();
    if docker_check.is_err() {
        anyhow::bail!(
            "Docker is not installed or not in PATH. \
             Install Docker from https://docs.docker.com/get-docker/"
        );
    }

    match command {
        NodeDockerCommands::Build => docker_build(),
        NodeDockerCommands::Start { args } => docker_start(args.profile.as_deref()),
        NodeDockerCommands::Stop { args } => docker_stop(args.profile.as_deref()),
        NodeDockerCommands::Restart { args } => {
            let _ = docker_stop(args.profile.as_deref());
            docker_start(args.profile.as_deref())
        }
        NodeDockerCommands::Status { args } => docker_status(args.profile.as_deref()),
        NodeDockerCommands::Logs { args, follow } => docker_logs(args.profile.as_deref(), follow),
    }
}

fn docker_build() -> Result<()> {
    eprintln!("Building node agent Docker image...");

    // Find the project root by looking for cli/Dockerfile.node
    let dockerfile = find_dockerfile()?;
    let context = dockerfile
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| anyhow::anyhow!("Could not determine project root from Dockerfile path"))?;

    let status = std::process::Command::new("docker")
        .args(["build", "-f"])
        .arg(&dockerfile)
        .args(["-t", DOCKER_IMAGE])
        .arg(context)
        .status()?;

    if !status.success() {
        anyhow::bail!("Docker build failed");
    }
    eprintln!("Image built: {DOCKER_IMAGE}");
    Ok(())
}

fn docker_start(profile: Option<&str>) -> Result<()> {
    let config_dir = docker_config_dir(profile)?;
    let container = docker_container_name(profile);

    if !config_dir.join("config.toml").exists() {
        let profile_hint = match profile {
            Some(p) if p != "default" => format!(" --profile {p}"),
            _ => String::new(),
        };
        anyhow::bail!(
            "No config found at {}. Register the node first:\n  nyxid node register --token <token> --url <ws-url>{profile_hint}",
            config_dir.display()
        );
    }

    // Check if image exists, prompt to build if not
    let image_check = std::process::Command::new("docker")
        .args(["image", "inspect", DOCKER_IMAGE])
        .output()?;
    if !image_check.status.success() {
        eprintln!("Image {DOCKER_IMAGE} not found. Building...");
        docker_build()?;
    }

    // Remove existing stopped container with the same name
    let _ = std::process::Command::new("docker")
        .args(["rm", "-f", &container])
        .output();

    let config_dir_str = config_dir.to_string_lossy();
    let volume = format!("{config_dir_str}:{DOCKER_CONFIG_DIR}:rw");

    let status = std::process::Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            &container,
            "--restart",
            "unless-stopped",
            "-v",
            &volume,
            DOCKER_IMAGE,
        ])
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to start container {container}");
    }
    eprintln!("Container {container} started.");
    eprintln!("  Logs:   nyxid node docker logs{}", profile_flag(profile));
    eprintln!("  Stop:   nyxid node docker stop{}", profile_flag(profile));
    eprintln!(
        "  Status: nyxid node docker status{}",
        profile_flag(profile)
    );
    Ok(())
}

fn docker_stop(profile: Option<&str>) -> Result<()> {
    let container = docker_container_name(profile);
    eprintln!("Stopping {container}...");
    let _ = std::process::Command::new("docker")
        .args(["stop", &container])
        .output();
    let _ = std::process::Command::new("docker")
        .args(["rm", &container])
        .output();
    eprintln!("Stopped.");
    Ok(())
}

fn docker_status(profile: Option<&str>) -> Result<()> {
    let container = docker_container_name(profile);
    let output = std::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=^{container}$"),
            "--format",
            "{{.Status}}",
        ])
        .output()?;
    let status_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if status_str.is_empty() {
        eprintln!("{container}: not found");
    } else if status_str.starts_with("Up") {
        eprintln!("{container}: running ({status_str})");
    } else {
        eprintln!("{container}: stopped ({status_str})");
    }
    Ok(())
}

fn docker_logs(profile: Option<&str>, follow: bool) -> Result<()> {
    let container = docker_container_name(profile);
    let mut cmd = std::process::Command::new("docker");
    cmd.args(["logs", "--tail", "50"]);
    if follow {
        cmd.arg("-f");
    }
    cmd.arg(&container);
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("Container {container} not found. Is it running?");
    }
    Ok(())
}

fn profile_flag(profile: Option<&str>) -> String {
    match profile {
        Some(p) if p != "default" => format!(" --profile {p}"),
        _ => String::new(),
    }
}

fn find_dockerfile() -> Result<std::path::PathBuf> {
    // Try relative to current exe (installed via cargo install)
    if let Ok(exe) = std::env::current_exe() {
        // Walk up looking for cli/Dockerfile.node
        let mut dir = exe.parent().map(std::path::Path::to_path_buf);
        for _ in 0..5 {
            if let Some(ref d) = dir {
                let candidate = d.join("cli/Dockerfile.node");
                if candidate.exists() {
                    return Ok(candidate);
                }
                dir = d.parent().map(std::path::Path::to_path_buf);
            }
        }
    }

    // Try current working directory
    let cwd = std::env::current_dir()?;
    let candidate = cwd.join("cli/Dockerfile.node");
    if candidate.exists() {
        return Ok(candidate);
    }

    anyhow::bail!(
        "Could not find cli/Dockerfile.node. Run this command from the NyxID project root, \
         or build the image manually:\n  docker build -f cli/Dockerfile.node -t {DOCKER_IMAGE} ."
    );
}
