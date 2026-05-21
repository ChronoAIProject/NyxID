//! CLI commands for managing developer OAuth apps (SUP-030).
//!
//! Developer apps are OIDC clients the caller registers with NyxID so a
//! downstream product can run Sign in with NyxID. Backed by
//! `/developer/oauth-clients`; supports `target_org_id` on create and
//! `?org_id=` on list so org admins can manage org-owned OAuth clients
//! without needing global admin. See `docs/ORG_MODEL.md`.

use std::io::Write;

use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::{Map, Value};

use crate::api::ApiClient;
use crate::cli::{DeveloperAppCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: DeveloperAppCommands) -> Result<()> {
    match command {
        DeveloperAppCommands::Create {
            name,
            redirect_uris,
            client_type,
            allowed_scopes,
            delegation_scopes,
            broker_capability,
            org,
            terminal,
            no_wait,
            auth,
        } => {
            if redirect_uris.is_empty() {
                bail!("At least one --redirect-uri is required");
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

            // Browser-flow gate: confidential clients mint a
            // `client_secret` (the leak surface). Public clients
            // never produce a secret, so we skip the wizard for
            // them and let the existing terminal output stand —
            // there is nothing to leak. Defaults to "public" when
            // unspecified, mirroring the backend default.
            let resolved_client_type = client_type.clone().unwrap_or_else(|| "public".to_string());
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = resolved_client_type == "confidential"
                && !terminal
                && (no_wait || (interactive_output && crate::wizard::is_browser_flow_eligible()));

            if wizard_eligible {
                let prefill = crate::wizard::DeveloperAppCreatePrefill {
                    name: Some(name),
                    redirect_uris: redirect_uris.clone(),
                    allowed_scopes,
                    delegation_scopes,
                    broker_capability,
                    org_id: org,
                };
                return crate::wizard::run_developer_app_create_wizard(&auth, prefill, no_wait)
                    .await;
            }

            let mut body = Map::new();
            body.insert("name".to_string(), Value::String(name));
            body.insert(
                "redirect_uris".to_string(),
                Value::Array(redirect_uris.into_iter().map(Value::String).collect()),
            );
            if let Some(ct) = client_type {
                body.insert("client_type".to_string(), Value::String(ct));
            }
            if let Some(scopes) = allowed_scopes {
                let items: Vec<Value> = scopes
                    .split_whitespace()
                    .map(|s| Value::String(s.to_string()))
                    .collect();
                body.insert("allowed_scopes".to_string(), Value::Array(items));
            }
            if let Some(ds) = delegation_scopes {
                body.insert("delegation_scopes".to_string(), Value::String(ds));
            }
            if let Some(enabled) = broker_capability {
                body.insert(
                    "broker_capability_enabled".to_string(),
                    Value::Bool(enabled),
                );
            }
            if let Some(org_id) = org {
                body.insert("target_org_id".to_string(), Value::String(org_id));
            }

            let result: Value = api
                .post("/developer/oauth-clients", &Value::Object(body))
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let id = result["id"].as_str().unwrap_or("-");
                    let name = result["client_name"].as_str().unwrap_or("-");
                    let ctype = result["client_type"].as_str().unwrap_or("-");
                    let scopes = result["allowed_scopes"].as_str().unwrap_or("-");
                    let dscopes = result["delegation_scopes"].as_str().unwrap_or("-");
                    let broker_capability = if result["broker_capability_enabled"]
                        .as_bool()
                        .unwrap_or(false)
                    {
                        "yes"
                    } else {
                        "no"
                    };
                    let secret = result["client_secret"].as_str();

                    eprintln!("Developer app created!");
                    eprintln!();
                    eprintln!("ID:                {id}");
                    eprintln!("Name:              {name}");
                    eprintln!("Type:              {ctype}");
                    eprintln!("Allowed scopes:    {scopes}");
                    eprintln!("Delegation scopes: {dscopes}");
                    eprintln!("Broker capability: {broker_capability}");
                    if let Some(s) = secret {
                        eprintln!(
                            "Client secret:     {s}  (save this -- confidential clients only, shown once)"
                        );
                    }
                }
            }
            Ok(())
        }

        DeveloperAppCommands::List { org, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let path = match org {
                Some(ref id) => {
                    format!(
                        "/developer/oauth-clients?org_id={}",
                        urlencoding::encode(id)
                    )
                }
                None => "/developer/oauth-clients".to_string(),
            };
            let result: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let items = result["clients"].as_array();
                    match items {
                        Some(items) if !items.is_empty() => {
                            let mut table = Table::new();
                            table.load_preset(UTF8_FULL_CONDENSED);
                            table.set_header([
                                "ID",
                                "Name",
                                "Type",
                                "Redirect URIs",
                                "Scopes",
                                "Active",
                                "Created",
                            ]);
                            for c in items {
                                // Show the full UUID — follow-up subcommands
                                // (show/update/delete/rotate-secret) all take
                                // the exact client id in the path.
                                let id = c["id"].as_str().unwrap_or("-");
                                let name = c["client_name"].as_str().unwrap_or("-");
                                let ctype = c["client_type"].as_str().unwrap_or("-");
                                let uris = c["redirect_uris"]
                                    .as_array()
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_else(|| "-".to_string());
                                let scopes = c["allowed_scopes"].as_str().unwrap_or("-");
                                let active = if c["is_active"].as_bool().unwrap_or(false) {
                                    "yes"
                                } else {
                                    "no"
                                };
                                let created = c["created_at"].as_str().unwrap_or("-");
                                table.add_row([id, name, ctype, &uris, scopes, active, created]);
                            }
                            eprintln!("{table}");
                        }
                        _ => eprintln!("No developer apps."),
                    }
                }
            }
            Ok(())
        }

        DeveloperAppCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.get(&format!("/developer/oauth-clients/{id}")).await?;
            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    print_app_detail(&result);
                }
            }
            Ok(())
        }

        DeveloperAppCommands::Update {
            id,
            name,
            redirect_uris,
            allowed_scopes,
            delegation_scopes,
            broker_capability,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;

            let mut body = Map::new();
            if let Some(n) = name {
                body.insert("name".to_string(), Value::String(n));
            }
            if !redirect_uris.is_empty() {
                body.insert(
                    "redirect_uris".to_string(),
                    Value::Array(redirect_uris.into_iter().map(Value::String).collect()),
                );
            }
            if let Some(scopes) = allowed_scopes {
                let items: Vec<Value> = scopes
                    .split_whitespace()
                    .map(|s| Value::String(s.to_string()))
                    .collect();
                body.insert("allowed_scopes".to_string(), Value::Array(items));
            }
            if let Some(ds) = delegation_scopes {
                body.insert("delegation_scopes".to_string(), Value::String(ds));
            }
            if let Some(enabled) = broker_capability {
                body.insert(
                    "broker_capability_enabled".to_string(),
                    Value::Bool(enabled),
                );
            }

            let result: Value = api
                .patch(
                    &format!("/developer/oauth-clients/{id}"),
                    &Value::Object(body),
                )
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Developer app {id} updated.");
                }
            }
            Ok(())
        }

        DeveloperAppCommands::Delete { id, yes, auth } => {
            if !yes && !confirm(&format!("Delete developer app {id}?"))? {
                return Ok(());
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/developer/oauth-clients/{id}"))
                .await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ok": true,
                        "id": id,
                        "resource_type": "developer_app",
                        "status": "deactivated",
                    }))?
                ),
                OutputFormat::Table => eprintln!("Developer app deactivated."),
            }
            Ok(())
        }

        DeveloperAppCommands::RotateSecret {
            id,
            terminal,
            no_wait,
            auth,
        } => {
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && (no_wait || (interactive_output && crate::wizard::is_browser_flow_eligible()));

            if wizard_eligible {
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                // Best-effort fetch of the display name (client_name)
                // for the confirm panel. Fallback to id if the fetch
                // fails — non-fatal.
                let display_name = match api
                    .get::<Value>(&format!("/developer/oauth-clients/{id}"))
                    .await
                {
                    Ok(c) => c["client_name"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| id.clone()),
                    Err(_) => id.clone(),
                };
                let prefill = crate::wizard::RotatePrefill {
                    resource_id: id,
                    display_name,
                };
                return crate::wizard::run_developer_app_rotate_secret_wizard(
                    &auth, prefill, no_wait,
                )
                .await;
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .post(
                    &format!("/developer/oauth-clients/{id}/rotate-secret"),
                    &Value::Null,
                )
                .await?;
            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let cid = result["id"].as_str().unwrap_or("-");
                    let secret = result["client_secret"].as_str().unwrap_or("-");
                    eprintln!("Secret rotated for {cid}.");
                    eprintln!();
                    eprintln!("Client secret: {secret}  (save this -- shown only once)");
                }
            }
            Ok(())
        }
    }
}

fn print_app_detail(c: &Value) {
    let id = c["id"].as_str().unwrap_or("-");
    let name = c["client_name"].as_str().unwrap_or("-");
    let ctype = c["client_type"].as_str().unwrap_or("-");
    let scopes = c["allowed_scopes"].as_str().unwrap_or("-");
    let dscopes = c["delegation_scopes"].as_str().unwrap_or("-");
    let broker_capability = if c["broker_capability_enabled"].as_bool().unwrap_or(false) {
        "yes"
    } else {
        "no"
    };
    let active = if c["is_active"].as_bool().unwrap_or(false) {
        "yes"
    } else {
        "no"
    };
    let created = c["created_at"].as_str().unwrap_or("-");
    let uris = c["redirect_uris"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("\n                   ")
        })
        .unwrap_or_else(|| "-".to_string());

    eprintln!("ID:                {id}");
    eprintln!("Name:              {name}");
    eprintln!("Type:              {ctype}");
    eprintln!("Active:            {active}");
    eprintln!("Allowed scopes:    {scopes}");
    eprintln!("Delegation scopes: {dscopes}");
    eprintln!("Broker capability: {broker_capability}");
    eprintln!("Redirect URIs:     {uris}");
    eprintln!("Created:           {created}");
}

fn confirm(prompt: &str) -> Result<bool> {
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        eprintln!("Cancelled.");
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // A syntactically valid org UUID. `resolve_org_id` short-circuits on
    // UUID-shaped input and returns it verbatim with no `/orgs` roundtrip,
    // so the create/list bodies should carry exactly this string.
    const ORG_UUID: &str = "11111111-1111-1111-1111-111111111111";

    // --- Create ---

    #[tokio::test]
    async fn create_posts_full_body_with_split_scopes_and_org() {
        let server = MockServer::start().await;
        // body_json is an EXACT match: this asserts the complete contract
        // — client_type passthrough, allowed_scopes split on whitespace
        // into an array, delegation_scopes kept as a raw string, broker
        // capability remapped to broker_capability_enabled, and the
        // resolved org UUID surfaced as target_org_id.
        Mock::given(method("POST"))
            .and(path("/api/v1/developer/oauth-clients"))
            .and(body_json(serde_json::json!({
                "name": "Acme Login",
                "redirect_uris": ["https://acme.test/cb", "https://acme.test/cb2"],
                "client_type": "confidential",
                "allowed_scopes": ["openid", "profile", "email"],
                "delegation_scopes": "https://api.test/read",
                "broker_capability_enabled": true,
                "target_org_id": ORG_UUID
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "client-1",
                "client_name": "Acme Login",
                "client_type": "confidential",
                "client_secret": "sek_shown_once"
            })))
            .expect(1)
            .mount(&server)
            .await;

        // terminal: true forces the scripted POST path; the confidential
        // client would otherwise be wizard-eligible.
        run(DeveloperAppCommands::Create {
            name: "Acme Login".to_string(),
            redirect_uris: vec![
                "https://acme.test/cb".to_string(),
                "https://acme.test/cb2".to_string(),
            ],
            client_type: Some("confidential".to_string()),
            allowed_scopes: Some("openid profile email".to_string()),
            delegation_scopes: Some("https://api.test/read".to_string()),
            broker_capability: Some(true),
            org: Some(ORG_UUID.to_string()),
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn create_minimal_body_omits_optional_fields() {
        let server = MockServer::start().await;
        // With no client_type/scopes/delegation/broker/org provided, the
        // body must contain ONLY name + redirect_uris — none of the
        // optional keys may leak through as null/empty.
        Mock::given(method("POST"))
            .and(path("/api/v1/developer/oauth-clients"))
            .and(body_json(serde_json::json!({
                "name": "Minimal",
                "redirect_uris": ["https://m.test/cb"]
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "client-2" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::Create {
            name: "Minimal".to_string(),
            redirect_uris: vec!["https://m.test/cb".to_string()],
            client_type: None,
            allowed_scopes: None,
            delegation_scopes: None,
            broker_capability: None,
            org: None,
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("minimal create should succeed");
    }

    #[tokio::test]
    async fn create_rejects_empty_redirect_uris_without_calling_api() {
        // Validation guard fires before any ApiClient construction or
        // network call — point at an unroutable base URL to prove no
        // request is made (a request would surface a connection error,
        // but the bail! must win first).
        let result = run(DeveloperAppCommands::Create {
            name: "NoRedirect".to_string(),
            redirect_uris: vec![],
            client_type: None,
            allowed_scopes: None,
            delegation_scopes: None,
            broker_capability: None,
            org: None,
            terminal: true,
            no_wait: false,
            auth: mock_auth("http://127.0.0.1:0"),
        })
        .await;

        let err = result.expect_err("empty redirect_uris must be rejected");
        assert!(
            err.to_string().contains("redirect-uri"),
            "expected redirect-uri validation message, got: {err}"
        );
    }

    // --- List ---

    #[tokio::test]
    async fn list_without_org_hits_base_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/developer/oauth-clients"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "clients": [{
                    "id": "client-1",
                    "client_name": "Acme",
                    "client_type": "public"
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::List {
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn list_with_org_appends_org_id_query_param() {
        let server = MockServer::start().await;
        // Org scoping is expressed as a `?org_id=` query param, not a body
        // — assert the resolved UUID lands there.
        Mock::given(method("GET"))
            .and(path("/api/v1/developer/oauth-clients"))
            .and(query_param("org_id", ORG_UUID))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "clients": [] })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::List {
            org: Some(ORG_UUID.to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("org-scoped list should succeed");
    }

    // --- Show ---

    #[tokio::test]
    async fn show_fetches_client_by_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/developer/oauth-clients/client-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "client-1",
                "client_name": "Acme"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::Show {
            id: "client-1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("show should succeed");
    }

    // --- Update ---

    #[tokio::test]
    async fn update_sends_only_provided_fields() {
        let server = MockServer::start().await;
        // Partial PATCH: only name + delegation_scopes were provided, so
        // the body must omit redirect_uris/allowed_scopes/broker entirely.
        Mock::given(method("PATCH"))
            .and(path("/api/v1/developer/oauth-clients/client-1"))
            .and(body_json(serde_json::json!({
                "name": "Renamed",
                "delegation_scopes": ""
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::Update {
            id: "client-1".to_string(),
            name: Some("Renamed".to_string()),
            redirect_uris: vec![],
            allowed_scopes: None,
            // Empty delegation_scopes is meaningful (disables token
            // exchange) and must be forwarded, not dropped.
            delegation_scopes: Some("".to_string()),
            broker_capability: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn update_replaces_redirect_uris_and_splits_scopes() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/developer/oauth-clients/client-1"))
            .and(body_json(serde_json::json!({
                "redirect_uris": ["https://x.test/cb"],
                "allowed_scopes": ["openid", "email"],
                "broker_capability_enabled": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::Update {
            id: "client-1".to_string(),
            name: None,
            redirect_uris: vec!["https://x.test/cb".to_string()],
            allowed_scopes: Some("openid email".to_string()),
            delegation_scopes: None,
            broker_capability: Some(false),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    // --- Delete ---

    #[tokio::test]
    async fn delete_with_yes_issues_delete_request() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/developer/oauth-clients/client-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::Delete {
            id: "client-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("delete should succeed");
    }

    #[tokio::test]
    async fn delete_surfaces_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/developer/oauth-clients/client-1"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let result = run(DeveloperAppCommands::Delete {
            id: "client-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "non-2xx delete should surface as an error");
    }

    // --- RotateSecret ---

    #[tokio::test]
    async fn rotate_secret_terminal_posts_null_body_to_rotate_endpoint() {
        let server = MockServer::start().await;
        // terminal: true bypasses the wizard and posts a `null` JSON body
        // (api.post(&Value::Null)) to the rotate-secret endpoint.
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/developer/oauth-clients/client-1/rotate-secret",
            ))
            .and(body_json(serde_json::Value::Null))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "client-1",
                "client_secret": "sek_rotated_once"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(DeveloperAppCommands::RotateSecret {
            id: "client-1".to_string(),
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("rotate-secret should succeed");
    }

    #[tokio::test]
    async fn rotate_secret_surfaces_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/developer/oauth-clients/client-1/rotate-secret",
            ))
            .respond_with(ResponseTemplate::new(409).set_body_string("not confidential"))
            .mount(&server)
            .await;

        let result = run(DeveloperAppCommands::RotateSecret {
            id: "client-1".to_string(),
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(
            result.is_err(),
            "rotate-secret on a public/invalid client should surface the error"
        );
    }
}
