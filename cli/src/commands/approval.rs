use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ApprovalCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: ApprovalCommands) -> Result<()> {
    match command {
        ApprovalCommands::List { org, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
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
                            let short_id = if id.len() > 8 { &id[..8] } else { id };
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
            let mut api = ApiClient::from_auth(&auth)?;
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
                    eprintln!("Description: {description}");
                    eprintln!("Created:     {created}");
                }
            }
            Ok(())
        }

        ApprovalCommands::Approve { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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
                        table.set_header(["ID", "Service", "Requester", "Granted", "Expires"]);

                        for grant in items {
                            let gid = grant["id"]
                                .as_str()
                                .or(grant["_id"].as_str())
                                .unwrap_or("-");
                            let short_id = if gid.len() > 8 { &gid[..8] } else { gid };
                            let service = grant["service_name"].as_str().unwrap_or("-");
                            let requester = grant["requester_label"]
                                .as_str()
                                .or(grant["requester_type"].as_str())
                                .unwrap_or("-");
                            let granted = grant["granted_at"].as_str().unwrap_or("-");
                            let expires = grant["expires_at"].as_str().unwrap_or("never");
                            table.add_row([short_id, service, requester, granted, expires]);
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

            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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

            let mut api = ApiClient::from_auth(&auth)?;
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
            let mut api = ApiClient::from_auth(&auth)?;
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
                        table.set_header(["Service ID", "Service", "Approval Required", "Mode"]);

                        for cfg in items {
                            let cid = cfg["service_id"].as_str().unwrap_or("-");
                            let short_id = if cid.len() > 8 { &cid[..8] } else { cid };
                            let service = cfg["service_name"].as_str().unwrap_or("-");
                            let require = cfg["approval_required"]
                                .as_bool()
                                .map(|b| b.to_string())
                                .unwrap_or_else(|| "-".to_string());
                            let mode = cfg["approval_mode"].as_str().unwrap_or("per_request");
                            table.add_row([short_id, service, &require, mode]);
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
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
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

            if body.is_empty() {
                eprintln!("No updates specified. Use --require-approval and/or --approval-mode.");
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
