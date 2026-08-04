use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ApprovalCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

// Mirror the server-side caps in `approval_policy.rs` so obvious mistakes are
// caught offline; the backend remains the source of truth and re-validates.
const MAX_APPROVAL_RULES: usize = 50;
const MAX_RULE_METHODS: usize = 16;
const MAX_RULE_RESOURCE_PATTERN_LEN: usize = 256;

fn is_valid_effect(effect: &str) -> bool {
    matches!(effect, "auto_allow" | "require_approval" | "deny")
}

/// Parse a `--rule` spec (`key=value;key=value`) into the JSON `ApprovalRule`
/// shape the backend expects. Keys: effect, methods, path/resource, verbs,
/// mode. Unspecified methods/verbs mean "any"; resource_pattern defaults to
/// `*`; effect defaults to `require_approval`; mode defaults to `per_request`.
fn parse_approval_rule(spec: &str) -> Result<Value> {
    let mut methods: Vec<String> = Vec::new();
    let mut verbs: Vec<String> = Vec::new();
    let mut resource_pattern: Option<String> = None;
    let mut effect: Option<String> = None;
    let mut mode: Option<String> = None;

    for segment in spec.split(';') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let (key, value) = segment.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("Invalid rule segment '{segment}' (expected key=value).")
        })?;
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "effect" => effect = Some(value.to_ascii_lowercase()),
            "mode" => mode = Some(value.to_ascii_lowercase()),
            "path" | "resource" | "resource_pattern" => resource_pattern = Some(value.to_string()),
            "method" | "methods" => {
                methods = value
                    .split(',')
                    .map(|m| m.trim().to_ascii_uppercase())
                    .filter(|m| !m.is_empty())
                    .collect();
            }
            "verb" | "verbs" => {
                verbs = value
                    .split(',')
                    .map(|v| v.trim().to_ascii_lowercase())
                    .filter(|v| !v.is_empty())
                    .collect();
            }
            other => anyhow::bail!(
                "Unknown rule field '{other}'. Allowed: effect, methods, path, verbs, mode."
            ),
        }
    }

    let effect = effect.unwrap_or_else(|| "require_approval".to_string());
    if !is_valid_effect(&effect) {
        anyhow::bail!(
            "Invalid rule effect '{effect}'. Must be 'auto_allow', 'require_approval', or 'deny'."
        );
    }
    for verb in &verbs {
        if !matches!(verb.as_str(), "read" | "write" | "destructive") {
            anyhow::bail!("Invalid verb '{verb}'. Must be 'read', 'write', or 'destructive'.");
        }
    }
    if let Some(ref m) = mode
        && m != "per_request"
        && m != "grant"
    {
        anyhow::bail!("Invalid rule mode '{m}'. Must be 'per_request' or 'grant'.");
    }
    if methods.len() > MAX_RULE_METHODS {
        anyhow::bail!(
            "A rule can include at most {MAX_RULE_METHODS} methods (got {}).",
            methods.len()
        );
    }
    if let Some(ref pattern) = resource_pattern
        && pattern.chars().count() > MAX_RULE_RESOURCE_PATTERN_LEN
    {
        anyhow::bail!(
            "Resource pattern must be {MAX_RULE_RESOURCE_PATTERN_LEN} characters or less."
        );
    }

    let mut rule = serde_json::Map::new();
    rule.insert(
        "methods".into(),
        Value::Array(methods.into_iter().map(Value::String).collect()),
    );
    rule.insert(
        "verbs".into(),
        Value::Array(verbs.into_iter().map(Value::String).collect()),
    );
    rule.insert(
        "resource_pattern".into(),
        Value::String(resource_pattern.unwrap_or_else(|| "*".to_string())),
    );
    rule.insert("effect".into(), Value::String(effect));
    rule.insert(
        "mode".into(),
        Value::String(mode.unwrap_or_else(|| "per_request".to_string())),
    );
    Ok(Value::Object(rule))
}

pub async fn run(command: ApprovalCommands) -> Result<()> {
    match command {
        ApprovalCommands::List { org, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = resolve_optional_org(&mut api, org).await?;
            let path = match org {
                Some(ref id) => {
                    format!("/approvals/requests?org_id={}", urlencoding::encode(id))
                }
                None => "/approvals/requests".to_string(),
            };
            let requests: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&requests)?);
                }
                OutputFormat::Table => {
                    let items = requests
                        .get("requests")
                        .and_then(|v| v.as_array())
                        .or_else(|| requests.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No approval requests.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "ID",
                            "Service",
                            "Status",
                            "Action",
                            "Requester",
                            "Created",
                        ]);

                        for req in items {
                            let id = req["id"].as_str().or(req["_id"].as_str()).unwrap_or("-");
                            let short_id = crate::commands::short_id(id);
                            let service = req["service_name"]
                                .as_str()
                                .or(req["service_slug"].as_str())
                                .unwrap_or("-");
                            let status = req["status"].as_str().unwrap_or("-");
                            let action = req["action_description"]
                                .as_str()
                                .or(req["operation_summary"].as_str())
                                .unwrap_or("-");
                            let requester = req["requester_label"]
                                .as_str()
                                .or(req["requester_type"].as_str())
                                .unwrap_or("-");
                            let created = req["created_at"].as_str().unwrap_or("-");
                            table.add_row([short_id, service, status, action, requester, created]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ApprovalCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let req: Value = api.get(&format!("/approvals/requests/{id}")).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&req)?);
                }
                OutputFormat::Table => {
                    let req_id = req["id"].as_str().or(req["_id"].as_str()).unwrap_or(&id);
                    let service = req["service_name"]
                        .as_str()
                        .or(req["service_slug"].as_str())
                        .unwrap_or("-");
                    let status = req["status"].as_str().unwrap_or("-");
                    let requester = req["requester_label"]
                        .as_str()
                        .or(req["requester_type"].as_str())
                        .unwrap_or("-");
                    let created = req["created_at"].as_str().unwrap_or("-");
                    let summary = req["operation_summary"].as_str().unwrap_or("-");
                    let description = req["action_description"].as_str().unwrap_or(summary);

                    eprintln!("Approval Request");
                    eprintln!();
                    eprintln!("ID:          {req_id}");
                    eprintln!("Service:     {service}");
                    eprintln!("Status:      {status}");
                    eprintln!("Requester:   {requester}");
                    eprintln!("Operation:   {summary}");
                    // Granular operation identity (present on requests created by
                    // the method/path/verb rule engine; absent on legacy requests).
                    if let Some(method) = req["http_method"].as_str() {
                        eprintln!("Method:      {method}");
                    }
                    if let Some(resource) = req["resource"].as_str() {
                        eprintln!("Resource:    {resource}");
                    }
                    if let Some(verb) = req["verb"].as_str() {
                        eprintln!("Verb:        {verb}");
                    }
                    eprintln!("Description: {description}");
                    eprintln!("Created:     {created}");
                }
            }
            Ok(())
        }

        ApprovalCommands::Approve { id, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let body = serde_json::json!({ "decision": "approved" });
            let result: Value = api
                .post(&format!("/approvals/requests/{id}/decide"), &body)
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Request {id} approved.");
                }
            }
            Ok(())
        }

        ApprovalCommands::Deny { id, reason, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut body = serde_json::json!({ "decision": "denied" });
            if let Some(reason) = reason {
                body["reason"] = Value::String(reason);
            }
            let result: Value = api
                .post(&format!("/approvals/requests/{id}/decide"), &body)
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Request {id} denied.");
                }
            }
            Ok(())
        }

        ApprovalCommands::Grants { org, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = resolve_optional_org(&mut api, org).await?;
            let path = match org {
                Some(ref id) => {
                    format!("/approvals/grants?org_id={}", urlencoding::encode(id))
                }
                None => "/approvals/grants".to_string(),
            };
            let grants: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&grants)?);
                }
                OutputFormat::Table => {
                    let items = grants
                        .get("grants")
                        .and_then(|v| v.as_array())
                        .or_else(|| grants.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No approval grants.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "ID",
                            "Service",
                            "Requester",
                            "Scope",
                            "Granted",
                            "Expires",
                        ]);

                        for grant in items {
                            let gid = grant["id"]
                                .as_str()
                                .or(grant["_id"].as_str())
                                .unwrap_or("-");
                            let short_id = crate::commands::short_id(gid);
                            let service = grant["service_name"].as_str().unwrap_or("-");
                            let requester = grant["requester_label"]
                                .as_str()
                                .or(grant["requester_type"].as_str())
                                .unwrap_or("-");
                            // A null/absent scope is a legacy service-wide grant
                            // (covers every operation on the service).
                            let scope = grant["scope"].as_str().unwrap_or("service-wide");
                            let granted = grant["granted_at"].as_str().unwrap_or("-");
                            let expires = grant["expires_at"].as_str().unwrap_or("never");
                            table.add_row([short_id, service, requester, scope, granted, expires]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ApprovalCommands::RevokeGrant { id, org, yes, auth } => {
            if !yes {
                eprint!("Revoke grant {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = resolve_optional_org(&mut api, org).await?;
            let path = match org {
                Some(ref org_id) => format!(
                    "/approvals/grants/{id}?org_id={}",
                    urlencoding::encode(org_id)
                ),
                None => format!("/approvals/grants/{id}"),
            };
            api.delete_empty(&path).await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Grant {id} revoked."),
            }
            Ok(())
        }

        ApprovalCommands::Enable { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let body = serde_json::json!({ "approval_required": true });
            let result: Value = api.put("/notifications/settings", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!(
                        "Global approval protection enabled. Services without per-service overrides now require your approval."
                    );
                }
            }
            Ok(())
        }

        ApprovalCommands::Disable { yes, auth } => {
            if !yes {
                eprint!(
                    "Disable global approval protection? Services without per-service overrides will stop requiring approval. [y/N] "
                );
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let body = serde_json::json!({ "approval_required": false });
            let result: Value = api.put("/notifications/settings", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!(
                        "Global approval protection disabled. Per-service overrides, if any, still take precedence."
                    );
                }
            }
            Ok(())
        }

        ApprovalCommands::ServiceConfigs { org, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = resolve_optional_org(&mut api, org).await?;
            let path = match org {
                Some(ref id) => {
                    format!(
                        "/approvals/service-configs?org_id={}",
                        urlencoding::encode(id)
                    )
                }
                None => "/approvals/service-configs".to_string(),
            };
            let configs: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&configs)?);
                }
                OutputFormat::Table => {
                    let items = configs
                        .get("configs")
                        .and_then(|v| v.as_array())
                        .or_else(|| configs.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No per-service approval configurations.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "Service ID",
                            "Service",
                            "Approval Required",
                            "Mode",
                            "Default Effect",
                            "Rules",
                        ]);

                        for cfg in items {
                            let cid = cfg["service_id"].as_str().unwrap_or("-");
                            let short_id = crate::commands::short_id(cid);
                            let service = cfg["service_name"].as_str().unwrap_or("-");
                            let require = cfg["approval_required"]
                                .as_bool()
                                .map(|b| b.to_string())
                                .unwrap_or_else(|| "-".to_string());
                            let mode = cfg["approval_mode"].as_str().unwrap_or("per_request");
                            let default_effect = cfg["default_effect"].as_str().unwrap_or("-");
                            let rule_count = cfg["rules"]
                                .as_array()
                                .map(|r| r.len())
                                .unwrap_or(0)
                                .to_string();
                            table.add_row([
                                short_id,
                                service,
                                &require,
                                mode,
                                default_effect,
                                &rule_count,
                            ]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ApprovalCommands::SetConfig {
            id,
            require_approval,
            approval_mode,
            default_effect,
            rules,
            clear_rules,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = resolve_optional_org(&mut api, org).await?;
            let mut body = serde_json::Map::new();

            if let Some(v) = require_approval {
                body.insert("approval_required".into(), Value::Bool(v));
            }

            if let Some(ref mode) = approval_mode {
                if mode != "per_request" && mode != "grant" {
                    anyhow::bail!(
                        "Invalid approval mode: {mode}. Must be 'per_request' or 'grant'."
                    );
                }
                body.insert("approval_mode".into(), Value::String(mode.clone()));
            }

            if let Some(ref effect) = default_effect {
                if !is_valid_effect(effect) {
                    anyhow::bail!(
                        "Invalid default effect: {effect}. Must be 'auto_allow', 'require_approval', or 'deny'."
                    );
                }
                body.insert("default_effect".into(), Value::String(effect.clone()));
            }

            if clear_rules {
                // Explicit empty array clears any granular rules server-side.
                body.insert("rules".into(), Value::Array(Vec::new()));
            } else if !rules.is_empty() {
                if rules.len() > MAX_APPROVAL_RULES {
                    anyhow::bail!(
                        "At most {MAX_APPROVAL_RULES} rules are allowed (got {}).",
                        rules.len()
                    );
                }
                let parsed: Vec<Value> = rules
                    .iter()
                    .map(|spec| parse_approval_rule(spec))
                    .collect::<Result<_>>()?;
                body.insert("rules".into(), Value::Array(parsed));
            }

            if body.is_empty() {
                eprintln!(
                    "No updates specified. Use --require-approval, --approval-mode, --default-effect, --rule, or --clear-rules."
                );
                return Ok(());
            }

            let path = match org {
                Some(ref org_id) => format!(
                    "/approvals/service-configs/{id}?org_id={}",
                    urlencoding::encode(org_id)
                ),
                None => format!("/approvals/service-configs/{id}"),
            };
            let result: Value = api.put(&path, &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Approval config updated for {id}.");
                }
            }
            Ok(())
        }
    }
}

async fn resolve_optional_org(api: &mut ApiClient, org: Option<String>) -> Result<Option<String>> {
    match org {
        Some(raw) => Ok(Some(resolve_org_id(api, &raw).await?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ORG_UUID: &str = "00000000-0000-0000-0000-0000000000aa";

    #[tokio::test]
    async fn list_fetches_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "requests": [{"id": "r1", "service_name": "openai", "status": "pending"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::List {
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn approve_posts_approved_decision() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/approvals/requests/r1/decide"))
            .and(body_json(serde_json::json!({ "decision": "approved" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::Approve {
            id: "r1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("approve should succeed");
    }

    #[tokio::test]
    async fn deny_includes_reason_in_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/approvals/requests/r1/decide"))
            .and(body_json(
                serde_json::json!({ "decision": "denied", "reason": "nope" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::Deny {
            id: "r1".to_string(),
            reason: Some("nope".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("deny should succeed");
    }

    #[tokio::test]
    async fn set_config_rejects_invalid_mode() {
        let server = MockServer::start().await;
        // Invalid mode must bail before any HTTP request.
        let result = run(ApprovalCommands::SetConfig {
            id: "svc-1".to_string(),
            require_approval: None,
            approval_mode: Some("bogus".to_string()),
            default_effect: None,
            rules: Vec::new(),
            clear_rules: false,
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "invalid approval mode must be rejected");
    }

    #[tokio::test]
    async fn set_config_puts_required_and_mode() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/approvals/service-configs/svc-1"))
            .and(body_json(serde_json::json!({
                "approval_required": true, "approval_mode": "grant"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::SetConfig {
            id: "svc-1".to_string(),
            require_approval: Some(true),
            approval_mode: Some("grant".to_string()),
            default_effect: None,
            rules: Vec::new(),
            clear_rules: false,
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("set-config should succeed");
    }

    #[tokio::test]
    async fn revoke_grant_with_yes_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/approvals/grants/g1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::RevokeGrant {
            id: "g1".to_string(),
            org: None,
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("revoke-grant should succeed");
    }

    #[tokio::test]
    async fn enable_sets_approval_required() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/notifications/settings"))
            .and(body_json(serde_json::json!({ "approval_required": true })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::Enable {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("enable should succeed");
    }

    #[tokio::test]
    async fn show_fetches_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/requests/r1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "r1", "service_name": "openai", "status": "pending"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::Show {
            id: "r1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("show should succeed");
    }

    #[tokio::test]
    async fn grants_fetches_grants() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/grants"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "grants": [{"id": "g1", "service_name": "openai"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::Grants {
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("grants should succeed");
    }

    #[tokio::test]
    async fn disable_clears_approval_required() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/notifications/settings"))
            .and(body_json(serde_json::json!({ "approval_required": false })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::Disable {
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("disable should succeed");
    }

    #[tokio::test]
    async fn service_configs_fetches_configs() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/service-configs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configs": [{"service_id": "svc-1", "service_name": "openai", "approval_required": true}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::ServiceConfigs {
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("service-configs should succeed");
    }

    #[tokio::test]
    async fn list_with_org_scopes_request_path() {
        let server = MockServer::start().await;
        // A UUID org short-circuits resolution (no /orgs lookup) and is
        // appended as ?org_id=… on the requests path.
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/requests"))
            .and(query_param("org_id", ORG_UUID))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "requests": [] })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::List {
            org: Some(ORG_UUID.to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("org-scoped list should succeed");
    }

    // --- Decision / set-config edge cases ---

    #[tokio::test]
    async fn deny_without_reason_omits_reason_field() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/approvals/requests/r1/decide"))
            .and(body_json(serde_json::json!({ "decision": "denied" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::Deny {
            id: "r1".to_string(),
            reason: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("deny without reason should succeed");
    }

    #[tokio::test]
    async fn set_config_with_no_fields_is_noop() {
        let server = MockServer::start().await;
        // No flags → "No updates specified", returns Ok without any HTTP.
        run(ApprovalCommands::SetConfig {
            id: "svc-1".to_string(),
            require_approval: None,
            approval_mode: None,
            default_effect: None,
            rules: Vec::new(),
            clear_rules: false,
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("no-op set-config should succeed");
    }

    #[tokio::test]
    async fn list_table_renders_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "requests": [{"id": "r1-abcdef12", "service_name": "openai", "status": "pending",
                              "action_description": "chat", "requester_label": "agent", "created_at": "2026-01-01"}]
            })))
            .mount(&server)
            .await;

        run(ApprovalCommands::List {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("list table should succeed");
    }

    #[tokio::test]
    async fn list_table_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/requests"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "requests": [] })),
            )
            .mount(&server)
            .await;

        run(ApprovalCommands::List {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("empty list should succeed");
    }

    #[tokio::test]
    async fn show_table_renders_detail() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/requests/r1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "r1", "service_name": "openai", "status": "pending",
                "operation_summary": "POST /v1/chat", "action_description": "Chat request",
                "requester_label": "coding-agent", "created_at": "2026-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        run(ApprovalCommands::Show {
            id: "r1".to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("show table should succeed");
    }

    #[tokio::test]
    async fn grants_table_renders_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/grants"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "grants": [{"id": "g1-abcdef12", "service_name": "openai",
                            "requester_label": "agent", "granted_at": "2026-01-01", "expires_at": "2027-01-01"}]
            })))
            .mount(&server)
            .await;

        run(ApprovalCommands::Grants {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("grants table should succeed");
    }

    #[tokio::test]
    async fn grants_table_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/grants"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "grants": [] })),
            )
            .mount(&server)
            .await;

        run(ApprovalCommands::Grants {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("empty grants should succeed");
    }

    #[tokio::test]
    async fn service_configs_table_renders_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/service-configs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configs": [{"service_id": "svc-abcdef12", "service_name": "openai",
                             "approval_required": true, "approval_mode": "grant"}]
            })))
            .mount(&server)
            .await;

        run(ApprovalCommands::ServiceConfigs {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("service-configs table should succeed");
    }

    #[tokio::test]
    async fn service_configs_table_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/service-configs"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "configs": [] })),
            )
            .mount(&server)
            .await;

        run(ApprovalCommands::ServiceConfigs {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("empty configs should succeed");
    }

    #[tokio::test]
    async fn enable_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/notifications/settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        run(ApprovalCommands::Enable {
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("enable table should succeed");
    }

    #[tokio::test]
    async fn revoke_grant_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/approvals/grants/g1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::RevokeGrant {
            id: "g1".to_string(),
            org: None,
            yes: true,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("revoke-grant table should succeed");
    }

    // --- Granular rule parsing & set-config ---

    #[test]
    fn parse_rule_applies_defaults() {
        let rule = parse_approval_rule("verbs=write").expect("rule should parse");
        assert_eq!(rule["effect"], "require_approval");
        assert_eq!(rule["mode"], "per_request");
        assert_eq!(rule["resource_pattern"], "*");
        assert_eq!(rule["verbs"], serde_json::json!(["write"]));
        assert_eq!(rule["methods"], serde_json::json!([]));
    }

    #[test]
    fn parse_rule_full_spec_normalizes_case() {
        let rule =
            parse_approval_rule("effect=Deny;methods=delete,post;path=/v1/admin/*;mode=grant")
                .expect("rule should parse");
        assert_eq!(rule["effect"], "deny");
        assert_eq!(rule["methods"], serde_json::json!(["DELETE", "POST"]));
        assert_eq!(rule["resource_pattern"], "/v1/admin/*");
        assert_eq!(rule["mode"], "grant");
    }

    #[test]
    fn parse_rule_rejects_bad_effect() {
        assert!(parse_approval_rule("effect=maybe").is_err());
    }

    #[test]
    fn parse_rule_rejects_bad_verb() {
        assert!(parse_approval_rule("verbs=sideways").is_err());
    }

    #[test]
    fn parse_rule_rejects_unknown_field() {
        assert!(parse_approval_rule("color=blue").is_err());
    }

    #[test]
    fn parse_rule_rejects_missing_equals() {
        assert!(parse_approval_rule("effect").is_err());
    }

    #[tokio::test]
    async fn set_config_puts_rules_and_default_effect() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/approvals/service-configs/svc-1"))
            .and(body_json(serde_json::json!({
                "default_effect": "deny",
                "rules": [
                    {
                        "methods": ["DELETE"],
                        "verbs": [],
                        "resource_pattern": "*",
                        "effect": "deny",
                        "mode": "per_request"
                    },
                    {
                        "methods": [],
                        "verbs": ["write"],
                        "resource_pattern": "/v1/chat/*",
                        "effect": "require_approval",
                        "mode": "grant"
                    }
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::SetConfig {
            id: "svc-1".to_string(),
            require_approval: None,
            approval_mode: None,
            default_effect: Some("deny".to_string()),
            rules: vec![
                "effect=deny;methods=DELETE".to_string(),
                "verbs=write;path=/v1/chat/*;mode=grant".to_string(),
            ],
            clear_rules: false,
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("set-config with rules should succeed");
    }

    #[tokio::test]
    async fn set_config_clear_rules_sends_empty_array() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/approvals/service-configs/svc-1"))
            .and(body_json(serde_json::json!({ "rules": [] })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ApprovalCommands::SetConfig {
            id: "svc-1".to_string(),
            require_approval: None,
            approval_mode: None,
            default_effect: None,
            rules: Vec::new(),
            clear_rules: true,
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("clear-rules should succeed");
    }

    #[tokio::test]
    async fn set_config_rejects_invalid_default_effect() {
        let server = MockServer::start().await;
        let result = run(ApprovalCommands::SetConfig {
            id: "svc-1".to_string(),
            require_approval: None,
            approval_mode: None,
            default_effect: Some("maybe".to_string()),
            rules: Vec::new(),
            clear_rules: false,
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "invalid default effect must be rejected");
    }

    #[tokio::test]
    async fn set_config_rejects_invalid_rule_before_http() {
        let server = MockServer::start().await;
        // A bad rule must fail before any HTTP request is made (no mock mounted).
        let result = run(ApprovalCommands::SetConfig {
            id: "svc-1".to_string(),
            require_approval: None,
            approval_mode: None,
            default_effect: None,
            rules: vec!["effect=nonsense".to_string()],
            clear_rules: false,
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "invalid rule must be rejected");
    }

    #[tokio::test]
    async fn grants_table_renders_scope() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/grants"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "grants": [{"id": "g1-abcdef12", "service_name": "openai",
                            "requester_label": "agent", "scope": "v1:http:post:write:/v1/chat/completions",
                            "granted_at": "2026-01-01", "expires_at": "2027-01-01"}]
            })))
            .mount(&server)
            .await;

        run(ApprovalCommands::Grants {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("grants table with scope should succeed");
    }

    #[tokio::test]
    async fn service_configs_table_renders_rules() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approvals/service-configs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configs": [{"service_id": "svc-abcdef12", "service_name": "openai",
                             "approval_required": false, "approval_mode": "per_request",
                             "default_effect": "auto_allow",
                             "rules": [{"methods": ["DELETE"], "effect": "deny",
                                        "resource_pattern": "*", "verbs": [], "mode": "per_request"}]}]
            })))
            .mount(&server)
            .await;

        run(ApprovalCommands::ServiceConfigs {
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("service-configs table with rules should succeed");
    }
}
