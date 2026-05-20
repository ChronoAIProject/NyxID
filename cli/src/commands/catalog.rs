use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{CatalogCommands, OutputFormat};

pub async fn run(command: CatalogCommands) -> Result<()> {
    match command {
        CatalogCommands::List { all, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let path = if all {
                "/catalog?include_all=true"
            } else {
                "/catalog"
            };
            let catalog: Value = api.get(path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&catalog)?);
                }
                OutputFormat::Table => {
                    eprintln!("Available Services");

                    let items = catalog
                        .get("entries")
                        .and_then(|v| v.as_array())
                        .or_else(|| catalog.as_array());
                    if let Some(items) = items {
                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["Slug", "Name", "Type", "Auth", "How to Add"]);

                        for item in items {
                            let slug = item["slug"].as_str().unwrap_or("-");
                            let name = item["name"].as_str().unwrap_or("-");
                            let svc_type = item["service_type"].as_str().unwrap_or("http");
                            let provider_type = item["provider_type"].as_str().unwrap_or("-");
                            let credential_mode =
                                item["credential_mode"].as_str().unwrap_or("admin");
                            let requires_gw =
                                item["requires_gateway_url"].as_bool().unwrap_or(false);

                            let type_label = if svc_type == "ssh" {
                                "SSH".to_string()
                            } else {
                                match provider_type {
                                    "oauth2" => "OAuth".to_string(),
                                    "device_code" => "Device Code".to_string(),
                                    "api_key" => "API Key".to_string(),
                                    _ => "HTTP".to_string(),
                                }
                            };

                            let how_to_add = if svc_type == "ssh" {
                                format!("nyxid service add {} --via-node <NODE>", slug)
                            } else if provider_type == "oauth2" {
                                if credential_mode == "user" || credential_mode == "both" {
                                    format!(
                                        "nyxid service add {} --oauth (needs client_id/secret)",
                                        slug
                                    )
                                } else {
                                    format!("nyxid service add {} --oauth", slug)
                                }
                            } else if provider_type == "device_code" {
                                format!("nyxid service add {} --device-code", slug)
                            } else if requires_gw {
                                format!(
                                    "nyxid service add {} --endpoint-url <URL> --credential-env <VAR>",
                                    slug
                                )
                            } else {
                                format!("nyxid service add {} --credential-env <VAR>", slug)
                            };

                            table.add_row([slug, name, &type_label, provider_type, &how_to_add]);
                        }
                        eprintln!("{table}");
                    }

                    eprintln!();
                    eprintln!("Use `nyxid catalog show <slug>` for details on a specific service.");
                }
            }
            Ok(())
        }
        CatalogCommands::Show { slug, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let item: Value = api.get(&format!("/catalog/{slug}")).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&item)?);
                }
                OutputFormat::Table => {
                    let name = item["name"].as_str().unwrap_or("-");
                    let item_slug = item["slug"].as_str().unwrap_or(&slug);
                    let url = item["base_url"].as_str().unwrap_or("(none)");
                    let auth_method = item["auth_method"].as_str().unwrap_or("bearer");
                    let auth_key = item["auth_key_name"].as_str().unwrap_or("Authorization");
                    let svc_type = item["service_type"].as_str().unwrap_or("http");
                    let provider_type = item["provider_type"].as_str().unwrap_or("-");
                    let credential_mode = item["credential_mode"].as_str().unwrap_or("admin");
                    let requires_gw = item["requires_gateway_url"].as_bool().unwrap_or(false);

                    eprintln!("Service: {name}");
                    eprintln!("Slug:           {item_slug}");
                    eprintln!("Type:           {svc_type}");
                    eprintln!("Default URL:    {url}");
                    eprintln!("Auth Method:    {auth_method}");
                    eprintln!("Auth Key Name:  {auth_key}");
                    eprintln!("Provider Type:  {provider_type}");
                    eprintln!("Credential Mode: {credential_mode}");
                    if requires_gw {
                        eprintln!("Requires URL:   yes (you must provide your instance URL)");
                    }

                    if let Some(desc) = item["description"].as_str()
                        && !desc.is_empty()
                    {
                        eprintln!();
                        eprintln!("Description:");
                        eprintln!("  {desc}");
                    }

                    // Rich metadata
                    let has_metadata = item["homepage_url"].is_string()
                        || item["repository_url"].is_string()
                        || item["issues_url"].is_string()
                        || item["examples_url"].is_string()
                        || item["openapi_spec_url"].is_string()
                        || item["asyncapi_spec_url"].is_string();

                    if has_metadata {
                        eprintln!();
                        eprintln!("Links:");
                        if let Some(v) = item["homepage_url"].as_str() {
                            eprintln!("  Homepage:     {v}");
                        }
                        if let Some(v) = item["repository_url"].as_str() {
                            eprintln!("  Repository:   {v}");
                        }
                        if let Some(v) = item["issues_url"].as_str() {
                            eprintln!("  Issues:       {v}");
                        }
                        if let Some(v) = item["examples_url"].as_str() {
                            eprintln!("  Skills & Examples: {v}");
                        }
                        if let Some(v) = item["openapi_spec_url"].as_str() {
                            eprintln!("  OpenAPI Spec: {v}");
                        }
                        if let Some(v) = item["asyncapi_spec_url"].as_str() {
                            eprintln!("  AsyncAPI:     {v}");
                        }
                    }

                    if let Some(caps) = item["capabilities"].as_object() {
                        let enabled: Vec<&str> = caps
                            .iter()
                            .filter(|(_, v)| v.as_bool() == Some(true))
                            .map(|(k, _)| k.as_str())
                            .collect();
                        if !enabled.is_empty() {
                            eprintln!();
                            eprintln!("Capabilities:");
                            for cap in &enabled {
                                eprintln!("  - {cap}");
                            }
                        }
                    }

                    if let Some(notes) = item["auth_notes"].as_str() {
                        eprintln!();
                        eprintln!("Auth Notes:");
                        eprintln!("  {notes}");
                    }

                    if let Some(lim) = item["known_limitations"].as_str() {
                        eprintln!();
                        eprintln!("Known Limitations:");
                        eprintln!("  {lim}");
                    }

                    if let Some(perms) = item["required_permissions"].as_array()
                        && !perms.is_empty()
                    {
                        eprintln!();
                        eprintln!("Required Permissions:");
                        for p in perms {
                            if let Some(s) = p.as_str() {
                                eprintln!("  - {s}");
                            }
                        }
                    }

                    if let Some(skills) = item["recommended_skills"].as_array()
                        && !skills.is_empty()
                    {
                        eprintln!();
                        eprintln!("Recommended Skills:");
                        for s in skills {
                            if let Some(v) = s.as_str() {
                                eprintln!("  - {v}");
                            }
                        }
                    }

                    if let Some(instructions) = item["api_key_instructions"].as_str() {
                        eprintln!();
                        eprintln!("How to get credentials:");
                        eprintln!("  {instructions}");
                    }
                    if let Some(api_key_url) = item["api_key_url"].as_str() {
                        eprintln!("  Get API key: {api_key_url}");
                    }
                    if let Some(docs) = item["documentation_url"].as_str() {
                        eprintln!("  Docs: {docs}");
                    }

                    eprintln!();
                    eprintln!("How to add:");

                    if svc_type == "ssh" {
                        eprintln!("  nyxid service add {item_slug} --via-node <NODE_NAME>");
                    } else if provider_type == "oauth2" {
                        if credential_mode == "user" || credential_mode == "both" {
                            eprintln!("  # Step 1: Set your OAuth app credentials");
                            eprintln!("  nyxid service credentials {item_slug} \\");
                            eprintln!(
                                "    --client-id-env MY_CLIENT_ID --client-secret-env MY_CLIENT_SECRET"
                            );
                            eprintln!();
                            eprintln!("  # Step 2: Connect via OAuth (opens browser)");
                            eprintln!("  nyxid service add {item_slug} --oauth");
                        } else {
                            eprintln!("  nyxid service add {item_slug} --oauth");
                        }
                    } else if provider_type == "device_code" {
                        eprintln!("  nyxid service add {item_slug} --device-code");
                    } else if requires_gw {
                        eprintln!("  nyxid service add {item_slug} \\");
                        eprintln!("    --endpoint-url <YOUR_INSTANCE_URL> \\");
                        eprintln!("    --credential-env <TOKEN_VAR>");
                    } else {
                        eprintln!("  nyxid service add {item_slug} --credential-env <API_KEY_VAR>");
                    }

                    eprintln!();
                    eprintln!("  # Or route through a node (credentials stay local):");
                    eprintln!("  nyxid service add {item_slug} --via-node <NODE_NAME>");

                    if item["openapi_spec_url"].is_string() {
                        eprintln!();
                        eprintln!("  # Discover available API endpoints:");
                        eprintln!("  nyxid catalog endpoints {item_slug}");
                    }
                }
            }
            Ok(())
        }
        CatalogCommands::Endpoints { slug, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.get(&format!("/catalog/{slug}/endpoints")).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let spec_url = result["openapi_spec_url"].as_str();
                    let endpoints = result["endpoints"].as_array();

                    if let Some(url) = spec_url {
                        eprintln!("OpenAPI Spec: {url}");
                        eprintln!();
                    }

                    if let Some(eps) = endpoints {
                        if eps.is_empty() {
                            eprintln!(
                                "No endpoints found. This service may not have an OpenAPI spec."
                            );
                        } else {
                            eprintln!("{} endpoints found:", eps.len());
                            eprintln!();

                            let mut table = Table::new();
                            table.load_preset(UTF8_FULL_CONDENSED);
                            table.set_header(["Method", "Path", "Name", "Description"]);

                            for ep in eps {
                                let method = ep["method"].as_str().unwrap_or("-");
                                let path = ep["path"].as_str().unwrap_or("-");
                                let name = ep["name"].as_str().unwrap_or("-");
                                let desc = ep["description"]
                                    .as_str()
                                    .map(|d| truncate_line(d, 60))
                                    .unwrap_or_else(|| "-".to_string());

                                table.add_row([method, path, name, &desc]);
                            }
                            eprintln!("{table}");
                        }
                    } else {
                        eprintln!("No endpoints data in response.");
                    }
                }
            }
            Ok(())
        }
    }
}

fn truncate_line(s: &str, max: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.chars().count() > max {
        let truncated: String = first_line.chars().take(max - 3).collect();
        format!("{truncated}...")
    } else {
        first_line.to_string()
    }
}
