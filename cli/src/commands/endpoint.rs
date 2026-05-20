use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{EndpointCommands, OutputFormat};

pub async fn run(command: EndpointCommands) -> Result<()> {
    match command {
        EndpointCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let endpoints: Value = api.get("/endpoints").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&endpoints)?);
                }
                OutputFormat::Table => {
                    let items = endpoints
                        .get("endpoints")
                        .and_then(|v| v.as_array())
                        .or_else(|| endpoints.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No endpoints.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["ID", "Label", "URL"]);

                        for ep in items {
                            let id = ep["id"].as_str().or(ep["_id"].as_str()).unwrap_or("-");
                            let short_id = if id.len() > 8 { &id[..8] } else { id };
                            let label = ep["label"].as_str().or(ep["name"].as_str()).unwrap_or("-");
                            let url = ep["url"]
                                .as_str()
                                .or(ep["base_url"].as_str())
                                .unwrap_or("-");
                            table.add_row([short_id, label, url]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        EndpointCommands::Update { id, url, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let body = serde_json::json!({ "url": url });
            let result: Value = api.put(&format!("/endpoints/{id}"), &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Endpoint {id} updated.");
                }
            }
            Ok(())
        }

        EndpointCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Delete endpoint {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/endpoints/{id}")).await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Endpoint deleted."),
            }
            Ok(())
        }
    }
}
