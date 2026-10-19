//! Human-only billing configuration, exact price inspection and readiness.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use chrono::{Duration, Utc};
use serde_json::{Value, json};

use super::{billing_support as support, managed_billing};
use crate::api::ApiClient;
use crate::cli::{
    AdminBillingCommands, BillingConfigCommands, BillingLedgerCommands, BillingServiceCommands,
    OutputFormat,
};

pub async fn run(command: AdminBillingCommands) -> Result<()> {
    match command {
        AdminBillingCommands::Config { command } => run_config(command).await,
        AdminBillingCommands::Overview { auth } => {
            let mut api = support::human_api(&auth).await?;
            let rows: Value = api.get("/admin/billing/overview").await?;
            support::print(&rows, auth.output, || {
                rows.as_array()
                    .into_iter()
                    .flatten()
                    .map(effective_table)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        AdminBillingCommands::Rates {
            service,
            include_retired,
            auth,
        } => {
            let mut api = support::human_api(&auth).await?;
            let codes = match service {
                Some(reference) => Some(metric_codes(
                    &support::service(&mut api, &reference).await?["billing"],
                )),
                None => None,
            };
            let mut rows: Vec<Value> = api.get("/admin/billing/rates").await?;
            rows.retain(|row| {
                (include_retired || row["retired_at"].is_null())
                    && codes.as_ref().is_none_or(|codes| {
                        row["lago_metric_code"]
                            .as_str()
                            .is_some_and(|code| codes.contains(code))
                    })
            });
            let result = json!(rows);
            support::print(&result, auth.output, || {
                support::table(
                    &[
                        "Metric code",
                        "Model",
                        "Credits / unit",
                        "Synced at",
                        "Stale",
                        "Retired",
                    ],
                    rows.iter().map(|row| {
                        [
                            "lago_metric_code",
                            "model",
                            "credits_per_unit",
                            "synced_at",
                            "stale",
                            "retired_at",
                        ]
                        .iter()
                        .map(|key| support::text(&row[*key]))
                        .collect()
                    }),
                )
            })
        }
        AdminBillingCommands::Service { command } => run_service(command).await,
        AdminBillingCommands::Ledger { command } => run_ledger(command).await,
    }
}

fn safe_config(mut value: Value) -> Value {
    if let Some(rows) = value["environment"].as_array_mut() {
        for row in rows {
            let secret = row.get("secret_set").is_some_and(|v| !v.is_null())
                || row["key"]
                    .as_str()
                    .is_some_and(|key| key.ends_with("_KEY") || key.ends_with("_SECRET"));
            if secret {
                row["value"] = Value::Null;
            }
        }
    }
    value
}

async fn run_config(command: BillingConfigCommands) -> Result<()> {
    let (value, auth) = match command {
        BillingConfigCommands::Show { auth } => {
            let mut api = support::human_api(&auth).await?;
            (api.get::<Value>("/admin/billing/config").await?, auth)
        }
        BillingConfigCommands::Set {
            default_overdraft_cap_credits,
            auth,
        } => {
            support::require_human(&auth)?;
            let amount = if default_overdraft_cap_credits == "default" {
                Value::Null
            } else {
                let raw = &default_overdraft_cap_credits;
                let amount = raw
                    .parse::<u64>()
                    .ok()
                    .filter(|amount| *amount <= 1_000_000_000_000)
                    .filter(|_| !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()))
                    .context(
                        "Overdraft cap must be whole credits in 0..=1000000000000, or default",
                    )?;
                json!(amount)
            };
            let mut api = support::human_api(&auth).await?;
            (
                api.put::<Value, _>(
                    "/admin/billing/config",
                    &json!({"default_overdraft_cap_credits":amount}),
                )
                .await?,
                auth,
            )
        }
    };
    let value = safe_config(value);
    support::print(&value, auth.output, || support::fields(&value))
}

pub(super) fn configured_rows(billing: &Value) -> Vec<(String, &Value)> {
    let mut rows = vec![];
    for name in ["platform_pricing", "byok_pricing", "platform_key_pricing"] {
        if let Some(lane) = billing.get(name).filter(|value| value.is_object()) {
            rows.push((name.to_owned(), lane));
            for field in ["components", "operations"] {
                for (index, row) in lane[field].as_array().into_iter().flatten().enumerate() {
                    rows.push((format!("{name}.{field}[{index}]"), row));
                }
            }
        }
    }
    rows
}

fn metric_codes(billing: &Value) -> BTreeSet<String> {
    configured_rows(billing)
        .iter()
        .filter_map(|(_, row)| row["lago_metric_code"].as_str())
        .chain(billing["lago_resale_metric_code"].as_str())
        .map(str::to_owned)
        .collect()
}

fn effective_table(row: &Value) -> String {
    let mut output = format!(
        "Service: {} ({})\n",
        support::text(&row["slug"]),
        support::text(&row["id"])
    );
    output.push_str(&support::table(
        &["Class / lane", "Platform charged", "Reason"],
        row["effective"]["classes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|class| {
                vec![
                    format!(
                        "{} / {}",
                        support::text(&class["credential_classes"]),
                        support::text(&class["lane"])
                    ),
                    support::text(&class["platform_charged"]),
                    support::text(&class["reason"]),
                ]
            }),
    ));
    let mut prices = vec![];
    for class in row["effective"]["classes"].as_array().into_iter().flatten() {
        for kind in ["prices", "pending"] {
            for price in class[kind].as_array().into_iter().flatten() {
                prices.push(vec![
                    support::text(&class["lane"]),
                    kind.into(),
                    support::text(&price["metric"]),
                    support::text(&price["credits_per_unit"]),
                    support::text(&price["sync_status"]),
                    support::text(&price["sync_error"]),
                    support::text(&price["cached_rate"]["credits_per_unit"]),
                    support::text(&price["cached_rate"]["stale"]),
                    support::text(&price["cached_rate"]["retired"]),
                ]);
            }
        }
    }
    output.push('\n');
    output.push_str(&support::table(
        &[
            "Lane",
            "State",
            "Metric",
            "Credits / unit",
            "Sync",
            "Sync error",
            "Cached rate",
            "Stale",
            "Retired",
        ],
        prices,
    ));
    output
}

fn configured_pricing_table(configuration: &Value) -> String {
    let mut output = String::from("Managed contract:\n");
    let contract = &configuration["managed_execution"];
    output.push_str(&support::table(
        &["Field", "Value"],
        [
            "model",
            "max_input_tokens",
            "max_output_tokens",
            "max_request_bytes",
            "max_response_bytes",
            "tariff_version",
            "tariff_source",
            "tariff_verified_at",
        ]
        .into_iter()
        .map(|key| {
            let value = if key == "tariff_verified_at" {
                support::bson_date(&contract[key])
                    .map(support::rfc3339)
                    .unwrap_or_else(|| support::text(&contract[key]))
            } else {
                support::text(&contract[key])
            };
            vec![key.into(), value]
        }),
    ));
    output.push_str("\nConfigured price lanes:\n");
    output.push_str(&support::table(
        &[
            "Lane / row",
            "Metric",
            "Operation",
            "Credits / unit",
            "Sync",
            "Sync error",
            "Lago code",
        ],
        configured_rows(configuration)
            .into_iter()
            .map(|(name, row)| {
                let mut cells = vec![name];
                cells.extend(
                    [
                        "metric",
                        "operation",
                        "credits_per_unit",
                        "sync_status",
                        "sync_error",
                        "lago_metric_code",
                    ]
                    .into_iter()
                    .map(|key| support::text(&row[key])),
                );
                cells
            }),
    ));
    output
}

async fn run_service(command: BillingServiceCommands) -> Result<()> {
    match command {
        BillingServiceCommands::Show { service, auth } => {
            let mut api = support::human_api(&auth).await?;
            let raw = support::service(&mut api, &service).await?;
            let effective: Value = api
                .get(&format!(
                    "/admin/billing/services/{}",
                    support::text(&raw["id"])
                ))
                .await?;
            let value = json!({
                "service": effective,
                "billing": raw["billing"],
                "credential_configured": raw["credential_configured"]
            });
            support::print(&value, auth.output, || {
                format!(
                    "{}\nCredential configured: {}\n{}",
                    effective_table(&value["service"]),
                    support::text(&value["credential_configured"]),
                    configured_pricing_table(&value["billing"])
                )
            })
        }
        BillingServiceCommands::Update {
            service,
            file,
            dry_run,
            auth,
        } => {
            support::require_human(&auth)?;
            let raw = std::fs::read(&file).context("Failed to read billing JSON file")?;
            let document: Value =
                serde_json::from_slice(&raw).context("Invalid billing JSON file")?;
            let patch = billing_patch(document)?;
            let mut api = support::human_api(&auth).await?;
            update(&mut api, &service, patch, dry_run, auth.output).await
        }
        BillingServiceCommands::SetManaged {
            service,
            managed,
            probe_response,
            dry_run,
            auth,
        } => {
            support::require_human(&auth)?;
            let (body, ceiling) = managed_billing::body(&managed, Utc::now())?;
            if let Some(path) = probe_response {
                let raw = std::fs::read(path).context("Failed to read probe response")?;
                let response =
                    serde_json::from_slice(&raw).context("Invalid probe response JSON")?;
                managed_billing::validate_probe(&response, &managed)?;
            }
            eprintln!("Conservative full reservation ceiling: {ceiling} credits");
            let mut api = support::human_api(&auth).await?;
            update(
                &mut api,
                &service,
                body["billing"].clone(),
                dry_run,
                auth.output,
            )
            .await
        }
        BillingServiceCommands::Resync { service, auth } => {
            let mut api = support::human_api(&auth).await?;
            let raw = support::service(&mut api, &service).await?;
            let value: Value = api
                .post(
                    &format!(
                        "/admin/billing/services/{}/resync",
                        support::text(&raw["id"])
                    ),
                    &json!({}),
                )
                .await?;
            support::print(&value, auth.output, || effective_table(&value["service"]))?;
            let ready = value["service"]["effective"]["classes"]
                .as_array()
                .is_some_and(|classes| {
                    classes.iter().all(|class| {
                        class["pending"].as_array().is_some_and(Vec::is_empty)
                            && class["prices"].as_array().is_some_and(|prices| {
                                prices.iter().all(|price| {
                                    price["sync_status"].is_null()
                                        || price["sync_status"] == "synced"
                                })
                            })
                    })
                });
            support::exit_if_not_ready(ready)
        }
        BillingServiceCommands::Verify {
            service,
            managed,
            auth,
        } => {
            let mut api = support::human_api(&auth).await?;
            let raw = support::service(&mut api, &service).await?;
            let config: Value = api.get("/admin/billing/config").await?;
            let effective: Value = api
                .get(&format!(
                    "/admin/billing/services/{}",
                    support::text(&raw["id"])
                ))
                .await?;
            let rates: Vec<Value> = api.get("/admin/billing/rates").await?;
            let endpoints = if managed {
                api.get(&format!(
                    "/services/{}/endpoints",
                    support::text(&raw["id"])
                ))
                .await?
            } else {
                Value::Null
            };
            let report = readiness(&config, &raw, &effective, &rates, managed, &endpoints);
            support::print(&report, auth.output, || {
                let mut result = support::table(
                    &["Check", "Result"],
                    report["checks"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(name, pass)| {
                            vec![
                                name.clone(),
                                if pass == true {
                                    "PASS".into()
                                } else {
                                    "FAIL".into()
                                },
                            ]
                        }),
                );
                result.push_str(&format!("\n{}", support::text(&report["limits"])));
                result
            })?;
            support::exit_if_not_ready(report["configuration_ready"] == true)
        }
    }
}

const SERVER_FIELDS: &[&str] = &[
    "lago_metric_code",
    "sync_status",
    "sync_error",
    "platform_pricing_cleanup_metric_code",
    "byok_pricing_cleanup_metric_code",
    "platform_key_pricing_cleanup_metric_code",
    "component_cleanup_metric_codes",
];

pub(super) fn strip_server_fields(value: &mut Value) -> bool {
    let mut stripped = false;
    match value {
        Value::Object(fields) => {
            for field in SERVER_FIELDS {
                stripped |= fields.remove(*field).is_some();
            }
            for value in fields.values_mut() {
                stripped |= strip_server_fields(value);
            }
        }
        Value::Array(rows) => {
            for row in rows {
                stripped |= strip_server_fields(row);
            }
        }
        _ => {}
    }
    stripped
}

fn billing_patch(document: Value) -> Result<Value> {
    let mut patch = if let Some(value) = document.get("billing") {
        if document.as_object().is_none_or(|object| object.len() != 1) {
            bail!("The wrapped file must contain only billing");
        }
        value.clone()
    } else {
        document
    };
    if strip_server_fields(&mut patch) {
        eprintln!("Warning: stripped server-owned billing synchronization fields");
    }
    validate_patch(&patch)?;
    Ok(patch)
}

fn allowed_keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let fields = value
        .as_object()
        .context("Billing settings must be JSON objects")?;
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        bail!("Unknown billing field; use only documented nonsecret billing settings");
    }
    Ok(())
}

fn validate_patch(patch: &Value) -> Result<()> {
    allowed_keys(
        patch,
        &[
            "managed_execution",
            "platform_billable",
            "platform_charge_nyxid_credentials_only",
            "platform_metric",
            "platform_pricing",
            "byok_pricing",
            "platform_key_pricing",
            "resale_billable",
            "resale_metric",
            "lago_resale_metric_code",
        ],
    )?;
    for name in ["platform_pricing", "byok_pricing", "platform_key_pricing"] {
        if let Some(lane) = patch.get(name).filter(|v| !v.is_null()) {
            allowed_keys(
                lane,
                &["metric", "credits_per_unit", "components", "operations"],
            )?;
            validate_price(lane)?;
            for name in ["components", "operations"] {
                if let Some(rows) = lane.get(name).filter(|v| !v.is_null()) {
                    for row in rows
                        .as_array()
                        .context("Billing components and operations must be arrays")?
                    {
                        allowed_keys(row, &["metric", "operation", "credits_per_unit"])?;
                        validate_price(row)?;
                    }
                }
            }
        }
    }
    if let Some(contract) = patch.get("managed_execution").filter(|v| !v.is_null()) {
        allowed_keys(
            contract,
            &[
                "model",
                "max_input_tokens",
                "max_output_tokens",
                "max_request_bytes",
                "max_response_bytes",
                "tariff_version",
                "tariff_source",
                "tariff_verified_at",
            ],
        )?;
    }
    Ok(())
}

fn validate_price(row: &Value) -> Result<()> {
    if let Some(price) = row.get("credits_per_unit") {
        support::price_pico(
            price
                .as_str()
                .context("Prices must be exact decimal strings")?,
        )?;
    }
    Ok(())
}

pub(super) fn merged_billing(current: &Value, patch: &Value) -> Value {
    let mut merged = current.as_object().cloned().unwrap_or_default();
    // BillingUpdate preserves legacy fields only when a lane key is present.
    if patch.get("byok_pricing").is_none() && patch.get("platform_key_pricing").is_none() {
        for (field, default) in [
            ("platform_billable", json!(false)),
            ("platform_metric", Value::Null),
            ("platform_pricing", Value::Null),
            ("resale_billable", json!(false)),
            ("resale_metric", json!("tokens")),
            ("lago_resale_metric_code", Value::Null),
        ] {
            if patch.get(field).is_none() {
                merged.insert(field.into(), default);
            }
        }
    }
    for (field, value) in patch.as_object().into_iter().flatten() {
        let mut value = value.clone();
        if ["byok_pricing", "platform_key_pricing"].contains(&field.as_str())
            && let Some(lane) = value.as_object_mut()
        {
            for extra in ["components", "operations"] {
                if !lane.contains_key(extra)
                    && let Some(previous) = merged.get(field).and_then(|v| v.get(extra))
                {
                    lane.insert(extra.into(), previous.clone());
                }
            }
        }
        merged.insert(field.clone(), value);
    }
    let mut value = Value::Object(merged);
    strip_server_fields(&mut value);
    value
}

pub(super) fn normalize_billing(value: &Value) -> Value {
    let mut value = value.clone();
    strip_server_fields(&mut value);
    fn normalize(value: &mut Value) {
        if value.get("$date").is_some()
            && let Some(date) = support::bson_date(value)
        {
            *value = json!({"$date":support::rfc3339(date)});
            return;
        }
        match value {
            Value::Object(fields) => {
                fields.retain(|_, v| !v.is_null());
                for (key, value) in fields.iter_mut() {
                    if key == "credits_per_unit"
                        && let Some(raw) = value.as_str()
                        && let Ok(pico) = support::decimal_pico(raw)
                    {
                        *value = json!(support::credits(pico));
                    } else {
                        normalize(value);
                    }
                }
                for key in ["components", "operations"] {
                    if fields
                        .get(key)
                        .is_some_and(|v| v.as_array().is_some_and(Vec::is_empty))
                    {
                        fields.remove(key);
                    }
                }
            }
            Value::Array(rows) => {
                for row in rows {
                    normalize(row);
                }
            }
            _ => {}
        }
    }
    normalize(&mut value);
    value
}

fn field_diff(current: &Value, desired: &Value) -> Vec<Value> {
    let before = normalize_billing(current);
    let after = normalize_billing(desired);
    let keys: BTreeSet<_> = before
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k.clone())
        .chain(
            after
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| k.clone()),
        )
        .collect();
    keys.into_iter()
        .filter(|key| before[key] != after[key])
        .map(|key| json!({"field":key,"before":before[&key],"after":after[&key]}))
        .collect()
}

async fn update(
    api: &mut ApiClient,
    reference: &str,
    patch: Value,
    dry_run: bool,
    output: OutputFormat,
) -> Result<()> {
    let raw = support::service(api, reference).await?;
    if dry_run {
        let merged = merged_billing(&raw["billing"], &patch);
        let diff = field_diff(&raw["billing"], &merged);
        let report = json!({"dry_run":true, "body":{"billing":merged}, "diff":diff});
        support::print(&report, output, || support::fields(&report))
    } else {
        let body = json!({"billing":patch});
        let result: Value = api
            .put(
                &format!("/admin/billing/services/{}", support::text(&raw["id"])),
                &body,
            )
            .await?;
        support::print(&result, output, || effective_table(&result))
    }
}

fn readiness(
    config: &Value,
    raw: &Value,
    effective: &Value,
    rates: &[Value],
    managed: bool,
    endpoints: &Value,
) -> Value {
    let billing = &raw["billing"];
    let rows = configured_rows(billing);
    let mut checks = BTreeMap::from([
        (
            "global_billing_enabled".to_string(),
            config["billing_enabled"] == true,
        ),
        ("lago_configured".into(), config["lago_configured"] == true),
        ("service_active".into(), raw["is_active"] == true),
        (
            "credential_configured".into(),
            raw["credential_configured"] == true,
        ),
        ("configured_prices_present".into(), !rows.is_empty()),
        (
            "effective_platform_billing_charged".into(),
            effective["effective"]["classes"]
                .as_array()
                .is_some_and(|classes| {
                    classes.iter().any(|class| {
                        class["lane"] == "platform_key"
                            && class["platform_charged"] == true
                            && class["reason"] == "lane_synced"
                    })
                }),
        ),
    ]);
    if managed {
        let active: Vec<_> = endpoints["endpoints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|endpoint| endpoint["is_active"] == true)
            .collect();
        checks.insert("one_active_operation".into(), active.len() == 1);
        let endpoint = active.first().copied().unwrap_or(&Value::Null);
        checks.insert(
            "managed_operation_route".into(),
            endpoint["method"] == "POST" && endpoint["path"] == "/ai/openai/chat/completions",
        );
        let contract = &billing["managed_execution"];
        let properties = &endpoint["request_body_schema"]["properties"];
        checks.insert(
            "operation_model_pinned".into(),
            contract["model"].is_string()
                && properties["model"]["enum"] == json!([contract["model"]]),
        );
        checks.insert(
            "operation_output_cap_pinned".into(),
            contract["max_output_tokens"].is_u64()
                && properties["max_tokens"]["maximum"] == contract["max_output_tokens"],
        );
        let rules = &raw["proxy_operation_policy"]["rules"];
        checks.insert(
            "proxy_only_managed_route".into(),
            rules.as_array().is_some_and(|rules| {
                rules.len() == 1
                    && rules[0]["method"] == "POST"
                    && rules[0]["path_template"] == "/ai/openai/chat/completions"
            }),
        );
        for field in [
            "platform_billable",
            "platform_charge_nyxid_credentials_only",
        ] {
            checks.insert(field.into(), billing[field] == true);
        }
        checks.insert(
            "resale_disabled".into(),
            billing["resale_billable"] == false,
        );
        checks.insert(
            "dedicated_internal_offering".into(),
            raw["service_category"] == "internal" && raw["requires_user_credential"] == false,
        );
        checks.insert(
            "restricted_audience".into(),
            raw["platform_key"]["audience"] == "restricted",
        );
        checks.insert(
            "managed_contract_present".into(),
            billing["managed_execution"].is_object(),
        );
        let now = Utc::now();
        checks.insert(
            "tariff_verification_fresh".into(),
            support::bson_date(&billing["managed_execution"]["tariff_verified_at"])
                .is_some_and(|date| date <= now && date >= now - Duration::days(30)),
        );
        let metrics: BTreeSet<_> = rows
            .iter()
            .filter_map(|(_, row)| row["metric"].as_str())
            .collect();
        checks.insert(
            "three_required_prices".into(),
            rows.len() == 3
                && billing["platform_key_pricing"]["metric"] == "input_tokens"
                && metrics
                    == BTreeSet::from(["input_tokens", "output_tokens", "cache_read_tokens"]),
        );
    }
    for (name, row) in rows {
        let code = row["lago_metric_code"]
            .as_str()
            .filter(|code| !code.is_empty());
        let cached: Vec<_> = rates
            .iter()
            .filter(|rate| {
                code.is_some()
                    && rate["lago_metric_code"].as_str() == code
                    && rate["model"].is_null()
            })
            .collect();
        let exact = cached.len() == 1
            && row["credits_per_unit"]
                .as_str()
                .and_then(|price| support::price_pico(price).ok())
                .is_some_and(|price| {
                    cached[0]["credits_per_unit"]
                        .as_str()
                        .and_then(|rate| support::price_pico(rate).ok())
                        == Some(price)
                });
        let fresh =
            cached.len() == 1 && cached[0]["stale"] == false && cached[0]["retired_at"].is_null();
        checks.insert(
            format!(
                "{name}.{}_synced_fresh_exact",
                support::text(&row["metric"])
            ),
            row["sync_status"] == "synced" && exact && fresh,
        );
    }
    json!({
        "configuration_ready": checks.values().all(|pass|*pass),
        "checks": checks,
        "limits": concat!(
            "Checks saved configuration and fresh exact prices. Does not prove replica versions, ",
            "payer funding, tariff provenance or upstream behavior. Access may remain disabled during this check."
        )
    })
}

async fn run_ledger(command: BillingLedgerCommands) -> Result<()> {
    let BillingLedgerCommands::Verify {
        mut from_seq,
        to_seq,
        limit,
        all,
        auth,
    } = command;
    support::require_human(&auth)?;
    if from_seq.zip(to_seq).is_some_and(|(from, to)| from > to) {
        bail!("from-seq must not exceed to-seq");
    }
    let mut api = support::human_api(&auth).await?;
    let mut reports = vec![];
    let mut seen = BTreeSet::new();
    loop {
        let query = [
            ("from_seq", from_seq.map(|v| v.to_string())),
            ("to_seq", to_seq.map(|v| v.to_string())),
            ("limit", limit.map(|v| v.to_string())),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| format!("{key}={value}")))
        .collect::<Vec<_>>()
        .join("&");
        let report: Value = api
            .get(&format!("/admin/billing-ledger/verify?{query}"))
            .await?;
        let broken = report["status"] == "broken" || report["anchor_valid"] == false;
        let next = report["next_from_seq"].as_i64();
        reports.push(report);
        if broken || !all || next.is_none() {
            break;
        }
        let next = next.context("Ledger cursor missing")?;
        if from_seq.is_some_and(|from| next <= from) || !seen.insert(next) {
            bail!("Ledger verifier returned a non-advancing cursor");
        }
        from_seq = Some(next);
    }
    let ready = reports
        .iter()
        .all(|row| row["status"] == "ok" && row["anchor_valid"] != false);
    let value = if all {
        json!({"status":if ready {"ok"} else {"broken"},"chunks":reports})
    } else {
        reports.remove(0)
    };
    support::print(&value, auth.output, || support::fields(&value))?;
    support::exit_if_not_ready(ready)
}

#[cfg(test)]
mod tests;
