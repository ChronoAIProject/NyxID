use super::service::catalog_admin::fetch_catalog_service;
use crate::{
    api::ApiClient,
    cli::{CatalogCommands, CatalogEndpointArgs, CatalogEndpointCommands, ToolsCommands},
    output,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub fn endpoint_body(args: &CatalogEndpointArgs, create: bool) -> Result<Value> {
    if create && (args.name.is_none() || args.method.is_none() || args.path.is_none()) {
        bail!("--name, --method and --path are required for endpoint add");
    }
    let mut body = json!({});
    for (key, value) in [
        ("name", &args.name),
        ("method", &args.method),
        ("path", &args.path),
        ("description", &args.description),
        ("data_scope", &args.data_scope),
        ("cost_class", &args.cost_class),
        ("execution", &args.execution),
    ] {
        if let Some(value) = value {
            body[key] = value.clone().into();
        }
    }
    for (key, path) in [
        ("parameters", &args.parameters_file),
        ("request_body_schema", &args.body_schema_file),
    ] {
        if let Some(path) = path {
            body[key] = serde_json::from_slice(
                &std::fs::read(path).with_context(|| format!("Read {}", path.display()))?,
            )
            .context("Invalid JSON file")?;
        }
    }
    Ok(body)
}

async fn service_id(api: &mut ApiClient, reference: &str) -> Result<String> {
    fetch_catalog_service(api, reference).await?["id"]
        .as_str()
        .map(str::to_string)
        .context("Catalog service has no id")
}
async fn endpoint_id(api: &mut ApiClient, service: &str, reference: &str) -> Result<String> {
    let rows: Value = api.get(&format!("/services/{service}/endpoints")).await?;
    rows["endpoints"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["id"] == reference || row["name"] == reference)
        })
        .and_then(|row| row["id"].as_str())
        .map(str::to_string)
        .context("Catalog endpoint not found")
}

pub async fn run_admin(command: CatalogCommands) -> Result<()> {
    match command {
        CatalogCommands::Topics { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.get("/tools/topics").await?;
            output::print_rows(
                &result,
                auth.output,
                None,
                &[("Topic", "slug"), ("Label", "label")],
            )
        }
        CatalogCommands::Discover { service, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let id = service_id(&mut api, &service).await?;
            let result: Value = api
                .post(&format!("/services/{id}/discover-endpoints"), &json!({}))
                .await?;
            output::print_rows(
                &result,
                auth.output,
                Some("endpoints"),
                &[
                    ("Name", "name"),
                    ("Method", "method"),
                    ("Path", "path"),
                    ("Publication", "publication"),
                ],
            )
        }
        CatalogCommands::Publish {
            service,
            operations,
            state,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let id = service_id(&mut api, &service).await?;
            let result: Value = api
                .post(
                    &format!("/services/{id}/publication"),
                    &json!({"state":state,"endpoint_names":operations}),
                )
                .await?;
            output::print_rows(
                &result,
                auth.output,
                Some("endpoints"),
                &[("Name", "name"), ("Publication", "publication")],
            )
        }
        CatalogCommands::Endpoint { command } => match command {
            CatalogEndpointCommands::List {
                service,
                published_only,
                auth,
                ..
            } => {
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let id = service_id(&mut api, &service).await?;
                let result: Value = api.get(&format!("/services/{id}/endpoints")).await?;
                let mut display = result;
                if published_only && let Some(rows) = display["endpoints"].as_array_mut() {
                    rows.retain(|row| row["publication"] == "published");
                }
                output::print_rows(
                    &display,
                    auth.output,
                    Some("endpoints"),
                    &[
                        ("Id", "id"),
                        ("Name", "name"),
                        ("Method", "method"),
                        ("Path", "path"),
                        ("Publication", "publication"),
                    ],
                )
            }
            CatalogEndpointCommands::Add {
                service,
                endpoint,
                auth,
            } => {
                let body = endpoint_body(&endpoint, true)?;
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let id = service_id(&mut api, &service).await?;
                let result: Value = api
                    .post(&format!("/services/{id}/endpoints"), &body)
                    .await?;
                output::print_rows(
                    &result,
                    auth.output,
                    None,
                    &[
                        ("Id", "id"),
                        ("Name", "name"),
                        ("Publication", "publication"),
                    ],
                )
            }
            CatalogEndpointCommands::Update {
                service,
                endpoint_id: reference,
                endpoint,
                auth,
            } => {
                let body = endpoint_body(&endpoint, false)?;
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let id = service_id(&mut api, &service).await?;
                let endpoint = endpoint_id(&mut api, &id, &reference).await?;
                let result: Value = api
                    .put(&format!("/services/{id}/endpoints/{endpoint}"), &body)
                    .await?;
                output::print_rows(&result, auth.output, None, &[("Result", "message")])
            }
            CatalogEndpointCommands::Disable {
                service,
                endpoint_id: reference,
                auth,
            } => {
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let id = service_id(&mut api, &service).await?;
                let endpoint = endpoint_id(&mut api, &id, &reference).await?;
                let result: Value = api
                    .post(
                        &format!("/services/{id}/endpoints/{endpoint}/publication"),
                        &json!({"state":"paused"}),
                    )
                    .await?;
                output::print_rows(
                    &result,
                    auth.output,
                    None,
                    &[("Name", "name"), ("Publication", "publication")],
                )
            }
        },
        _ => bail!("Unsupported catalog administration command"),
    }
}

fn format_price(price: &Value) -> String {
    if price == "free" || price.is_null() {
        return "Free".into();
    }
    format!(
        "{} credits / {}",
        price["credits_per_unit"].as_str().unwrap_or("?"),
        price["metric"].as_str().unwrap_or("request")
    )
}
/// Effective platform price of one operation: its synced price, else the base.
fn operation_price<'a>(price: &'a Value, operation: &str) -> Option<&'a str> {
    let base = price["credits_per_unit"].as_str()?;
    Some(
        price["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|candidate| {
                candidate["operation"] == operation
                    && candidate["sync_status"]
                        .as_str()
                        .is_none_or(|status| status == "synced")
            })
            .and_then(|candidate| candidate["credits_per_unit"].as_str())
            .unwrap_or(base),
    )
}
/// Exact ordering for decimal credit strings with at most 12 fractional digits.
fn picocredits(value: &str) -> u128 {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let fraction = format!("{fraction:0<12}");
    whole.parse::<u128>().unwrap_or(0) * 1_000_000_000_000
        + fraction[..12].parse::<u128>().unwrap_or(0)
}
fn format_tool_price(row: &Value) -> String {
    let price = &row["pricing"]["platform"];
    let operations = row["operations"].as_array().into_iter().flatten();
    let prices: Vec<&str> = if price["operations"]
        .as_array()
        .is_some_and(|operations| !operations.is_empty())
    {
        operations
            .filter_map(|operation| operation_price(price, operation["name"].as_str()?))
            .collect()
    } else {
        Vec::new()
    };
    match (
        prices.iter().min_by_key(|value| picocredits(value)),
        prices.iter().max_by_key(|value| picocredits(value)),
    ) {
        (Some(min), Some(max)) if picocredits(min) != picocredits(max) => {
            format!("From {min} to {max} credits / request")
        }
        (Some(only), Some(_)) => format!("{only} credits / request"),
        _ => format_price(price),
    }
}
fn format_limits(limits: &Value) -> String {
    if limits.is_null() {
        return "none".into();
    }
    format!(
        "{}/s, burst {}",
        limits["rate_limit_per_second"], limits["burst"]
    )
}
fn format_topics(topics: &Value) -> String {
    topics
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}
fn table_offering(row: &Value) -> Value {
    let mut display = row.clone();
    display["pricing"] = format_tool_price(row).into();
    display["limits"] = format_limits(&row["limits"]).into();
    display["topics"] = format_topics(&row["topics"]).into();
    display
}

/// Table rows for `tools show`, with each operation's effective price.
fn operation_rows(row: &Value) -> Value {
    let price = &row["pricing"]["platform"];
    let mut operations = row["operations"].clone();
    for operation in operations.as_array_mut().into_iter().flatten() {
        let label = match operation["name"].as_str() {
            Some(name) if price["metric"] == "requests" => operation_price(price, name)
                .map_or_else(
                    || format_price(price),
                    |value| format!("{value} credits / request"),
                ),
            _ => format_price(price),
        };
        operation["price"] = label.into();
    }
    operations
}

pub async fn run_tools(command: ToolsCommands) -> Result<()> {
    match command {
        ToolsCommands::List { topic, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.get("/tools").await?;
            let mut display = result;
            if let Some(topic) = topic
                && let Some(rows) = display.as_array_mut()
            {
                rows.retain(|row| {
                    row["topics"]
                        .as_array()
                        .is_some_and(|topics| topics.iter().any(|t| t == &topic))
                });
            }
            if !matches!(auth.output, crate::cli::OutputFormat::Json) {
                display = display
                    .as_array()
                    .map(|rows| Value::Array(rows.iter().map(table_offering).collect()))
                    .unwrap_or(display);
            }
            output::print_rows(
                &display,
                auth.output,
                None,
                &[
                    ("Slug", "slug"),
                    ("Name", "name"),
                    ("Supplier", "supplier"),
                    ("Topics", "topics"),
                    ("Pricing", "pricing"),
                    ("Limits", "limits"),
                ],
            )
        }
        ToolsCommands::Show { slug, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.get(&format!("/tools/{slug}")).await?;
            if matches!(auth.output, crate::cli::OutputFormat::Json) {
                return output::print_rows(&result, auth.output, None, &[]);
            }
            output::print_rows(
                &table_offering(&result),
                auth.output,
                None,
                &[
                    ("Slug", "slug"),
                    ("Name", "name"),
                    ("Supplier", "supplier"),
                    ("Pricing", "pricing"),
                    ("Limits", "limits"),
                    ("Topics", "topics"),
                ],
            )?;
            output::print_rows(
                &operation_rows(&result),
                auth.output,
                None,
                &[
                    ("Name", "name"),
                    ("Method", "method"),
                    ("Path", "path"),
                    ("Price", "price"),
                    ("Risk", "risk"),
                    ("Data scope", "data_scope"),
                    ("Cost class", "cost_class"),
                ],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn tool_table_formatting_is_human_readable_without_mutating_json() {
        assert_eq!(format_price(&json!("free")), "Free");
        assert_eq!(
            format_price(&json!({"credits_per_unit":"0.012","metric":"requests"})),
            "0.012 credits / requests"
        );
        let tool = json!({
            "pricing": {"platform": {"metric": "requests", "credits_per_unit": "0.1", "operations": [
                {"operation": "search", "credits_per_unit": "0.25", "sync_status": "synced"},
                {"operation": "lookup", "credits_per_unit": "0.05", "sync_status": "synced"},
                {"operation": "export", "credits_per_unit": "9", "sync_status": "pending"},
            ]}},
            "operations": [{"name": "search"}, {"name": "lookup"}, {"name": "export"}],
        });
        assert_eq!(
            format_tool_price(&tool),
            "From 0.05 to 0.25 credits / request"
        );
        assert_eq!(
            operation_rows(&tool)
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["price"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "0.25 credits / request",
                "0.05 credits / request",
                // A pending price is not charged yet.
                "0.1 credits / request",
            ]
        );
        assert_eq!(
            format_tool_price(
                &json!({"pricing": {"platform": "free"}, "operations": [{"name": "search"}]})
            ),
            "Free"
        );
        assert_eq!(picocredits("1.000000000001"), 1_000_000_000_001);
        assert_eq!(format_limits(&Value::Null), "none");
        assert_eq!(
            format_limits(&json!({"rate_limit_per_second":5,"burst":10})),
            "5/s, burst 10"
        );
        assert_eq!(
            format_topics(&json!(["web-search", "page-fetch"])),
            "web-search, page-fetch"
        );
        let original = json!({"pricing":{"platform":"free"},"limits":null,"topics":["web-search"],"operations":[{"name":"read","method":"GET","path":"/read","risk":"read","data_scope":"public","cost_class":"free"}]});
        let display = table_offering(&original);
        assert_eq!(display["pricing"], "Free");
        assert_eq!(display["limits"], "none");
        assert_eq!(display["topics"], "web-search");
        assert!(original["pricing"].is_object());
        assert_eq!(display["operations"], original["operations"]);
    }
    #[test]
    fn catalog_and_tools_arguments_parse() {
        for args in [
            vec![
                "nyxid",
                "catalog",
                "endpoint",
                "list",
                "tools-x",
                "--published-only",
            ],
            vec![
                "nyxid",
                "catalog",
                "publish",
                "tools-x",
                "--operation",
                "read",
                "--operation",
                "search",
                "--state",
                "paused",
            ],
            vec![
                "nyxid",
                "tools",
                "list",
                "--topic",
                "web-search",
                "--output",
                "json",
            ],
            vec!["nyxid", "tools", "show", "tools-x"],
        ] {
            assert!(crate::cli::Cli::try_parse_from(args).is_ok());
        }
        assert!(
            crate::cli::Cli::try_parse_from(["nyxid", "catalog", "publish", "tools-x"]).is_err()
        );
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "catalog",
                "publish",
                "tools-x",
                "--operation",
                "read",
                "--state",
                "invalid"
            ])
            .is_err()
        );
    }
    #[test]
    fn endpoint_body_keeps_omission_and_requires_create_identity() {
        assert!(endpoint_body(&CatalogEndpointArgs::default(), true).is_err());
        assert_eq!(
            endpoint_body(&CatalogEndpointArgs::default(), false).unwrap(),
            json!({})
        );
        let args = CatalogEndpointArgs {
            name: Some("read".into()),
            method: Some("GET".into()),
            path: Some("/items".into()),
            data_scope: Some("public".into()),
            cost_class: Some("free".into()),
            ..Default::default()
        };
        assert_eq!(
            endpoint_body(&args, true).unwrap(),
            json!({"name":"read","method":"GET","path":"/items","data_scope":"public","cost_class":"free"})
        );
    }
    #[test]
    fn tool_flags_replace_and_clear_topics() {
        let mut body = json!({});
        crate::cli::CatalogServiceArgs {
            offering_kind: Some("tool".into()),
            topics: vec!["web-search".into()],
            supplier: Some("Example".into()),
            service_category: Some("internal".into()),
            ..Default::default()
        }
        .apply_tool_fields(&mut body);
        assert_eq!(
            body,
            json!({"offering_kind":"tool","topics":["web-search"],"supplier":"Example","service_category":"internal"})
        );
        crate::cli::CatalogServiceArgs {
            clear_topics: true,
            clear_supplier: true,
            ..Default::default()
        }
        .apply_tool_fields(&mut body);
        assert_eq!(body["topics"], json!([]));
        assert_eq!(body["supplier"], Value::Null);
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "service",
                "update",
                "tools-x",
                "--catalog-admin",
                "--clear-supplier",
                "--supplier",
                "Vendor"
            ])
            .is_err()
        );
    }
}

#[cfg(test)]
mod twin_arguments {
    use clap::Parser;
    #[test]
    fn twin_add_accepts_flag_slug_and_requires_catalog_admin() {
        let args = [
            "nyxid",
            "service",
            "add",
            "--catalog-admin",
            "--twin-of",
            "api-twitter",
            "--slug",
            "tools-x",
            "--offering-kind",
            "tool",
        ];
        let parsed = crate::cli::Cli::try_parse_from(args).unwrap();
        assert!(
            matches!(parsed.command, crate::cli::Commands::Service {command: crate::cli::ServiceCommands::Add {twin_of: Some(source), custom_slug: Some(slug), ..}} if source == "api-twitter" && slug == "tools-x")
        );
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "service",
                "add",
                "--twin-of",
                "api-twitter"
            ])
            .is_err()
        );
        assert!(
            crate::cli::Cli::try_parse_from(["nyxid", "catalog", "spec", "show", "tools-x"])
                .is_err()
        );
    }
}
