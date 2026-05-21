use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ProfileCommands};

pub async fn run(command: ProfileCommands) -> Result<()> {
    match command {
        ProfileCommands::Update { name, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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

            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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

            let mut api = ApiClient::from_auth_checked(&auth).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // `Update` maps the optional `--name` flag onto a `display_name` field
    // in the PUT body. Assert the exact request body so the field-name
    // mapping (name -> display_name) is locked in.
    #[tokio::test]
    async fn update_sends_name_as_display_name() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/users/me"))
            .and(body_json(serde_json::json!({ "display_name": "Alice" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "display_name": "Alice" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::Update {
            name: Some("Alice".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("profile update should succeed");
    }

    // With no fields to update the handler must short-circuit and make no
    // HTTP call at all. A mock with `.expect(0)` fails the test (on server
    // drop) if any request reaches it, proving the early return fired.
    #[tokio::test]
    async fn update_with_no_fields_makes_no_request() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        run(ProfileCommands::Update {
            name: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("empty update should be a no-op Ok");
    }

    // `--yes` bypasses the interactive confirmation prompt and issues the
    // account-deletion DELETE directly.
    #[tokio::test]
    async fn delete_with_yes_issues_delete() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/users/me"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::Delete {
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("account delete should succeed");
    }

    #[tokio::test]
    async fn consents_lists_via_get() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me/consents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "consents": [
                    {"client_id": "c-1", "client_name": "App", "scopes": "openid",
                     "granted_at": "2026-01-01"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::Consents {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("consents list should succeed");
    }

    // The client_id argument must be interpolated into the DELETE path.
    // The mock only matches the exact `/consents/client-1` path, so a
    // mismatched/missing interpolation fails the `.expect(1)` assertion.
    #[tokio::test]
    async fn revoke_consent_with_yes_targets_client_id_path() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/users/me/consents/client-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ProfileCommands::RevokeConsent {
            client_id: "client-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("revoke consent should succeed");
    }

    // 5xx from the upstream must surface as an error, not be swallowed.
    #[tokio::test]
    async fn consents_surfaces_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me/consents"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let result = run(ProfileCommands::Consents {
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "5xx should surface as an error");
    }
}
