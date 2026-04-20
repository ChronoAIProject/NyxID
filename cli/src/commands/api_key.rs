use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ApiKeyCommands, AuthArgs, OutputFormat};

pub async fn run(command: ApiKeyCommands) -> Result<()> {
    match command {
        ApiKeyCommands::Create {
            name,
            scopes,
            expires_in_days,
            allowed_services,
            allowed_nodes,
            allow_all_services,
            allow_all_nodes,
            platform,
            callback_url,
            org,
            terminal,
            auth,
        } => {
            use std::io::IsTerminal;
            // Wizard gate — mirrors Rotate. Any of --terminal,
            // --output json, piped stdout, SSH, or NYXID_NO_WIZARD
            // falls through to the scripted path below with
            // byte-identical behavior to pre-wizard.
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && interactive_output
                && std::io::stdout().is_terminal()
                && crate::wizard::is_wizard_eligible();

            if wizard_eligible {
                let prefill = crate::wizard::ApiKeyCreatePrefill {
                    name: name.clone(),
                    platform: platform.clone(),
                    scopes: scopes.clone(),
                    expires_in_days,
                    allow_all_services,
                    allow_all_nodes,
                    allowed_services_csv: allowed_services.clone(),
                    allowed_nodes_csv: allowed_nodes.clone(),
                    callback_url: callback_url.clone(),
                    org_id: org.clone(),
                };
                return crate::wizard::run_api_key_create_wizard(&auth, prefill).await;
            }

            let mut api = ApiClient::from_auth(&auth)?;

            let key_name = match name {
                Some(n) => n,
                None => {
                    eprint!("Key name: ");
                    std::io::stderr().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    let trimmed = input.trim().to_string();
                    if trimmed.is_empty() {
                        "CLI Key".to_string()
                    } else {
                        trimmed
                    }
                }
            };

            let scope_str = scopes.as_deref().unwrap_or("read write");

            let mut body = serde_json::json!({
                "name": key_name,
                "scopes": scope_str,
            });

            if let Some(days) = expires_in_days
                && days > 0
            {
                let expires = chrono::Utc::now() + chrono::Duration::days(i64::from(days));
                body["expires_at"] = Value::String(expires.to_rfc3339());
            }

            if let Some(services) = allowed_services {
                let ids: Vec<&str> = services.split(',').map(|s| s.trim()).collect();
                body["allowed_service_ids"] = serde_json::json!(ids);
            }
            if let Some(nodes) = allowed_nodes {
                let ids: Vec<&str> = nodes.split(',').map(|s| s.trim()).collect();
                body["allowed_node_ids"] = serde_json::json!(ids);
            }
            if allow_all_services {
                body["allow_all_services"] = Value::Bool(true);
            }
            if allow_all_nodes {
                body["allow_all_nodes"] = Value::Bool(true);
            }
            if let Some(ref platform) = platform {
                body["platform"] = Value::String(platform.clone());
            }
            if let Some(ref url) = callback_url {
                body["callback_url"] = Value::String(url.clone());
            }
            if let Some(ref org_id) = org {
                body["target_org_id"] = Value::String(org_id.clone());
            }

            let result: Value = api.post("/api-keys", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let name = result["name"].as_str().unwrap_or("-");
                    let key = result["full_key"].as_str().unwrap_or("-");
                    let scopes = result["scopes"].as_str().unwrap_or("-");
                    let expires = result["expires_at"].as_str().unwrap_or("never");

                    eprintln!("API key created!");
                    eprintln!();
                    eprintln!("Name:    {name}");
                    eprintln!("Key:     {key}  (save this -- shown only once)");
                    eprintln!("Scopes:  {scopes}");
                    eprintln!("Expires: {expires}");
                    eprintln!();
                    eprintln!("Set as environment variable:");
                    eprintln!("  export NYXID_API_KEY=\"{key}\"");
                }
            }
            Ok(())
        }

        ApiKeyCommands::List { org, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let path = match org {
                Some(ref id) => format!("/api-keys?org_id={}", urlencoding::encode(id)),
                None => "/api-keys".to_string(),
            };
            let keys: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&keys)?);
                }
                OutputFormat::Table => {
                    let items = keys
                        .get("keys")
                        .and_then(|v| v.as_array())
                        .or_else(|| keys.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No API keys.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "ID",
                            "Name",
                            "Scopes",
                            "Services",
                            "Nodes",
                            "Last Used",
                        ]);

                        for key in items {
                            let id = key["id"].as_str().or(key["_id"].as_str()).unwrap_or("-");
                            let name = key["name"].as_str().unwrap_or("-");
                            let scopes = key["scopes"].as_str().unwrap_or("-");
                            let services = if key["allow_all_services"].as_bool().unwrap_or(true) {
                                "all".to_string()
                            } else {
                                key["allowed_services"]
                                    .as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|s| {
                                                s["slug"].as_str().or(s["label"].as_str())
                                            })
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_else(|| "-".to_string())
                            };
                            let nodes = if key["allow_all_nodes"].as_bool().unwrap_or(true) {
                                "all".to_string()
                            } else {
                                key["allowed_nodes"]
                                    .as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|n| n["name"].as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_else(|| "-".to_string())
                            };
                            let last_used = key["last_used_at"].as_str().unwrap_or("never");
                            table.add_row([id, name, scopes, &services, &nodes, last_used]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ApiKeyCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let key: Value = api.get(&format!("/api-keys/{id}")).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&key)?);
                }
                OutputFormat::Table => {
                    let name = key["name"].as_str().unwrap_or("-");
                    let key_id = key["id"].as_str().or(key["_id"].as_str()).unwrap_or(&id);
                    let scopes = key["scopes"].as_str().unwrap_or("-");
                    let expires = key["expires_at"].as_str().unwrap_or("never");
                    let last_used = key["last_used_at"].as_str().unwrap_or("never");
                    let prefix = key["key_prefix"].as_str().unwrap_or("-");

                    eprintln!("Name:       {name}");
                    eprintln!("ID:         {key_id}");
                    eprintln!("Prefix:     {prefix}");
                    eprintln!("Scopes:     {scopes}");
                    eprintln!("Expires:    {expires}");
                    eprintln!("Last Used:  {last_used}");

                    if let Some(services) = key["allowed_service_ids"].as_array()
                        && !services.is_empty()
                    {
                        let ids: Vec<&str> = services.iter().filter_map(|v| v.as_str()).collect();
                        eprintln!("Allowed Services: {}", ids.join(", "));
                    }
                    if key["allow_all_services"].as_bool().unwrap_or(false) {
                        eprintln!("Allowed Services: all");
                    }
                    if let Some(nodes) = key["allowed_node_ids"].as_array()
                        && !nodes.is_empty()
                    {
                        let ids: Vec<&str> = nodes.iter().filter_map(|v| v.as_str()).collect();
                        eprintln!("Allowed Nodes:    {}", ids.join(", "));
                    }
                    if key["allow_all_nodes"].as_bool().unwrap_or(false) {
                        eprintln!("Allowed Nodes:    all");
                    }
                }
            }
            Ok(())
        }

        ApiKeyCommands::Rotate { id, terminal, auth } => {
            use std::io::IsTerminal;
            // Wizard mode (v3 DisplayOnce) when output is interactive,
            // stdout is a TTY, and the environment can open a local
            // browser. Mirrors the v2 `service add` gate. Anything else
            // (--terminal, --output json, piped, SSH, NYXID_NO_WIZARD)
            // falls through to the scripted path BELOW, byte-identical
            // to pre-wizard behavior.
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && interactive_output
                && std::io::stdout().is_terminal()
                && crate::wizard::is_wizard_eligible();

            if wizard_eligible {
                let mut api = ApiClient::from_auth(&auth)?;
                // Resolve id-or-name → canonical id BEFORE handing off to
                // the wizard. Refuses on ambiguous names (see
                // `find_key_by_name`) so we can never rotate the wrong key.
                let key_id = resolve_key_id(&mut api, &id).await?;
                // Best-effort fetch of the display name for the confirm
                // panel. Fallback to id if the fetch fails — non-fatal.
                let display_name = match api.get::<Value>(&format!("/api-keys/{key_id}")).await {
                    Ok(key) => key["name"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| key_id.clone()),
                    Err(_) => key_id.clone(),
                };
                let prefill = crate::wizard::RotatePrefill {
                    resource_id: key_id,
                    display_name,
                };
                return crate::wizard::run_api_key_rotate_wizard(&auth, prefill).await;
            }

            // Scripted / headless path — UNCHANGED from pre-wizard
            // behavior so existing CI / scripts keep working.
            let mut api = ApiClient::from_auth(&auth)?;
            let result: Value = api
                .post(&format!("/api-keys/{id}/rotate"), &serde_json::json!({}))
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let key = result["full_key"].as_str().unwrap_or("-");
                    eprintln!("Key rotated!");
                    eprintln!("New Key: {key}  (save this -- shown only once)");
                }
            }
            Ok(())
        }

        ApiKeyCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Revoke API key {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty(&format!("/api-keys/{id}")).await?;
            eprintln!("API key revoked.");
            Ok(())
        }

        ApiKeyCommands::Update {
            id,
            name,
            scopes,
            allowed_services,
            allowed_nodes,
            allow_all_services,
            allow_all_nodes,
            callback_url,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            let mut body = serde_json::Map::new();

            if let Some(name) = name {
                body.insert("name".into(), Value::String(name));
            }
            if let Some(scopes) = scopes {
                body.insert("scopes".into(), Value::String(scopes));
            }
            if let Some(services) = allowed_services {
                let ids: Vec<&str> = services.split(',').map(|s| s.trim()).collect();
                body.insert("allowed_service_ids".into(), serde_json::json!(ids));
            }
            if let Some(nodes) = allowed_nodes {
                let ids: Vec<&str> = nodes.split(',').map(|s| s.trim()).collect();
                body.insert("allowed_node_ids".into(), serde_json::json!(ids));
            }
            if let Some(v) = allow_all_services {
                body.insert("allow_all_services".into(), Value::Bool(v));
            }
            if let Some(v) = allow_all_nodes {
                body.insert("allow_all_nodes".into(), Value::Bool(v));
            }
            if let Some(url) = callback_url {
                if url.is_empty() {
                    body.insert("callback_url".into(), Value::Null);
                } else {
                    body.insert("callback_url".into(), Value::String(url));
                }
            }

            let _: Value = api.put(&format!("/api-keys/{id}"), &body).await?;
            eprintln!("API key updated.");
            Ok(())
        }

        ApiKeyCommands::Bind {
            id,
            service,
            credential,
            auth,
        } => bind_credential(&auth, &id, &service, credential.as_deref()).await,
    }
}

fn array_from_response<'a>(
    value: &'a serde_json::Value,
    field_names: &[&str],
) -> Option<&'a [serde_json::Value]> {
    field_names
        .iter()
        .find_map(|field| value.get(*field).and_then(|entry| entry.as_array()))
        .map(Vec::as_slice)
        .or_else(|| value.as_array().map(Vec::as_slice))
}

/// Find an API key by name, returning the full key object. Refuses to
/// proceed if the name is ambiguous (multiple keys with the same name)
/// — silently picking the first match could rotate / mutate the wrong
/// key, which is irreversible for rotation. Caller must use the ID
/// directly to disambiguate.
async fn find_key_by_name(api: &mut ApiClient, name: &str) -> Result<Value> {
    let keys: Value = api.get("/api-keys").await?;
    let items = array_from_response(&keys, &["keys", "api_keys"]);

    let matches: Vec<&Value> = items
        .map(|arr| {
            arr.iter()
                .filter(|k| k["name"].as_str() == Some(name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    match matches.len() {
        0 => anyhow::bail!("API key '{name}' not found. Run `nyxid api-key list` to see keys."),
        1 => Ok(matches[0].clone()),
        n => anyhow::bail!(
            "Name '{name}' matches {n} keys. Pass the ID instead — run `nyxid api-key list` to see them."
        ),
    }
}

/// Resolve an API key identifier (ID or name) to a key ID string.
async fn resolve_key_id(api: &mut ApiClient, id_or_name: &str) -> Result<String> {
    // Try as a direct ID first (GET /api-keys/{id})
    if let Ok(key) = api.get::<Value>(&format!("/api-keys/{id_or_name}")).await
        && let Some(key_id) = key["id"].as_str().or(key["_id"].as_str())
    {
        return Ok(key_id.to_string());
    }

    // Fall back to name lookup
    let key = find_key_by_name(api, id_or_name).await?;
    key["id"]
        .as_str()
        .or(key["_id"].as_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("Key has no ID"))
}

async fn bind_credential(
    auth: &AuthArgs,
    id_or_name: &str,
    service_slug: &str,
    credential_label: Option<&str>,
) -> Result<()> {
    let mut api = ApiClient::from_auth(auth)?;

    // Resolve API key ID
    let key_id = resolve_key_id(&mut api, id_or_name).await?;

    // Resolve service slug to UserService ID (via /keys which includes credential info)
    let keys: Value = api.get("/keys").await?;
    let keys_arr = array_from_response(&keys, &["keys"]).unwrap_or(&[]);
    let service = keys_arr
        .iter()
        .find(|s| s["slug"].as_str() == Some(service_slug))
        .ok_or_else(|| anyhow::anyhow!("Service '{service_slug}' not found"))?;
    let user_service_id = service["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Service has no ID"))?;

    // Resolve credential: use explicit label if provided, otherwise auto-resolve
    // from the service's configured credential (api_key_id field).
    let user_api_key_id = if let Some(label) = credential_label {
        let ext_keys: Value = api.get("/api-keys/external").await?;
        let ext_arr = array_from_response(&ext_keys, &["api_keys"]).unwrap_or(&[]);
        let cred = ext_arr
            .iter()
            .find(|k| k["label"].as_str() == Some(label) || k["name"].as_str() == Some(label))
            .ok_or_else(|| anyhow::anyhow!("Credential '{label}' not found"))?;
        cred["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Credential has no ID"))?
            .to_string()
    } else {
        // Auto-resolve from service's configured credential
        service["api_key_id"]
            .as_str()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Service '{service_slug}' has no credential configured. \
                     Either add a credential to the service first, or use --credential <label>."
                )
            })?
            .to_string()
    };

    // Create binding
    let body = serde_json::json!({
        "user_service_id": user_service_id,
        "user_api_key_id": user_api_key_id,
    });

    let resp: Value = api
        .post(&format!("/api-keys/{key_id}/bindings"), &body)
        .await?;

    match auth.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        OutputFormat::Table => {
            let binding_id = resp["id"].as_str().unwrap_or("-");
            eprintln!("Binding created: {binding_id}");
            eprintln!("  Key:     {id_or_name}");
            eprintln!("  Service: {service_slug}");
        }
    }

    Ok(())
}
