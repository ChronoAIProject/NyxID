use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{NodeCredentialAdminCommands, OutputFormat};

pub async fn run(command: NodeCredentialAdminCommands) -> Result<()> {
    match command {
        NodeCredentialAdminCommands::Push {
            node,
            slug,
            injection_method,
            field_name,
            target_url,
            label,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let node_id = crate::commands::node::resolve_node_id(&mut api, &node).await?;
            let body = serde_json::json!({
                "service_slug": slug,
                "injection_method": injection_method.wire_value(),
                "field_name": field_name,
                "target_url": target_url,
                "label": label,
            });
            let pending: Value = api
                .post(&format!("/nodes/{node_id}/credentials/push"), &body)
                .await?;

            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&pending)?),
                OutputFormat::Table => {
                    let pending_id = pending["id"].as_str().unwrap_or("-");
                    let slug = pending["service_slug"].as_str().unwrap_or("-");
                    let method = pending["injection_method"].as_str().unwrap_or("-");
                    let field = pending["field_name"].as_str().unwrap_or("-");
                    eprintln!("Pending credential created: {pending_id}");
                    eprintln!();
                    eprintln!("Relay this setup metadata to the VM operator:");
                    eprintln!("  slug: {slug}");
                    eprintln!("  injection method: {method}");
                    eprintln!("  field name: {field}");
                    eprintln!();
                    eprintln!(
                        "The VM operator should run `nyxid node credentials pending`, then `nyxid node credentials accept {slug}`."
                    );
                    eprintln!("Do not send the secret value; it is entered on the VM.");
                }
            }
            Ok(())
        }
        NodeCredentialAdminCommands::List { node, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let node_id = crate::commands::node::resolve_node_id(&mut api, &node).await?;
            let response: Value = api
                .get(&format!("/nodes/{node_id}/credentials/pending"))
                .await?;

            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&response)?),
                OutputFormat::Table => {
                    let pending = response
                        .get("pending_credentials")
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if pending.is_empty() {
                        eprintln!("No pending credentials for this node.");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header(["ID", "Slug", "Method", "Field", "Age", "Expires"]);
                    for item in pending {
                        let id = item["id"].as_str().unwrap_or("-");
                        let slug = item["service_slug"].as_str().unwrap_or("-");
                        let method = item["injection_method"].as_str().unwrap_or("-");
                        let field = item["field_name"].as_str().unwrap_or("-");
                        let created_at = item["created_at"].as_str().unwrap_or("-");
                        let expires_at = item["expires_at"].as_str().unwrap_or("-");
                        table.add_row([
                            id.to_string(),
                            slug.to_string(),
                            method.to_string(),
                            field.to_string(),
                            format_age(created_at),
                            expires_at.to_string(),
                        ]);
                    }
                    eprintln!("{table}");
                }
            }
            Ok(())
        }
        NodeCredentialAdminCommands::Cancel {
            node,
            pending_id,
            yes,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let node_id = crate::commands::node::resolve_node_id(&mut api, &node).await?;

            if !yes {
                eprintln!("Cancel pending credential: {pending_id}");
                eprintln!("Node: {node}");
                eprint!("Proceed? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            api.delete_empty(&format!(
                "/nodes/{node_id}/credentials/pending/{pending_id}"
            ))
            .await?;

            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": pending_id,
                        "canceled": true,
                    }))?
                ),
                OutputFormat::Table => eprintln!("Pending credential canceled."),
            }
            Ok(())
        }
    }
}

fn format_age(created_at: &str) -> String {
    let Ok(created) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return "-".to_string();
    };
    let age = chrono::Utc::now().signed_duration_since(created.with_timezone(&chrono::Utc));
    let seconds = age.num_seconds().max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::format_age;

    #[test]
    fn format_age_handles_invalid_timestamp() {
        assert_eq!(format_age("not-a-date"), "-");
    }
}
