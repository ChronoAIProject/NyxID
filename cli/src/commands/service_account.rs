//! CLI commands for managing service accounts (SUP-030).
//!
//! Service accounts are machine identities that authenticate with the
//! `grant_type=client_credentials` flow. The backend exposes them under
//! `/admin/service-accounts`; the org-scoped paths accept `target_org_id`
//! on create and `org_id` on list, so org admins can manage SAs owned by
//! their org without needing global admin. See `docs/ORG_MODEL.md`.

use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::{Map, Value};

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ServiceAccountCommands};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: ServiceAccountCommands) -> Result<()> {
    match command {
        ServiceAccountCommands::Create {
            name,
            scopes,
            description,
            rate_limit_override,
            role_ids,
            org,
            terminal,
            no_wait,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

            // Browser-flow gate — see `api_key.rs::Create` for the
            // full predicate rationale. `--terminal` / `--no-wizard`
            // / piped output / `NYXID_NO_WIZARD=1` fall through to
            // the byte-identical scripted path below; `--no-wait`
            // forces the resumable pairing variant.
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && (no_wait || (interactive_output && crate::wizard::is_browser_flow_eligible()));

            if wizard_eligible {
                let prefill = crate::wizard::ServiceAccountCreatePrefill {
                    name: Some(name),
                    scopes: Some(scopes),
                    description,
                    rate_limit_override,
                    role_ids_csv: role_ids,
                    org_id: org,
                };
                return crate::wizard::run_service_account_create_wizard(&auth, prefill, no_wait)
                    .await;
            }

            let mut body = Map::new();
            body.insert("name".to_string(), Value::String(name));
            body.insert("allowed_scopes".to_string(), Value::String(scopes));
            if let Some(desc) = description {
                body.insert("description".to_string(), Value::String(desc));
            }
            if let Some(limit) = rate_limit_override {
                body.insert(
                    "rate_limit_override".to_string(),
                    Value::Number(limit.into()),
                );
            }
            if let Some(roles) = role_ids {
                let ids: Vec<Value> = roles
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::String(s.to_string()))
                    .collect();
                body.insert("role_ids".to_string(), Value::Array(ids));
            }
            if let Some(org_id) = org {
                body.insert("target_org_id".to_string(), Value::String(org_id));
            }

            let result: Value = api
                .post("/admin/service-accounts", &Value::Object(body))
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let id = result["id"].as_str().unwrap_or("-");
                    let name = result["name"].as_str().unwrap_or("-");
                    let client_id = result["client_id"].as_str().unwrap_or("-");
                    let secret = result["client_secret"].as_str().unwrap_or("-");
                    let scopes = result["allowed_scopes"].as_str().unwrap_or("-");

                    eprintln!("Service account created!");
                    eprintln!();
                    eprintln!("ID:            {id}");
                    eprintln!("Name:          {name}");
                    eprintln!("Client ID:     {client_id}");
                    eprintln!("Client secret: {secret}  (save this -- shown only once)");
                    eprintln!("Scopes:        {scopes}");
                }
            }
            Ok(())
        }

        ServiceAccountCommands::List {
            org,
            search,
            page,
            per_page,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let mut qs = vec![
                format!("page={page}"),
                format!("per_page={}", per_page.min(100)),
            ];
            if let Some(ref q) = search {
                qs.push(format!("search={}", urlencoding::encode(q)));
            }
            if let Some(ref id) = org {
                qs.push(format!("org_id={}", urlencoding::encode(id)));
            }
            let path = format!("/admin/service-accounts?{}", qs.join("&"));
            let result: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let items = result["service_accounts"].as_array();
                    match items {
                        Some(items) if !items.is_empty() => {
                            let mut table = Table::new();
                            table.load_preset(UTF8_FULL_CONDENSED);
                            table.set_header([
                                "ID",
                                "Name",
                                "Client ID",
                                "Scopes",
                                "Active",
                                "Created",
                            ]);
                            for sa in items {
                                // Show the full UUID — follow-up subcommands
                                // (show/update/delete/rotate-secret/revoke-tokens)
                                // take the exact id and the backend does not
                                // resolve prefixes or client_id. Truncating
                                // would force users to re-run with --output
                                // json to get a usable identifier.
                                let id = sa["id"].as_str().unwrap_or("-");
                                let name = sa["name"].as_str().unwrap_or("-");
                                let client_id = sa["client_id"].as_str().unwrap_or("-");
                                let scopes = sa["allowed_scopes"].as_str().unwrap_or("-");
                                let active = if sa["is_active"].as_bool().unwrap_or(false) {
                                    "yes"
                                } else {
                                    "no"
                                };
                                let created = sa["created_at"].as_str().unwrap_or("-");
                                table.add_row([id, name, client_id, scopes, active, created]);
                            }
                            eprintln!("{table}");
                        }
                        _ => eprintln!("No service accounts."),
                    }
                }
            }
            Ok(())
        }

        ServiceAccountCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let result: Value = api.get(&format!("/admin/service-accounts/{id}")).await?;
            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    print_sa_detail(&result);
                }
            }
            Ok(())
        }

        ServiceAccountCommands::Update {
            id,
            name,
            description,
            scopes,
            role_ids,
            is_active,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            let mut body = Map::new();
            if let Some(n) = name {
                body.insert("name".to_string(), Value::String(n));
            }
            if let Some(d) = description {
                body.insert("description".to_string(), Value::String(d));
            }
            if let Some(s) = scopes {
                body.insert("allowed_scopes".to_string(), Value::String(s));
            }
            if let Some(roles) = role_ids {
                let ids: Vec<Value> = roles
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::String(s.to_string()))
                    .collect();
                body.insert("role_ids".to_string(), Value::Array(ids));
            }
            if let Some(active) = is_active {
                body.insert("is_active".to_string(), Value::Bool(active));
            }

            let result: Value = api
                .put(
                    &format!("/admin/service-accounts/{id}"),
                    &Value::Object(body),
                )
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Service account {id} updated.");
                }
            }
            Ok(())
        }

        ServiceAccountCommands::Delete { id, yes, auth } => {
            if !yes && !confirm(&format!("Delete service account {id}?"))? {
                return Ok(());
            }
            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty(&format!("/admin/service-accounts/{id}"))
                .await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ok": true,
                        "id": id,
                        "resource_type": "service_account",
                        "status": "deactivated",
                    }))?
                ),
                OutputFormat::Table => eprintln!("Service account deactivated."),
            }
            Ok(())
        }

        ServiceAccountCommands::RotateSecret {
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
                // Best-effort fetch of the display name for the
                // confirm panel. Fallback to id if the fetch fails —
                // non-fatal; the confirm panel just shows the raw id.
                let display_name = match api
                    .get::<Value>(&format!("/admin/service-accounts/{id}"))
                    .await
                {
                    Ok(sa) => sa["name"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| id.clone()),
                    Err(_) => id.clone(),
                };
                let prefill = crate::wizard::RotatePrefill {
                    resource_id: id,
                    display_name,
                };
                return crate::wizard::run_service_account_rotate_secret_wizard(
                    &auth, prefill, no_wait,
                )
                .await;
            }

            let mut api = ApiClient::from_auth(&auth)?;
            let result: Value = api
                .post(
                    &format!("/admin/service-accounts/{id}/rotate-secret"),
                    &Value::Null,
                )
                .await?;
            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let client_id = result["client_id"].as_str().unwrap_or("-");
                    let secret = result["client_secret"].as_str().unwrap_or("-");
                    eprintln!("Secret rotated. All existing tokens have been revoked.");
                    eprintln!();
                    eprintln!("Client ID:     {client_id}");
                    eprintln!("Client secret: {secret}  (save this -- shown only once)");
                }
            }
            Ok(())
        }

        ServiceAccountCommands::RevokeTokens { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let result: Value = api
                .post(
                    &format!("/admin/service-accounts/{id}/revoke-tokens"),
                    &Value::Null,
                )
                .await?;
            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let count = result["revoked_count"].as_u64().unwrap_or(0);
                    eprintln!("Revoked {count} active tokens.");
                }
            }
            Ok(())
        }
    }
}

fn print_sa_detail(sa: &Value) {
    let id = sa["id"].as_str().unwrap_or("-");
    let name = sa["name"].as_str().unwrap_or("-");
    let desc = sa["description"].as_str().unwrap_or("-");
    let client_id = sa["client_id"].as_str().unwrap_or("-");
    let prefix = sa["secret_prefix"].as_str().unwrap_or("-");
    let scopes = sa["allowed_scopes"].as_str().unwrap_or("-");
    let roles = sa["role_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "-".to_string());
    let active = if sa["is_active"].as_bool().unwrap_or(false) {
        "yes"
    } else {
        "no"
    };
    let created_by = sa["created_by"].as_str().unwrap_or("-");
    let created = sa["created_at"].as_str().unwrap_or("-");
    let rate_limit = sa["rate_limit_override"]
        .as_u64()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "default".to_string());

    eprintln!("ID:              {id}");
    eprintln!("Name:            {name}");
    eprintln!("Description:     {desc}");
    eprintln!("Client ID:       {client_id}");
    eprintln!("Secret prefix:   {prefix}");
    eprintln!("Scopes:          {scopes}");
    eprintln!("Roles:           {roles}");
    eprintln!("Active:          {active}");
    eprintln!("Rate limit:      {rate_limit}");
    eprintln!("Owner (user_id): {created_by}");
    eprintln!("Created:         {created}");
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
