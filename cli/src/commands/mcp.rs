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
                    let config = serde_json::json!({
                        "mcp_url": mcp_url,
                        "tool": tool,
                        "base_url": base,
                    });
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
