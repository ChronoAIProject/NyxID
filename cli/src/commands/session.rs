use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, SessionCommands};

pub async fn run(command: SessionCommands) -> Result<()> {
    match command {
        SessionCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let sessions: Value = api.get("/sessions").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&sessions)?);
                }
                OutputFormat::Table => {
                    let items = sessions.as_array();
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No active sessions.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["ID", "Client", "IP", "Created", "Expires"]);

                        for session in items {
                            let id = session["id"]
                                .as_str()
                                .or(session["_id"].as_str())
                                .unwrap_or("-");
                            let short_id = if id.len() > 8 { &id[..8] } else { id };
                            let client = session["user_agent"].as_str().unwrap_or("-");
                            let ip = session["ip_address"].as_str().unwrap_or("-");
                            let created = session["created_at"].as_str().unwrap_or("-");
                            let expires = session["expires_at"].as_str().unwrap_or("-");
                            table.add_row([short_id, client, ip, created, expires]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }
    }
}
