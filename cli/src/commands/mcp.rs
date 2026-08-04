use anyhow::Result;

use crate::api::ApiClient;
use crate::cli::{McpCommands, OutputFormat};

pub async fn run(command: McpCommands) -> Result<()> {
    match command {
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
