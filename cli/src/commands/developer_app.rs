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

            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;

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
            let mut api = ApiClient::from_auth(&auth)?;
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
                let mut api = ApiClient::from_auth(&auth)?;
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

            let mut api = ApiClient::from_auth(&auth)?;
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
