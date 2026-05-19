use std::io::Write;

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ExternalKeyCommands, OutputFormat};

pub async fn run(command: ExternalKeyCommands) -> Result<()> {
    match command {
        ExternalKeyCommands::List { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let keys: Value = api.get("/api-keys/external").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&keys)?);
                }
                OutputFormat::Table => {
                    let items = keys
                        .get("api_keys")
                        .and_then(|v| v.as_array())
                        .or_else(|| keys.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No external API keys.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["ID", "Label", "Credential Type", "Created"]);

                        for key in items {
                            let id = key["id"].as_str().or(key["_id"].as_str()).unwrap_or("-");
                            let short_id = if id.len() > 8 { &id[..8] } else { id };
                            let label = key["label"]
                                .as_str()
                                .or(key["name"].as_str())
                                .unwrap_or("-");
                            let cred_type = key["credential_type"].as_str().unwrap_or("-");
                            let created = key["created_at"].as_str().unwrap_or("-");
                            table.add_row([short_id, label, cred_type, created]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ExternalKeyCommands::Rotate {
            id,
            credential_env,
            credential,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            let credential = if let Some(c) = credential {
                c
            } else if let Some(env_var) = &credential_env {
                std::env::var(env_var)
                    .with_context(|| format!("Environment variable {env_var} not set"))?
            } else {
                rpassword::prompt_password("New credential: ")
                    .map_err(|e| anyhow::anyhow!("{e}"))?
            };
            if credential.is_empty() {
                bail!("Credential is required");
            }

            let body = serde_json::json!({ "credential": credential });
            let result: Value = api.put(&format!("/api-keys/external/{id}"), &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("External credential rotated for {id}.");
                }
            }
            Ok(())
        }

        ExternalKeyCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Delete external key {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty(&format!("/api-keys/external/{id}"))
                .await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("External key deleted."),
            }
            Ok(())
        }
    }
}
