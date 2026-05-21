use std::io::Write;

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ExternalKeyCommands, OutputFormat};

pub async fn run(command: ExternalKeyCommands) -> Result<()> {
    match command {
        ExternalKeyCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
                            let short_id = crate::commands::short_id(id);
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;

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

            let mut api = ApiClient::from_auth_checked(&auth).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_fetches_external_keys_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/external"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_keys": [
                    {"id": "key-abc12345", "label": "OpenAI", "credential_type": "bearer", "created_at": "2026-01-01"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ExternalKeyCommands::List {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn rotate_sends_credential_in_body() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/api-keys/external/key-1"))
            .and(body_json(serde_json::json!({ "credential": "new-secret" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "key-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ExternalKeyCommands::Rotate {
            id: "key-1".to_string(),
            credential_env: None,
            credential: Some("new-secret".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("rotate should succeed");
    }

    #[tokio::test]
    async fn rotate_rejects_empty_credential() {
        let server = MockServer::start().await;
        // No mock mounted: an empty credential must fail before any request.
        let result = run(ExternalKeyCommands::Rotate {
            id: "key-1".to_string(),
            credential_env: None,
            credential: Some(String::new()),
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "empty credential should be rejected");
    }

    #[tokio::test]
    async fn delete_with_yes_issues_delete_request() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/api-keys/external/key-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ExternalKeyCommands::Delete {
            id: "key-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("delete should succeed");
    }
}
