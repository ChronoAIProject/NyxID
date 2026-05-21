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
                            let short_id = crate::commands::short_id(id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_sessions_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sessions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "sess-12345678", "user_agent": "cli", "ip_address": "1.2.3.4",
                 "created_at": "2026-01-01", "expires_at": "2026-02-01"}
            ])))
            .expect(1)
            .mount(&server)
            .await;

        run(SessionCommands::List {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("session list json should succeed");
    }

    #[tokio::test]
    async fn list_sessions_table_renders_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sessions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "sess-abcdef99", "user_agent": "browser", "ip_address": "5.6.7.8",
                 "created_at": "2026-01-01", "expires_at": "2026-02-01"}
            ])))
            .mount(&server)
            .await;

        run(SessionCommands::List {
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("session list table should succeed");
    }
}
