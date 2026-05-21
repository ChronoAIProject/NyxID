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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;

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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
                let mut api = ApiClient::from_auth_checked(&auth).await?;
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

            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // A literal UUID — resolve_org_id returns it directly without an HTTP
    // call, so the request body should carry it verbatim as target_org_id.
    const ORG_UUID: &str = "11111111-1111-1111-1111-111111111111";

    // --- Create (scripted path; --terminal bypasses the browser wizard) ---

    #[tokio::test]
    async fn create_scripted_posts_minimal_body() {
        let server = MockServer::start().await;
        // body_json is exact: confirms only name + allowed_scopes are sent
        // when no optional flags are supplied (description/rate_limit/roles
        // must be absent, NOT null).
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/service-accounts"))
            .and(body_json(serde_json::json!({
                "name": "ci-bot",
                "allowed_scopes": "openid profile"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "sa-1",
                "name": "ci-bot",
                "client_id": "cid",
                "client_secret": "shh",
                "allowed_scopes": "openid profile"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::Create {
            name: "ci-bot".to_string(),
            scopes: "openid profile".to_string(),
            description: None,
            rate_limit_override: None,
            role_ids: None,
            org: None,
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create should succeed");
    }

    #[tokio::test]
    async fn create_serializes_all_optionals_and_org_and_splits_role_csv() {
        let server = MockServer::start().await;
        // Exact body asserts the body-building decision logic:
        //  - description string
        //  - rate_limit_override as a JSON number (not string)
        //  - role_ids CSV is split, trimmed, empties dropped -> array
        //  - org UUID lands in target_org_id (resolve_org_id is a no-op for UUID)
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/service-accounts"))
            .and(body_json(serde_json::json!({
                "name": "ci-bot",
                "allowed_scopes": "openid",
                "description": "build runner",
                "rate_limit_override": 25,
                "role_ids": ["role-a", "role-b"],
                "target_org_id": ORG_UUID
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "sa-2" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::Create {
            name: "ci-bot".to_string(),
            scopes: "openid".to_string(),
            description: Some("build runner".to_string()),
            rate_limit_override: Some(25),
            // leading/trailing spaces + a trailing empty segment exercise the
            // trim()/filter(!is_empty) path in the CSV parser.
            role_ids: Some(" role-a , role-b , ".to_string()),
            org: Some(ORG_UUID.to_string()),
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("create with optionals should succeed");
    }

    #[tokio::test]
    async fn create_surfaces_server_error() {
        let server = MockServer::start().await;
        // 403 from the backend (e.g. non-admin) must bubble up as Err,
        // not be swallowed.
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/service-accounts"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "error": "forbidden"
            })))
            .mount(&server)
            .await;

        let result = run(ServiceAccountCommands::Create {
            name: "ci-bot".to_string(),
            scopes: "openid".to_string(),
            description: None,
            rate_limit_override: None,
            role_ids: None,
            org: None,
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "403 from backend must surface as error");
    }

    // --- List (query-string construction + per_page clamp) ---

    #[tokio::test]
    async fn list_clamps_per_page_and_forwards_search_and_org() {
        let server = MockServer::start().await;
        // per_page=250 must be clamped to 100 (per_page.min(100)); search
        // and org are URL-encoded and forwarded as query params.
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/service-accounts"))
            .and(query_param("page", "2"))
            .and(query_param("per_page", "100"))
            .and(query_param("search", "bot"))
            .and(query_param("org_id", ORG_UUID))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "service_accounts": [{"id": "sa-1", "name": "bot"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::List {
            org: Some(ORG_UUID.to_string()),
            search: Some("bot".to_string()),
            page: 2,
            per_page: 250,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("list should succeed");
    }

    // --- Show ---

    #[tokio::test]
    async fn show_fetches_by_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/service-accounts/sa-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "sa-1",
                "name": "bot"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::Show {
            id: "sa-1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("show should succeed");
    }

    // --- Update (only-changed-fields PATCH semantics over PUT) ---

    #[tokio::test]
    async fn update_sends_only_supplied_fields() {
        let server = MockServer::start().await;
        // Exact body: name + is_active=false are set, every other column is
        // None and must be omitted from the body entirely.
        Mock::given(method("PUT"))
            .and(path("/api/v1/admin/service-accounts/sa-1"))
            .and(body_json(serde_json::json!({
                "name": "renamed",
                "is_active": false
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "sa-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::Update {
            id: "sa-1".to_string(),
            name: Some("renamed".to_string()),
            description: None,
            scopes: None,
            role_ids: None,
            is_active: Some(false),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn update_splits_role_ids_csv() {
        let server = MockServer::start().await;
        // Confirms Update reuses the same CSV split/trim logic as Create.
        Mock::given(method("PUT"))
            .and(path("/api/v1/admin/service-accounts/sa-1"))
            .and(body_json(serde_json::json!({
                "allowed_scopes": "openid email",
                "role_ids": ["r1", "r2"]
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "sa-1" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::Update {
            id: "sa-1".to_string(),
            name: None,
            description: None,
            scopes: Some("openid email".to_string()),
            role_ids: Some("r1, ,r2".to_string()),
            is_active: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    // --- Delete (--yes skips the stdin confirm prompt) ---

    #[tokio::test]
    async fn delete_with_yes_issues_delete_request() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/admin/service-accounts/sa-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::Delete {
            id: "sa-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("delete should succeed");
    }

    // --- RotateSecret (scripted path; --terminal bypasses the wizard) ---

    #[tokio::test]
    async fn rotate_secret_scripted_posts_rotate_endpoint() {
        let server = MockServer::start().await;
        // null body — the rotate POST carries no payload.
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/service-accounts/sa-1/rotate-secret"))
            .and(body_json(serde_json::Value::Null))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "client_id": "cid",
                "client_secret": "new-secret"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::RotateSecret {
            id: "sa-1".to_string(),
            terminal: true,
            no_wait: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("rotate-secret should succeed");
    }

    // --- RevokeTokens ---

    #[tokio::test]
    async fn revoke_tokens_posts_revoke_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/service-accounts/sa-1/revoke-tokens"))
            .and(body_json(serde_json::Value::Null))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "revoked_count": 3 })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ServiceAccountCommands::RevokeTokens {
            id: "sa-1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("revoke-tokens should succeed");
    }
}
