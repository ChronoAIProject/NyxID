use std::io::Write;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OpenClawCommands, OutputFormat};

pub async fn run(command: OpenClawCommands) -> Result<()> {
    match command {
        OpenClawCommands::Setup {
            url,
            token_env,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            let gateway_url = match url {
                Some(u) => u,
                None => {
                    eprint!("OpenClaw gateway URL: ");
                    std::io::stderr().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    let trimmed = input.trim().to_string();
                    if trimmed.is_empty() {
                        bail!("Gateway URL is required");
                    }
                    trimmed
                }
            };

            let credential = if let Some(env_var) = &token_env {
                std::env::var(env_var)
                    .with_context(|| format!("Environment variable {env_var} not set"))?
            } else {
                rpassword::prompt_password("Bearer token: ")?
            };
            if credential.is_empty() {
                bail!("Bearer token is required");
            }

            let body = serde_json::json!({
                "service_slug": "llm-openclaw",
                "credential": credential,
                "endpoint_url": gateway_url,
                "label": "OpenClaw",
            });

            let result: Value = api.post("/keys", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let slug = result["slug"]
                        .as_str()
                        .or(result["service_slug"].as_str())
                        .unwrap_or("llm-openclaw");
                    let endpoint = result["endpoint_url"].as_str().unwrap_or(&gateway_url);
                    let status = result["status"].as_str().unwrap_or("active");

                    eprintln!("OpenClaw configured!");
                    eprintln!();
                    eprintln!("Slug:      {slug}");
                    eprintln!("Endpoint:  {endpoint}");
                    eprintln!("Status:    {status}");
                    eprintln!();
                    eprintln!("Proxy URL: {}/api/v1/proxy/s/{slug}/", api.base_url_root());
                    eprintln!();
                    eprintln!("Generate MCP config:");
                    eprintln!(
                        "  nyxid mcp config --tool claude-code --base-url {}",
                        auth.resolved_base_url()?
                    );
                }
            }
            Ok(())
        }
    }
}
