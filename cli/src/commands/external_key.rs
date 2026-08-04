use std::io::Write;

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ExternalKeyCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

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

        ExternalKeyCommands::AddGcpServiceAccount {
            key_file,
            label,
            scopes,
            services,
            org,
            auth,
        } => {
            let key_json = std::fs::read_to_string(&key_file)
                .with_context(|| format!("Failed to read {}", key_file.display()))?;

            // Local sanity check before sending: confirm it's a service
            // account key file, not (say) an OAuth client secret.
            let parsed: Value = serde_json::from_str(&key_json)
                .context("Service account file is not valid JSON")?;
            let is_sa = parsed.get("type").and_then(|v| v.as_str()) == Some("service_account")
                || parsed
                    .get("private_key")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty());
            if !is_sa {
                bail!(
                    "{} does not look like a Google service account key (no private_key). \
                     Create one in the GCP console under IAM & Admin → Service Accounts → Keys.",
                    key_file.display()
                );
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let target_org_id = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let mut body = serde_json::json!({
                "label": label,
                "key_json": key_json,
                "scopes": scopes,
                "service_slugs": services,
            });
            if let Some(org_id) = target_org_id {
                body["target_org_id"] = Value::String(org_id);
            }
            let result: Value = api
                .post("/api-keys/external/gcp-service-account", &body)
                .await?;

            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => {
                    let id = result["id"].as_str().unwrap_or("-");
                    let lbl = result["label"].as_str().unwrap_or("-");
                    eprintln!("Created GCP service account credential {id} ({lbl}).");
                    if !services.is_empty() {
                        eprintln!("Bound to service(s): {}", services.join(", "));
                    }
                    eprintln!(
                        "Tokens now mint automatically from the service account — no more re-auth."
                    );
                }
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

    const ORG_UUID: &str = "11111111-1111-4111-8111-111111111111";
    const GCP_SA_JSON: &str = r#"{"type":"service_account","private_key":"fake-private-key"}"#;

    fn write_temp_gcp_sa_key(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("nyxid-{name}-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&path, GCP_SA_JSON).expect("write temp service-account key");
        path
    }

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
    async fn list_table_renders_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/external"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_keys": [
                    {"id": "key-abc12345", "label": "OpenAI", "credential_type": "bearer", "created_at": "2026-01-01"}
                ]
            })))
            .mount(&server)
            .await;

        run(ExternalKeyCommands::List {
            auth: crate::test_support::mock_auth_with_output(
                server.uri(),
                crate::cli::OutputFormat::Table,
            ),
        })
        .await
        .expect("list table should succeed");
    }

    #[tokio::test]
    async fn list_table_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/external"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "api_keys": [] })),
            )
            .mount(&server)
            .await;

        run(ExternalKeyCommands::List {
            auth: crate::test_support::mock_auth_with_output(
                server.uri(),
                crate::cli::OutputFormat::Table,
            ),
        })
        .await
        .expect("empty list should succeed");
    }

    #[tokio::test]
    async fn rotate_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/api-keys/external/key-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "key-1" })),
            )
            .mount(&server)
            .await;

        run(ExternalKeyCommands::Rotate {
            id: "key-1".to_string(),
            credential_env: None,
            credential: Some("new-cred".to_string()),
            auth: crate::test_support::mock_auth_with_output(
                server.uri(),
                crate::cli::OutputFormat::Table,
            ),
        })
        .await
        .expect("rotate table should succeed");
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

    #[tokio::test]
    async fn add_gcp_service_account_posts_target_org_id() {
        let server = MockServer::start().await;
        let key_file = write_temp_gcp_sa_key("gcp-sa-org");
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys/external/gcp-service-account"))
            .and(body_json(serde_json::json!({
                "label": "Org GCP Reader",
                "key_json": GCP_SA_JSON,
                "scopes": "https://www.googleapis.com/auth/bigquery.readonly",
                "service_slugs": ["google-bigquery", "google-cloud-billing"],
                "target_org_id": ORG_UUID
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "key-org-1",
                "label": "Org GCP Reader"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ExternalKeyCommands::AddGcpServiceAccount {
            key_file: key_file.clone(),
            label: Some("Org GCP Reader".to_string()),
            scopes: Some("https://www.googleapis.com/auth/bigquery.readonly".to_string()),
            services: vec![
                "google-bigquery".to_string(),
                "google-cloud-billing".to_string(),
            ],
            org: Some(ORG_UUID.to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("org GCP SA create should succeed");

        let _ = std::fs::remove_file(key_file);
    }

    #[tokio::test]
    async fn add_gcp_service_account_without_org_omits_target_org_id() {
        let server = MockServer::start().await;
        let key_file = write_temp_gcp_sa_key("gcp-sa-personal");
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys/external/gcp-service-account"))
            .and(body_json(serde_json::json!({
                "label": null,
                "key_json": GCP_SA_JSON,
                "scopes": null,
                "service_slugs": ["google-bigquery"]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "key-personal-1",
                "label": "svc@test-project.iam.gserviceaccount.com"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ExternalKeyCommands::AddGcpServiceAccount {
            key_file: key_file.clone(),
            label: None,
            scopes: None,
            services: vec!["google-bigquery".to_string()],
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("personal GCP SA create should succeed");

        let _ = std::fs::remove_file(key_file);
    }
}
