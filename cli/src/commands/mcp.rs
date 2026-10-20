mod client;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::api::ApiClient;
use crate::cli::{McpCommands, OutputFormat};

pub async fn run(command: McpCommands) -> Result<()> {
    match command {
        McpCommands::Tools { service, auth } => {
            let mut api = super::billing_support::human_api(&auth).await?;
            let service_tools = match service.as_deref() {
                Some(slug) => {
                    let catalog: Value = api.get("/mcp/config").await?;
                    let services = catalog["services"]
                        .as_array()
                        .context("MCP catalog has no services array")?;
                    let rows: Vec<_> = services
                        .iter()
                        .filter(|row| row["service_slug"] == slug)
                        .collect();
                    if rows.len() != 1 {
                        bail!("Expected exactly one visible service matching {slug}");
                    }
                    Some(
                        rows[0]["endpoints"]
                            .as_array()
                            .context("MCP catalog service has no endpoints array")?
                            .iter()
                            .map(|endpoint| {
                                endpoint["name"]
                                    .as_str()
                                    .map(|name| format!("{slug}__{name}"))
                                    .context("MCP catalog operation has no name")
                            })
                            .collect::<Result<Vec<_>>>()?,
                    )
                }
                None => None,
            };
            let mut client = client::Client::for_discovery(api);
            let mut tools = vec![];
            let mut cursor = None;
            let mut seen = std::collections::BTreeSet::new();
            loop {
                let params = cursor
                    .as_ref()
                    .map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
                let result = client.request("tools/list", params).await?;
                tools.extend(
                    result["tools"]
                        .as_array()
                        .context("MCP tools/list has no tools array")?
                        .iter()
                        .cloned(),
                );
                cursor = result["nextCursor"].as_str().map(str::to_owned);
                let Some(next) = &cursor else {
                    break;
                };
                if !seen.insert(next.clone()) {
                    bail!("MCP tools/list returned a repeated cursor");
                }
            }
            if let Some(names) = service_tools {
                let lazy = tools.iter().any(|tool| tool["name"] == "nyx__search_tools");
                let mut complete = Vec::with_capacity(names.len());
                for name in names {
                    let listed = tools.iter().find(|tool| tool["name"] == name).cloned();
                    let tool = match listed {
                        Some(tool) => tool,
                        None if lazy => {
                            // The catalog enumerates every operation. Search each exact
                            // name for its server-built schema, and fail on any omission.
                            let params = json!({
                                "name": "nyx__search_tools", "arguments": {"query": name}
                            });
                            let result = client.request("tools/call", params).await?;
                            if result["isError"] == true {
                                bail!("MCP schema discovery failed for {name}");
                            }
                            result["content"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|content| content["text"].as_str())
                                .filter_map(|text| serde_json::from_str::<Value>(text).ok())
                                .flat_map(|value| {
                                    value["matches"].as_array().cloned().unwrap_or_default()
                                })
                                .find(|tool| tool["name"] == name)
                                .with_context(|| {
                                    format!("MCP discovery is incomplete: no schema for {name}")
                                })?
                        }
                        None => bail!("MCP discovery is incomplete: no schema for {name}"),
                    };
                    if !tool["inputSchema"].is_object() {
                        bail!("MCP discovery is incomplete: invalid schema for {name}");
                    }
                    complete.push(tool);
                }
                tools = complete;
            }
            let result = json!({"tools":tools});
            super::billing_support::print(&result, auth.output, || {
                super::billing_support::table(
                    &["Tool", "Description", "Input schema"],
                    tools.iter().map(|tool| {
                        ["name", "description", "inputSchema"]
                            .iter()
                            .map(|key| super::billing_support::text(&tool[*key]))
                            .collect()
                    }),
                )
            })
        }
        McpCommands::Call {
            tool,
            args,
            envelope,
            timeout_secs,
            auth,
        } => {
            super::billing_support::require_human(&auth)?;
            let raw = if let Some(path) = args.strip_prefix('@') {
                std::fs::read_to_string(path).context("Failed to read MCP arguments file")?
            } else {
                args
            };
            let args: Value =
                serde_json::from_str(&raw).context("MCP --args must be a JSON object or @file")?;
            if !args.is_object() {
                bail!("MCP --args must be a JSON object");
            }
            let api = super::billing_support::human_api(&auth).await?;
            let mut client = client::Client::new(api);
            let params = if tool.starts_with("nyx__") || tool.starts_with("nyxid__") {
                json!({"name":tool,"arguments":args})
            } else {
                // The universal dispatcher activates the service in this fresh
                // session and applies the same execution authorization gates.
                json!({
                    "name": "nyx__call_tool",
                    "arguments": {
                        "tool_name": tool,
                        "arguments_json": serde_json::to_string(&args)?
                    }
                })
            };
            let started_at = super::billing_support::rfc3339(chrono::Utc::now());
            let request = client.request("tools/call", params);
            let result = if timeout_secs == 0 {
                request.await?
            } else {
                match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), request)
                    .await
                {
                    Ok(result) => result?,
                    Err(_) => {
                        if !client.call_dispatched() {
                            bail!(
                                "MCP call timed out before tools/call was dispatched; nothing was executed"
                            );
                        }
                        let finished_at = super::billing_support::rfc3339(chrono::Utc::now());
                        let value = json!({
                            "error": "managed_outcome_uncertain",
                            "tool": tool,
                            "started_at": started_at,
                            "finished_at": finished_at,
                            "message": concat!(
                                "MCP call timed out; inspect billing executions within these bounds before retrying. ",
                                "The provider may have completed and charged the call."
                            )
                        });
                        super::billing_support::print(&value, auth.output, || {
                            super::billing_support::fields(&value)
                        })?;
                        return Err(super::admin_tools::PendingReadiness { exit_code: 2 }.into());
                    }
                }
            };
            let finished_at = super::billing_support::rfc3339(chrono::Utc::now());
            let is_error = result["isError"] == true;
            if is_error
                && let Some(id) = result["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|content| content["text"].as_str())
                    .find_map(super::billing_support::pending_id)
            {
                return super::billing_support::pending(id, auth.output);
            }
            let value = if envelope {
                json!({
                    "tool": tool,
                    "started_at": started_at,
                    "finished_at": finished_at,
                    "result": result
                })
            } else {
                result
            };
            super::billing_support::print(&value, auth.output, || {
                super::billing_support::fields(&value)
            })?;
            if is_error {
                return Err(super::admin_tools::PendingReadiness { exit_code: 1 }.into());
            }
            Ok(())
        }
        McpCommands::Discover { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let catalog: serde_json::Value = api.get("/mcp/config").await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&catalog)?),
                OutputFormat::Table => {
                    let mut table = comfy_table::Table::new();
                    table.load_preset(comfy_table::presets::UTF8_FULL_CONDENSED);
                    table.set_header(["Service", "Slug", "Operations", "Generic proxy"]);
                    if let Some(services) = catalog["services"].as_array() {
                        for service in services {
                            table.add_row([
                                service["service_name"].as_str().unwrap_or("-").to_owned(),
                                service["service_slug"].as_str().unwrap_or("-").to_owned(),
                                service["endpoints"]
                                    .as_array()
                                    .map_or(0, Vec::len)
                                    .to_string(),
                                service["is_generic_proxy"]
                                    .as_bool()
                                    .unwrap_or(false)
                                    .to_string(),
                            ]);
                        }
                    }
                    println!("{table}");
                    eprintln!(concat!(
                        "Use --output json for operation IDs, schemas, recommended skills and discovery diagnostics. ",
                        "Execution still checks live permissions and provider availability."
                    ));
                }
            }
            Ok(())
        }
        McpCommands::Config { tool, auth } => {
            let api = ApiClient::from_auth(&auth)?;
            let base = api.base_url_root();
            let mcp_url = format!("{base}/mcp");

            match auth.output {
                OutputFormat::Json => {
                    let config = mcp_config_json(&tool, base);
                    println!("{}", serde_json::to_string_pretty(&config)?);
                }
                OutputFormat::Table => match tool.as_str() {
                    "cursor" => {
                        println!("Add to .cursor/mcp.json:");
                        println!();
                        println!("{{");
                        println!("  \"mcpServers\": {{");
                        println!("    \"nyxid\": {{");
                        println!("      \"url\": \"{mcp_url}\"");
                        println!("    }}");
                        println!("  }}");
                        println!("}}");
                        println!();
                        println!("Restart Cursor. Authenticate via browser when prompted.");
                    }
                    "claude-code" => {
                        println!("Run in terminal:");
                        println!();
                        println!("  claude mcp add --transport http --scope user nyxid {mcp_url}");
                        println!();
                        println!("Or add to .claude/settings.json:");
                        println!();
                        println!("{{");
                        println!("  \"mcpServers\": {{");
                        println!("    \"nyxid\": {{");
                        println!("      \"type\": \"http\",");
                        println!("      \"url\": \"{mcp_url}\"");
                        println!("    }}");
                        println!("  }}");
                        println!("}}");
                        println!();
                        println!("Restart Claude Code. Authenticate via browser when prompted.");
                    }
                    "vscode" => {
                        println!("Add to .vscode/mcp.json:");
                        println!();
                        println!("{{");
                        println!("  \"servers\": {{");
                        println!("    \"nyxid\": {{");
                        println!("      \"type\": \"http\",");
                        println!("      \"url\": \"{mcp_url}\"");
                        println!("    }}");
                        println!("  }}");
                        println!("}}");
                        println!();
                        println!("Authenticate via browser when prompted.");
                    }
                    "codex" => {
                        println!("Run in terminal:");
                        println!();
                        println!("  codex mcp add nyxid --url {mcp_url}");
                        println!();
                        println!("Or add to ~/.codex/config.toml:");
                        println!();
                        println!("[mcp_servers.nyxid]");
                        println!("url = \"{mcp_url}\"");
                        println!();
                        println!("Then run: codex mcp login nyxid");
                    }
                    _ => {
                        println!("MCP Server URL: {mcp_url}");
                        println!();
                        println!("Authentication is handled via OAuth.");
                        println!("Your MCP client will open a browser window to authenticate");
                        println!("when it connects for the first time.");
                    }
                },
            }
            Ok(())
        }
    }
}

/// Build the machine-readable MCP config object emitted by
/// `--output json`. Kept pure so the agent-facing contract — the
/// `mcp_url` value clients copy into their config — is unit-testable
/// without capturing stdout. `mcp_url` is always `{base}/mcp`.
fn mcp_config_json(tool: &str, base: &str) -> serde_json::Value {
    serde_json::json!({
        "mcp_url": format!("{base}/mcp"),
        "tool": tool,
        "base_url": base,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_config_json_derives_url_from_base() {
        let v = mcp_config_json("claude-code", "https://auth.nyxid.dev");
        assert_eq!(v["mcp_url"], "https://auth.nyxid.dev/mcp");
        assert_eq!(v["tool"], "claude-code");
        assert_eq!(v["base_url"], "https://auth.nyxid.dev");
    }
}

#[cfg(test)]
#[path = "mcp/tests.rs"]
mod transport_tests;
