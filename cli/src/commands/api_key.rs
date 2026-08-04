use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ApiKeyCommands, AuthArgs, OutputFormat};
use crate::org_resolver::resolve_org_id;

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
            no_wait,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

            // Browser-flow gate — opens the local wizard when a
            // browser is available, or the remote pairing transport
            // (code + URL) otherwise. `--terminal` and
            // `NYXID_NO_WIZARD=1` fall through to the scripted path
            // below with byte-identical behavior to pre-wizard.
            //
            // `--no-wait` is specifically designed for agent
            // wrappers that want a resumable pairing id — and those
            // callers almost always request `--output json`. So
            // `--no-wait` wins over the `--output json → scripted`
            // gate; only `--terminal` overrides it.
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && (no_wait || (interactive_output && crate::wizard::is_browser_flow_eligible()));

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
                return crate::wizard::run_api_key_create_wizard(&auth, prefill, no_wait).await;
            }

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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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

        ApiKeyCommands::Rotate {
            id,
            terminal,
            no_wait,
            auth,
        } => {
            // Browser-flow gate — local wizard or remote pairing.
            // `--terminal` and `NYXID_NO_WIZARD=1` fall through to
            // the scripted path BELOW. `--no-wait` forces the
            // pairing variant and wins over both `--output json`
            // and local-browser preference — agent wrappers
            // specifically opt into the resumable pairing handoff.
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && (no_wait || (interactive_output && crate::wizard::is_browser_flow_eligible()));

            if wizard_eligible {
                let mut api = ApiClient::from_auth_checked(&auth).await?;
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
                return crate::wizard::run_api_key_rotate_wizard(&auth, prefill, no_wait).await;
            }

            // Scripted / headless path — UNCHANGED from pre-wizard
            // behavior so existing CI / scripts keep working.
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/api-keys/{id}")).await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("API key revoked."),
            }
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;

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
    let mut api = ApiClient::from_auth_checked(auth).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- Command-level integration tests (against a mock server) ---

    #[tokio::test]
    async fn create_posts_key_with_name_and_platform() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys"))
            .and(body_json(serde_json::json!({
                "name": "agent-key",
                "scopes": "read write",
                "platform": "claude-code"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "agent-key", "full_key": "nyxid_ag_secret", "scopes": "read write"
            })))
            .expect(1)
            .mount(&server)
            .await;

        // terminal: true forces the scripted (non-wizard) path that
        // existing CI/scripts use — byte-identical to pre-wizard.
        run(ApiKeyCommands::Create {
            name: Some("agent-key".to_string()),
            scopes: None,
            expires_in_days: None,
            allowed_services: None,
            allowed_nodes: None,
            allow_all_services: false,
            allow_all_nodes: false,
            platform: Some("claude-code".to_string()),
            callback_url: None,
            org: None,
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn list_fetches_keys_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"id": "key-1", "name": "agent-key", "scopes": "read write"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::List {
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn show_fetches_key_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/key-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "key-1", "name": "agent-key", "scopes": "read write"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Show {
            id: "key-1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("show should succeed");
    }

    #[tokio::test]
    async fn rotate_posts_rotate_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys/key-1/rotate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "full_key": "nyxid_ag_rotated" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Rotate {
            id: "key-1".to_string(),
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("rotate should succeed");
    }

    #[tokio::test]
    async fn delete_with_yes_issues_delete_request() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/api-keys/key-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Delete {
            id: "key-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("delete should succeed");
    }

    #[tokio::test]
    async fn update_sends_only_changed_fields() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/api-keys/key-1"))
            .and(body_json(serde_json::json!({ "name": "renamed" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Update {
            id: "key-1".to_string(),
            name: Some("renamed".to_string()),
            scopes: None,
            allowed_services: None,
            allowed_nodes: None,
            allow_all_services: None,
            allow_all_nodes: None,
            callback_url: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn bind_auto_resolves_service_credential_and_posts_binding() {
        let server = MockServer::start().await;
        // resolve_key_id → direct GET /api-keys/{id}
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/key-1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "id": "key-1", "name": "agent-key" })),
            )
            .mount(&server)
            .await;
        // service lookup via GET /keys (slug → id + configured api_key_id)
        Mock::given(method("GET"))
            .and(path("/api/v1/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"slug": "openai", "id": "svc-1", "api_key_id": "cred-1"}]
            })))
            .mount(&server)
            .await;
        // binding create with the auto-resolved credential
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys/key-1/bindings"))
            .and(body_json(serde_json::json!({
                "user_service_id": "svc-1",
                "user_api_key_id": "cred-1"
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "bind-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Bind {
            id: "key-1".to_string(),
            service: "openai".to_string(),
            credential: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("bind should succeed");
    }

    // --- Resolution-logic tests (security-relevant) ---

    #[tokio::test]
    async fn find_key_by_name_refuses_ambiguous_match() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"name": "dup", "id": "a"}, {"name": "dup", "id": "b"}]
            })))
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        let err = find_key_by_name(&mut api, "dup")
            .await
            .expect_err("ambiguous name must be refused");
        assert!(
            err.to_string().contains("matches 2"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn resolve_key_id_prefers_direct_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/abc123"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "abc123" })),
            )
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        let id = resolve_key_id(&mut api, "abc123").await.expect("resolve");
        assert_eq!(id, "abc123");
    }

    // --- Pure helper tests ---

    #[test]
    fn array_from_response_finds_first_present_field() {
        let v = serde_json::json!({ "api_keys": [1, 2] });
        let arr = array_from_response(&v, &["keys", "api_keys"]).expect("array");
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn array_from_response_falls_back_to_top_level_array() {
        let v = serde_json::json!([1, 2, 3]);
        assert_eq!(array_from_response(&v, &["keys"]).expect("array").len(), 3);
    }

    #[test]
    fn array_from_response_returns_none_when_absent() {
        let v = serde_json::json!({ "other": 1 });
        assert!(array_from_response(&v, &["keys"]).is_none());
    }
}

#[cfg(test)]
mod option_tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn create_includes_scope_and_callback_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys"))
            .and(body_partial_json(serde_json::json!({
                "allowed_service_ids": ["svc-a", "svc-b"],
                "allowed_node_ids": ["node-1"],
                "callback_url": "https://cb.example"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "name": "k", "full_key": "nyxid_ag_x" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Create {
            name: Some("k".to_string()),
            scopes: None,
            expires_in_days: None,
            allowed_services: Some("svc-a,svc-b".to_string()),
            allowed_nodes: Some("node-1".to_string()),
            allow_all_services: false,
            allow_all_nodes: false,
            platform: None,
            callback_url: Some("https://cb.example".to_string()),
            org: None,
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn create_sends_allow_all_flags() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys"))
            .and(body_partial_json(serde_json::json!({
                "allow_all_services": true, "allow_all_nodes": true
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "name": "k", "full_key": "nyxid_ag_x" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Create {
            name: Some("k".to_string()),
            scopes: None,
            expires_in_days: None,
            allowed_services: None,
            allowed_nodes: None,
            allow_all_services: true,
            allow_all_nodes: true,
            platform: None,
            callback_url: None,
            org: None,
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn update_includes_changed_scope_fields() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/api-keys/key-1"))
            .and(body_partial_json(serde_json::json!({
                "scopes": "read",
                "allowed_service_ids": ["svc-a"],
                "allow_all_nodes": true,
                "callback_url": "https://cb"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Update {
            id: "key-1".to_string(),
            name: None,
            scopes: Some("read".to_string()),
            allowed_services: Some("svc-a".to_string()),
            allowed_nodes: None,
            allow_all_services: None,
            allow_all_nodes: Some(true),
            callback_url: Some("https://cb".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn find_key_by_name_errors_when_absent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"name": "other", "id": "x"}]
            })))
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        let err = find_key_by_name(&mut api, "missing")
            .await
            .expect_err("absent name must error");
        assert!(err.to_string().contains("not found"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_key_id_falls_back_to_name_lookup() {
        let server = MockServer::start().await;
        // Direct id lookup 404s → falls back to name search via /api-keys.
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/myname"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"name": "myname", "id": "key-x"}]
            })))
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        let id = resolve_key_id(&mut api, "myname").await.expect("resolve");
        assert_eq!(id, "key-x");
    }

    #[tokio::test]
    async fn create_table_output_with_expiry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "k", "full_key": "nyxid_ag_x", "scopes": "read write",
                "expires_at": "2027-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        run(ApiKeyCommands::Create {
            name: Some("k".to_string()),
            scopes: Some("read write".to_string()),
            expires_in_days: Some(30),
            allowed_services: None,
            allowed_nodes: None,
            allow_all_services: false,
            allow_all_nodes: false,
            platform: None,
            callback_url: None,
            org: None,
            terminal: true,
            no_wait: false,
            auth: crate::test_support::mock_auth_with_output(
                server.uri(),
                crate::cli::OutputFormat::Table,
            ),
        })
        .await
        .expect("create table should succeed");
    }

    #[tokio::test]
    async fn list_table_renders_scope_columns() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{
                    "id": "key-1", "name": "agent", "scopes": "read",
                    "allow_all_services": false, "allow_all_nodes": false,
                    "allowed_services": [{"slug": "openai"}],
                    "allowed_nodes": [{"name": "laptop"}],
                    "last_used_at": "2026-05-20T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;

        run(ApiKeyCommands::List {
            org: None,
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
            .and(path("/api/v1/api-keys"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "keys": [] })),
            )
            .mount(&server)
            .await;

        run(ApiKeyCommands::List {
            org: None,
            auth: crate::test_support::mock_auth_with_output(
                server.uri(),
                crate::cli::OutputFormat::Table,
            ),
        })
        .await
        .expect("empty list should succeed");
    }

    #[tokio::test]
    async fn show_table_renders_scope_fields() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/key-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "key-1", "name": "agent", "scopes": "read write",
                "key_prefix": "nyxid_ag_", "expires_at": "2027-01-01",
                "last_used_at": "2026-05-20", "allow_all_services": true,
                "allow_all_nodes": true,
                "allowed_service_ids": ["svc-a"],
                "allowed_node_ids": ["node-a"]
            })))
            .mount(&server)
            .await;

        run(ApiKeyCommands::Show {
            id: "key-1".to_string(),
            auth: crate::test_support::mock_auth_with_output(
                server.uri(),
                crate::cli::OutputFormat::Table,
            ),
        })
        .await
        .expect("show table should succeed");
    }

    #[tokio::test]
    async fn rotate_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys/key-1/rotate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "full_key": "nyxid_ag_new" })),
            )
            .mount(&server)
            .await;

        run(ApiKeyCommands::Rotate {
            id: "key-1".to_string(),
            terminal: true,
            no_wait: false,
            auth: crate::test_support::mock_auth_with_output(
                server.uri(),
                crate::cli::OutputFormat::Table,
            ),
        })
        .await
        .expect("rotate table should succeed");
    }

    #[tokio::test]
    async fn update_clears_callback_url_with_empty_string() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/api-keys/key-1"))
            .and(body_partial_json(
                serde_json::json!({ "callback_url": null }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        run(ApiKeyCommands::Update {
            id: "key-1".to_string(),
            name: None,
            scopes: None,
            allowed_services: None,
            allowed_nodes: None,
            allow_all_services: None,
            allow_all_nodes: None,
            callback_url: Some(String::new()),
            auth: crate::test_support::mock_auth(server.uri()),
        })
        .await
        .expect("update clear callback should succeed");
    }

    #[tokio::test]
    async fn bind_explicit_credential_label_overrides_service_default() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/key-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "key-1" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"slug": "openai", "id": "svc-1", "api_key_id": "cred-default"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys/external"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_keys": [{"label": "Prod Key", "id": "cred-explicit"}]
            })))
            .mount(&server)
            .await;
        // The binding must use the LABEL-resolved credential (cred-explicit),
        // NOT the service's default api_key_id (cred-default).
        Mock::given(method("POST"))
            .and(path("/api/v1/api-keys/key-1/bindings"))
            .and(body_json(serde_json::json!({
                "user_service_id": "svc-1",
                "user_api_key_id": "cred-explicit"
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "bind-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ApiKeyCommands::Bind {
            id: "key-1".to_string(),
            service: "openai".to_string(),
            credential: Some("Prod Key".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("bind with label should succeed");
    }
}
