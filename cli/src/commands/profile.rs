use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ProfileCommands};

pub async fn run(command: ProfileCommands) -> Result<()> {
    match command {
        ProfileCommands::Update { name, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let mut body = serde_json::Map::new();

            if let Some(name) = name {
                body.insert("display_name".into(), Value::String(name));
            }

            if body.is_empty() {
                eprintln!("No updates specified. Use --name to update your display name.");
                return Ok(());
            }

            let result: Value = api.put("/users/me", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Profile updated.");
                    if let Some(name) = result["display_name"].as_str() {
                        eprintln!("Name: {name}");
                    }
                }
            }
            Ok(())
        }

        ProfileCommands::Delete { yes, auth } => {
            if !yes {
                eprint!("Permanently delete your account? This cannot be undone. [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty("/users/me").await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Account deleted."),
            }
            Ok(())
        }

        ProfileCommands::Consents { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let consents: Value = api.get("/users/me/consents").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&consents)?);
                }
                OutputFormat::Table => {
                    let items = consents
                        .get("consents")
                        .and_then(|v| v.as_array())
                        .or_else(|| consents.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No OAuth consents.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["Client ID", "App Name", "Scopes", "Granted"]);

                        for consent in items {
                            let client_id = consent["client_id"].as_str().unwrap_or("-");
                            let app_name = consent["client_name"].as_str().unwrap_or("-");
                            let scopes = consent["scopes"].as_str().unwrap_or("-");
                            let granted = consent["granted_at"].as_str().unwrap_or("-");
                            table.add_row([client_id, app_name, scopes, granted]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ProfileCommands::RevokeConsent {
            client_id,
            yes,
            auth,
        } => {
            if !yes {
                eprint!("Revoke consent for client {client_id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty(&format!("/users/me/consents/{client_id}"))
                .await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Consent revoked for client {client_id}."),
            }
            Ok(())
        }
    }
}
