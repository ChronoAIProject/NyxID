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
            output::print_rows(
                &result,
                auth.output,
                None,
                &[
                    ("Slug", "slug"),
                    ("Name", "name"),
                    ("Supplier", "supplier"),
                    ("Pricing", "pricing"),
                    ("Operations", "operations"),
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
            ..Default::default()
        }
        .apply_tool_fields(&mut body);
        assert_eq!(body["topics"], json!([]));
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
