use anyhow::{Result, anyhow, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, PoolCommands, PoolFailoverArgs, PoolRetryCauseArg};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: PoolCommands) -> Result<()> {
    match command {
        PoolCommands::Create {
            slug,
            name,
            description,
            strategy,
            members,
            members_file,
            contract,
            tier_balance,
            failover_file,
            org,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org_id = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

            let mut body = serde_json::json!({
                "slug": slug,
                "name": name,
                "strategy": strategy.as_str(),
                "members": members
                    .into_iter()
                    .map(|user_service_id| serde_json::json!({ "user_service_id": user_service_id }))
                    .collect::<Vec<_>>(),
            });
            body["member_contract"] = Value::String(contract);
            body["tier_balance"] = Value::String(tier_balance);
            if let Some(file) = members_file {
                if !body["members"].as_array().is_some_and(Vec::is_empty) {
                    bail!("Choose --member or --members-file");
                }
                let members = read_json(&file)?;
                if !members.is_array() {
                    bail!("Members file must contain a JSON array");
                }
                body["members"] = members;
            }
            if let Some(file) = failover_file {
                body["failover"] = read_json(&file)?;
            }
            insert_opt_str(&mut body, "description", description.as_deref());
            insert_opt_str(&mut body, "org_id", org_id.as_deref());

            let pool: Value = api.post("/service-pools", &body).await?;
            print_pool_created(output, &pool)
        }
        PoolCommands::List { org, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org_id = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let path = service_pools_path(org_id.as_deref());
            let pools: Value = api.get(&path).await?;
            print_pool_list(output, &pools)
        }
        PoolCommands::Show {
            pool,
            org,
            method,
            path,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let mut shown: Value = api
                .get(&format!("/service-pools/{}", urlencoding::encode(&pool)))
                .await?;
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            if let Some(method) = method {
                query.append_pair("method", &method);
            }
            if let Some(path) = path {
                query.append_pair("path", &path);
            }
            let query = query.finish();
            let health: Value = api
                .get(&format!(
                    "/service-pools/{}/health{}",
                    urlencoding::encode(&pool),
                    if query.is_empty() {
                        String::new()
                    } else {
                        format!("?{query}")
                    }
                ))
                .await?;
            shown["health"] = health;
            print_pool(output, &shown)
        }
        PoolCommands::Delete { pool_id, org, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool_id = resolve_pool(&mut api, &pool_id, org.as_deref()).await?;
            api.delete_empty(&format!("/service-pools/{}", urlencoding::encode(&pool_id)))
                .await?;
            match output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Service pool deleted."),
            }
            Ok(())
        }
        PoolCommands::AddMember {
            pool,
            service,
            weight,
            enabled,
            priority,
            model,
            clear_model,
            same_api_compatible,
            org,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let mut body = serde_json::json!({ "user_service_id": service });
            if let Some(weight) = weight {
                body["weight"] = serde_json::json!(weight);
            }
            if let Some(enabled) = enabled {
                body["enabled"] = Value::Bool(enabled);
            }
            if let Some(priority) = priority {
                body["priority"] = priority.into();
            }
            if let Some(model) = model {
                body["model"] = model.into();
            }
            if clear_model {
                body["model"] = Value::Null;
            }
            if let Some(declared) = same_api_compatible {
                body["same_api_compatible"] = declared.into();
            }
            let pool: Value = api
                .post(
                    &format!("/service-pools/{}/members", urlencoding::encode(&pool)),
                    &body,
                )
                .await?;
            print_pool(output, &pool)
        }
        PoolCommands::RemoveMember {
            pool,
            service,
            org,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let pool: Value = api
                .delete(&format!(
                    "/service-pools/{}/members/{}",
                    urlencoding::encode(&pool),
                    urlencoding::encode(&service)
                ))
                .await?;
            print_pool(output, &pool)
        }
        PoolCommands::SetStrategy {
            pool,
            strategy,
            tier_balance,
            org,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let mut body = serde_json::json!({ "strategy": strategy.as_str() });
            insert_opt_str(&mut body, "tier_balance", tier_balance.as_deref());
            let (endpoint, current) = pool_snapshot(&mut api, &pool).await?;
            body["expected_revision"] = snapshot_revision(&current)?;
            let pool: Value = api.put(&endpoint, &body).await?;
            print_pool(output, &pool)
        }
        PoolCommands::Update {
            pool,
            file,
            org,
            auth,
        } => {
            let mut body = read_json(&file)?;
            if !body.is_object() {
                bail!("Update file must contain a JSON object");
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let (endpoint, current) = pool_snapshot(&mut api, &pool).await?;
            if body.get("expected_revision").is_none() {
                body["expected_revision"] = snapshot_revision(&current)?;
            }
            let saved = api.put(&endpoint, &body).await?;
            print_pool(auth.output, &saved)
        }
        PoolCommands::SetFailover {
            pool,
            file,
            disable,
            defaults,
            settings,
            org,
            auth,
        } => {
            let patch = failover_patch(&settings)?;
            let inline = !patch.as_object().expect("patch is an object").is_empty();
            if [file.is_some(), disable, defaults, inline]
                .into_iter()
                .filter(|selected| *selected)
                .count()
                != 1
            {
                bail!("Choose policy flags or exactly one of --file, --defaults, --disable");
            }
            let replacement = if disable {
                Some(serde_json::json!({"max_attempts":1}))
            } else if let Some(file) = file {
                let policy = read_json(&file)?;
                if !policy.is_object() && !policy.is_null() {
                    bail!("Failover file must contain a JSON object or null");
                }
                Some(policy)
            } else if defaults {
                Some(Value::Null)
            } else {
                None
            };
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let (endpoint, current) = pool_snapshot(&mut api, &pool).await?;
            let policy = match replacement {
                Some(policy) => policy,
                None => merge_failover(current.get("failover"), patch)?,
            };
            let body = serde_json::json!({
                "failover":policy,
                "expected_revision":snapshot_revision(&current)?,
            });
            let saved = api.put(&endpoint, &body).await?;
            print_pool(auth.output, &saved)
        }
        PoolCommands::Candidates {
            pool,
            contract,
            strategy,
            after,
            search,
            limit,
            method,
            path,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let endpoint = if let Some(pool) = pool {
                let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
                format!("/service-pools/{}/candidates", urlencoding::encode(&pool))
            } else {
                "/service-pools/candidates".into()
            };
            let mut query = format!("{endpoint}?method={}", urlencoding::encode(&method));
            if let Some(contract) = contract {
                query.push_str(&format!(
                    "&member_contract={}",
                    urlencoding::encode(&contract)
                ));
            }
            if let Some(strategy) = strategy {
                query.push_str(&format!("&strategy={}", strategy.as_str()));
            }
            if let Some(path) = path {
                query.push_str(&format!("&path={}", urlencoding::encode(&path)));
            }
            if let Some(org) = org {
                let org = resolve_org_id(&mut api, &org).await?;
                query.push_str(&format!("&org_id={}", urlencoding::encode(&org)));
            }
            query.push_str(&format!("&limit={limit}"));
            if let Some(after) = after {
                query.push_str(&format!("&after={}", urlencoding::encode(&after)));
            }
            if let Some(search) = search {
                query.push_str(&format!("&search={}", urlencoding::encode(&search)));
            }
            let result: Value = api.get(&query).await?;
            print_candidates(auth.output, &result)
        }
        PoolCommands::Health {
            pool,
            method,
            path,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let mut endpoint = format!(
                "/service-pools/{}/health?method={}",
                urlencoding::encode(&pool),
                urlencoding::encode(&method)
            );
            if let Some(path) = path {
                endpoint.push_str(&format!("&path={}", urlencoding::encode(&path)));
            }
            let result: Value = api.get(&endpoint).await?;
            print_candidates(auth.output, &result)
        }
        PoolCommands::ResetHealth {
            pool,
            service,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let pool = resolve_pool(&mut api, &pool, org.as_deref()).await?;
            let result: Value = api
                .post(
                    &format!("/service-pools/{}/health/reset", urlencoding::encode(&pool)),
                    &serde_json::json!({"user_service_id":service}),
                )
                .await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => eprintln!("Cooldown reset."),
            }
            Ok(())
        }
    }
}

fn read_json(file: &std::path::Path) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(file)?)?)
}

fn failover_patch(settings: &PoolFailoverArgs) -> Result<Value> {
    if settings.retry_on.contains(&PoolRetryCauseArg::None) && settings.retry_on.len() != 1 {
        bail!("--retry-on none cannot be combined with another retry cause");
    }
    let retry_on = if settings.retry_on.is_empty() {
        None
    } else {
        let mut causes = Vec::new();
        for cause in &settings.retry_on {
            for canonical in cause.causes() {
                if !causes.contains(canonical) {
                    causes.push(*canonical);
                }
            }
        }
        Some(causes)
    };
    let mut patch = serde_json::json!({
        "retry_on":retry_on,
        "max_attempts":settings.max_attempts,
        "per_attempt_timeout_ms":settings.per_attempt_timeout_ms,
        "overall_deadline_ms":settings.overall_deadline_ms,
        "max_replay_body_bytes":settings.max_replay_body_bytes,
        "retry_ambiguous_dispatch":settings.retry_ambiguous_dispatch,
    });
    patch.as_object_mut().unwrap().retain(|_, v| !v.is_null());
    let mut cooldown = serde_json::json!({
        "base_ms":settings.cooldown_base_ms,
        "max_ms":settings.cooldown_max_ms,
        "failures_to_open":settings.cooldown_failures_to_open,
        "honor_retry_after":settings.honor_retry_after,
    });
    let fields = cooldown.as_object_mut().unwrap();
    fields.retain(|_, v| !v.is_null());
    if !fields.is_empty() {
        patch["cooldown"] = cooldown;
    }
    Ok(patch)
}

fn merge_failover(saved: Option<&Value>, patch: Value) -> Result<Value> {
    let mut merged = match saved {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(Value::Object(policy)) => policy.clone(),
        _ => bail!("Saved failover policy must be an object or null; reload the pool"),
    };
    for (field, value) in patch.as_object().expect("patch is an object") {
        if field == "cooldown" {
            let mut cooldown = match merged.remove("cooldown") {
                None | Some(Value::Null) => serde_json::Map::new(),
                Some(Value::Object(cooldown)) => cooldown,
                _ => bail!("Saved cooldown policy must be an object; reload the pool"),
            };
            cooldown.extend(
                value
                    .as_object()
                    .expect("cooldown patch is an object")
                    .clone(),
            );
            merged.insert(field.clone(), Value::Object(cooldown));
        } else {
            merged.insert(field.clone(), value.clone());
        }
    }
    Ok(Value::Object(merged))
}

async fn pool_snapshot(api: &mut ApiClient, pool: &str) -> Result<(String, Value)> {
    let current: Value = api
        .get(&format!("/service-pools/{}", urlencoding::encode(pool)))
        .await?;
    let id = current["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!("Pool snapshot has no ID; reload the pool"))?;
    // Use the same concrete pool even if its slug is concurrently reused.
    Ok((
        format!("/service-pools/{}", urlencoding::encode(id)),
        current,
    ))
}

fn snapshot_revision(current: &Value) -> Result<Value> {
    match current.get("config_revision") {
        None => Ok(0.into()),
        Some(value) if value.as_u64().is_some() => Ok(value.clone()),
        _ => bail!("Pool snapshot has an invalid revision; reload the pool"),
    }
}

async fn resolve_pool(api: &mut ApiClient, pool: &str, org: Option<&str>) -> Result<String> {
    let Some(org) = org else {
        return Ok(pool.to_owned());
    };
    let owner = resolve_org_id(api, org).await?;
    let response: Value = api.get(&service_pools_path(Some(&owner))).await?;
    let matches: Vec<_> = response["pools"]
        .as_array()
        .ok_or_else(|| anyhow!("Invalid pool list"))?
        .iter()
        .filter(|row| row["id"].as_str() == Some(pool) || row["slug"].as_str() == Some(pool))
        .collect();
    if matches.len() != 1 {
        bail!("Pool '{pool}' is not uniquely available in this organization");
    }
    Ok(matches[0]["id"]
        .as_str()
        .ok_or_else(|| anyhow!("Pool has no ID"))?
        .to_owned())
}

fn print_candidates(output: OutputFormat, result: &Value) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }
    if let (Some(method), Some(path)) = (result["method"].as_str(), result["path"].as_str()) {
        eprintln!("Operation: {method} {path}");
    }
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header([
        "Service",
        "Binding",
        "Protocol",
        "Eligible",
        "Reason",
        "Declaration",
        "Cooldown",
        "Failures",
    ]);
    for row in result["candidates"].as_array().into_iter().flatten() {
        table.add_row([
            row["slug"].as_str().unwrap_or("-"),
            row["credential_binding"].as_str().unwrap_or("-"),
            row["protocol"].as_str().unwrap_or("-"),
            if row["eligible"] == true { "yes" } else { "no" },
            row["reason"].as_str().unwrap_or("-"),
            if row["requires_compatibility_declaration"] == true {
                "required"
            } else {
                "-"
            },
            row["cooldown_until"].as_str().unwrap_or("-"),
            &row["consecutive_failures"].to_string(),
        ]);
    }
    eprintln!("{table}");
    if result["has_more"] == true {
        eprintln!(
            "More candidates available. Continue with --after {}",
            result["next_cursor"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

fn service_pools_path(org_id: Option<&str>) -> String {
    match org_id {
        Some(org_id) => format!("/service-pools?org_id={}", urlencoding::encode(org_id)),
        None => "/service-pools".to_string(),
    }
}

fn insert_opt_str(body: &mut Value, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        body[key] = Value::String(value.to_string());
    }
}

fn print_pool_created(output: OutputFormat, pool: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(pool)?),
        OutputFormat::Table => {
            eprintln!(
                "Service pool '{}' created.",
                pool.get("slug").and_then(Value::as_str).unwrap_or("-")
            );
            print_pool_table_summary(pool);
        }
    }
    Ok(())
}

fn print_pool(output: OutputFormat, pool: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(pool)?),
        OutputFormat::Table => print_pool_detail(pool),
    }
    Ok(())
}

fn print_pool_list(output: OutputFormat, pools: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(pools)?),
        OutputFormat::Table => {
            let items = pools
                .get("pools")
                .and_then(Value::as_array)
                .or_else(|| pools.as_array());
            if let Some(items) = items {
                if items.is_empty() {
                    eprintln!("No service pools.");
                    return Ok(());
                }

                let mut table = Table::new();
                table.load_preset(UTF8_FULL_CONDENSED);
                table.set_header(["ID", "Slug", "Name", "Strategy", "Members", "Active"]);
                for pool in items {
                    let id = pool
                        .get("id")
                        .or_else(|| pool.get("_id"))
                        .and_then(Value::as_str)
                        .unwrap_or("-");
                    table.add_row([
                        crate::commands::short_id(id).to_string(),
                        pool.get("slug")
                            .and_then(Value::as_str)
                            .unwrap_or("-")
                            .to_string(),
                        pool.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("-")
                            .to_string(),
                        pool.get("strategy")
                            .and_then(Value::as_str)
                            .unwrap_or("-")
                            .to_string(),
                        member_count(pool).to_string(),
                        yes_no(
                            pool.get("is_active")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        ),
                    ]);
                }
                eprintln!("{table}");
            }
        }
    }
    Ok(())
}

fn print_pool_detail(pool: &Value) {
    eprintln!(
        "ID:        {}",
        pool.get("id")
            .or_else(|| pool.get("_id"))
            .and_then(Value::as_str)
            .unwrap_or("-")
    );
    eprintln!(
        "Slug:      {}",
        pool.get("slug").and_then(Value::as_str).unwrap_or("-")
    );
    eprintln!(
        "Name:      {}",
        pool.get("name").and_then(Value::as_str).unwrap_or("-")
    );
    if let Some(description) = pool.get("description").and_then(Value::as_str) {
        eprintln!("Description: {description}");
    }
    eprintln!(
        "Strategy:  {}",
        pool.get("strategy").and_then(Value::as_str).unwrap_or("-")
    );
    eprintln!(
        "Active:    {}",
        yes_no(
            pool.get("is_active")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        )
    );
    eprintln!(
        "Contract:  {}",
        pool["member_contract"].as_str().unwrap_or("same_api")
    );
    eprintln!(
        "Tier balance: {}",
        pool["tier_balance"].as_str().unwrap_or("round_robin")
    );
    eprintln!(
        "Failover: {}",
        pool.get("failover")
            .filter(|value| !value.is_null())
            .map(Value::to_string)
            .unwrap_or_else(|| "strategy defaults".into())
    );
    eprintln!("Members:   {}", member_count(pool));
    if let (Some(method), Some(path)) = (
        pool.pointer("/health/method").and_then(Value::as_str),
        pool.pointer("/health/path").and_then(Value::as_str),
    ) {
        eprintln!("Health operation: {method} {path}");
    }

    if let Some(members) = pool.get("members").and_then(Value::as_array)
        && !members.is_empty()
    {
        eprintln!();
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_header([
            "Service ID",
            "Priority",
            "Model",
            "Weight",
            "Enabled",
            "Cooldown",
            "Health",
        ]);
        for member in members {
            let health = pool
                .pointer("/health/candidates")
                .and_then(Value::as_array)
                .and_then(|rows| {
                    rows.iter()
                        .find(|row| row["user_service_id"] == member["user_service_id"])
                });
            table.add_row([
                member
                    .get("user_service_id")
                    .and_then(Value::as_str)
                    .map(crate::commands::short_id)
                    .unwrap_or("-")
                    .to_string(),
                member
                    .get("priority")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .to_string(),
                member
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_owned(),
                member
                    .get("weight")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .to_string(),
                yes_no(
                    member
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                ),
                health
                    .and_then(|h| h["cooldown_until"].as_str())
                    .unwrap_or("-")
                    .into(),
                health
                    .and_then(|h| h["reason"].as_str())
                    .unwrap_or("Available")
                    .into(),
            ]);
        }
        eprintln!("{table}");
    }
}

fn print_pool_table_summary(pool: &Value) {
    eprintln!(
        "ID:       {}",
        pool.get("id")
            .or_else(|| pool.get("_id"))
            .and_then(Value::as_str)
            .unwrap_or("-")
    );
    eprintln!(
        "Slug:     {}",
        pool.get("slug").and_then(Value::as_str).unwrap_or("-")
    );
    eprintln!(
        "Strategy: {}",
        pool.get("strategy").and_then(Value::as_str).unwrap_or("-")
    );
    eprintln!("Members:  {}", member_count(pool));
}

fn member_count(pool: &Value) -> usize {
    pool.get("members")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn yes_no(value: bool) -> String {
    if value {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{OutputFormat, PoolCommands, PoolStrategyArg};
    use crate::test_support::mock_auth_with_output;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ORG_UUID: &str = "11111111-1111-4111-8111-111111111111";

    fn pool_response() -> Value {
        serde_json::json!({
            "id": "pool-1",
            "owner_user_id": "owner-1",
            "slug": "llm-pool",
            "name": "LLM Pool",
            "strategy": "weighted",
            "members": [
                { "user_service_id": "svc-1", "weight": 2, "enabled": true }
            ],
            "rr_counter": 0,
            "is_active": true,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        })
    }

    #[test]
    fn strategy_arg_uses_backend_wire_values() {
        assert_eq!(PoolStrategyArg::RoundRobin.as_str(), "round_robin");
        assert_eq!(PoolStrategyArg::Weighted.as_str(), "weighted");
    }

    #[tokio::test]
    async fn create_posts_pool_body_with_resolved_org_and_unresolved_members() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": ORG_UUID,
                "slug": "acme",
                "display_name": "Acme"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/service-pools"))
            .and(body_json(serde_json::json!({
                "slug": "llm-pool",
                "name": "LLM Pool",
                "description": "primary pool",
                "strategy": "weighted",
                "member_contract":"same_api", "tier_balance":"round_robin",
                "members": [{ "user_service_id": "llm-openai" }],
                "org_id": ORG_UUID
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;

        run(PoolCommands::Create {
            slug: "llm-pool".to_string(),
            name: "LLM Pool".to_string(),
            description: Some("primary pool".to_string()),
            strategy: PoolStrategyArg::Weighted,
            members_file: None,
            contract: "same_api".into(),
            tier_balance: "round_robin".into(),
            failover_file: None,
            members: vec!["llm-openai".to_string()],
            org: Some("acme".to_string()),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool create should succeed");
    }

    #[tokio::test]
    async fn list_appends_resolved_org_query_param() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": ORG_UUID,
                "slug": "acme",
                "display_name": "Acme"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/service-pools"))
            .and(query_param("org_id", ORG_UUID))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pools": [pool_response()]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(PoolCommands::List {
            org: Some("acme".to_string()),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool list should succeed");
    }

    #[tokio::test]
    async fn show_fetches_pool_by_id_or_slug() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/service-pools/llm-pool"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/service-pools/llm-pool/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"candidates":[]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        run(PoolCommands::Show {
            pool: "llm-pool".to_string(),
            org: None,
            method: None,
            path: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool show should succeed");
    }

    #[tokio::test]
    async fn add_member_posts_identifiers_to_backend_contract() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/service-pools/llm-pool/members"))
            .and(body_json(serde_json::json!({
                "user_service_id": "llm-openai",
                "weight": 3,
                "enabled": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;

        run(PoolCommands::AddMember {
            pool: "llm-pool".to_string(),
            service: "llm-openai".to_string(),
            weight: Some(3),
            enabled: Some(false),
            priority: None,
            model: None,
            clear_model: false,
            same_api_compatible: None,
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool add-member should succeed");
    }

    #[tokio::test]
    async fn remove_member_deletes_identifier_path() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/service-pools/llm-pool/members/llm-openai"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;

        run(PoolCommands::RemoveMember {
            pool: "llm-pool".to_string(),
            service: "llm-openai".to_string(),
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool remove-member should succeed");
    }

    #[tokio::test]
    async fn set_strategy_puts_strategy_update() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/service-pools/llm-pool"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/service-pools/pool-1"))
            .and(body_json(
                serde_json::json!({ "strategy": "round_robin", "expected_revision": 0 }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;

        run(PoolCommands::SetStrategy {
            pool: "llm-pool".to_string(),
            strategy: PoolStrategyArg::RoundRobin,
            tier_balance: None,
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool set-strategy should succeed");
    }

    #[tokio::test]
    async fn delete_uses_id_without_slug_resolution() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/service-pools/pool-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(PoolCommands::Delete {
            pool_id: "pool-1".to_string(),
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool delete should succeed");
    }
    #[tokio::test]
    async fn pool_update_resolves_org_slug_and_sends_one_atomic_revision_and_null_payload() {
        let server = MockServer::start().await;
        let canonical = "22222222-2222-4222-8222-222222222222";
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id":ORG_UUID,"slug":"acme","display_name":"Acme"}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/service-pools"))
            .and(query_param("org_id", ORG_UUID))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"pools":[{"id":canonical,"slug":"shared"}]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/service-pools/{canonical}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id":canonical,"config_revision":12})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut body = serde_json::json!({"member_contract":"ai_chat","description":null,"failover":null,"members":[{"user_service_id":"member","model":"native-model","priority":2}]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&body).unwrap()).unwrap();
        body["expected_revision"] = 12.into();
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/service-pools/{canonical}")))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;
        run(PoolCommands::Update {
            pool: "shared".into(),
            file: file.path().to_owned(),
            org: Some("acme".into()),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn pool_candidates_omit_defaults_and_preserve_explicit_operation_and_strategy() {
        for explicit in [false, true] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/service-pools/ai/candidates"))
                .and(move |req: &wiremock::Request| {
                    let query: std::collections::HashMap<_, _> =
                        req.url.query_pairs().into_owned().collect();
                    if explicit {
                        query.get("path").is_some_and(|v| v == "/items")
                            && query
                                .get("member_contract")
                                .is_some_and(|v| v == "same_api")
                            && query.get("strategy").is_some_and(|v| v == "round_robin")
                    } else {
                        !query.contains_key("path")
                            && !query.contains_key("member_contract")
                            && !query.contains_key("strategy")
                    }
                })
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"candidates":[],"has_more":false})),
                )
                .expect(1)
                .mount(&server)
                .await;
            run(PoolCommands::Candidates {
                pool: Some("ai".into()),
                contract: explicit.then(|| "same_api".into()),
                strategy: explicit.then_some(PoolStrategyArg::RoundRobin),
                method: "POST".into(),
                path: explicit.then(|| "/items".into()),
                org: None,
                after: None,
                search: None,
                limit: 100,
                auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn pool_failover_defaults_clear_override_with_revision_and_show_reads_operation_health() {
        let server = MockServer::start().await;
        let mut current = pool_response();
        current["config_revision"] = 8.into();
        Mock::given(method("GET"))
            .and(path("/api/v1/service-pools/ai"))
            .respond_with(ResponseTemplate::new(200).set_body_json(current))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/service-pools/pool-1"))
            .and(body_json(
                serde_json::json!({"failover":null,"expected_revision":8}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(pool_response()))
            .expect(1)
            .mount(&server)
            .await;
        run(PoolCommands::SetFailover {
            pool: "ai".into(),
            file: None,
            disable: false,
            defaults: true,
            settings: Default::default(),
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .unwrap();
        Mock::given(method("GET")).and(path("/api/v1/service-pools/ai/health")).and(query_param("method","GET")).and(query_param("path","/tasks/item")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"candidates":[{"user_service_id":"svc-1","cooldown_until":"2026-10-01T00:00:00Z","reason":"cooldown"}]}))).expect(1).mount(&server).await;
        run(PoolCommands::Show {
            pool: "ai".into(),
            org: None,
            method: Some("GET".into()),
            path: Some("/tasks/item".into()),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
#[path = "pool_failover_tests.rs"]
mod failover_tests;
