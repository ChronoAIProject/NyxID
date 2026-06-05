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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
                        "The VM operator must SSH to the node-agent machine and run `nyxid node credentials pending`, then `nyxid node credentials accept {slug}`."
                    );
                    eprintln!(
                        "`nyxid node credentials` is node-side only; it is not available on the user-side CLI."
                    );
                    eprintln!("Do not send the secret value; it is entered on the VM.");
                }
            }
            Ok(())
        }
        NodeCredentialAdminCommands::List { node, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
    use crate::cli::{NodeCredentialAdminCommands, OutputFormat, PendingCredentialInjectionMethod};
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn format_age_handles_invalid_timestamp() {
        assert_eq!(format_age("not-a-date"), "-");
    }

    #[test]
    fn format_age_reports_seconds_minutes_hours_and_days() {
        let now = chrono::Utc::now();
        assert_eq!(
            format_age(&(now - chrono::Duration::seconds(42)).to_rfc3339()),
            "42s"
        );
        assert_eq!(
            format_age(&(now - chrono::Duration::minutes(17)).to_rfc3339()),
            "17m"
        );
        assert_eq!(
            format_age(&(now - chrono::Duration::hours(5)).to_rfc3339()),
            "5h"
        );
        assert_eq!(
            format_age(&(now - chrono::Duration::days(3)).to_rfc3339()),
            "3d"
        );
    }

    #[test]
    fn format_age_clamps_future_timestamps_to_zero_seconds() {
        let future = chrono::Utc::now() + chrono::Duration::minutes(5);
        assert_eq!(format_age(&future.to_rfc3339()), "0s");
    }

    #[tokio::test]
    async fn push_resolves_node_name_and_posts_pending_credential_metadata() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("GET"))
            .and(path("/api/v1/nodes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "nodes": [
                    {"id": node_id, "name": "edge-node"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!("/api/v1/nodes/{node_id}/credentials/push")))
            .and(body_json(json!({
                "service_slug": "openai",
                "injection_method": "header",
                "field_name": "Authorization",
                "target_url": "https://api.openai.com/v1",
                "label": "OpenAI production"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "pending-1",
                "service_slug": "openai",
                "injection_method": "header",
                "field_name": "Authorization"
            })))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::Push {
            node: "edge-node".to_string(),
            slug: "openai".to_string(),
            injection_method: PendingCredentialInjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.openai.com/v1".to_string()),
            label: Some("OpenAI production".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("push should succeed");
    }

    #[tokio::test]
    async fn list_fetches_pending_credentials_for_resolved_node() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("GET"))
            .and(path("/api/v1/nodes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "nodes": [
                    {"id": node_id, "name": "edge-node"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/nodes/{node_id}/credentials/pending")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "pending_credentials": [
                    {
                        "id": "pending-1",
                        "service_slug": "openai",
                        "injection_method": "header",
                        "field_name": "Authorization",
                        "created_at": "2026-01-01T00:00:00Z",
                        "expires_at": "2026-01-01T01:00:00Z"
                    }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::List {
            node: "edge-node".to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn list_handles_empty_pending_credentials_response() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/nodes/{node_id}/credentials/pending")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "pending_credentials": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::List {
            node: node_id.to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("empty list should succeed");
    }

    #[tokio::test]
    async fn cancel_with_yes_deletes_pending_credential_without_prompt() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("DELETE"))
            .and(path(format!(
                "/api/v1/nodes/{node_id}/credentials/pending/pending-1"
            )))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::Cancel {
            node: node_id.to_string(),
            pending_id: "pending-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("cancel should succeed");
    }
}
