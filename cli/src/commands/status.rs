use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::OutputFormat;

pub async fn run(api: &mut ApiClient, output: OutputFormat) -> Result<()> {
    let user: Value = api.get_value("/users/me").await?;
    let services_resp: Value = api.get_value("/keys").await?;
    let api_keys_resp: Value = api.get_value("/api-keys").await?;
    let nodes_resp: Value = api.get_value("/nodes").await?;

    // Unwrap from response wrappers: { "keys": [...] } or { "nodes": [...] }
    let services = services_resp
        .get("keys")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let api_keys = api_keys_resp
        .get("keys")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let nodes = nodes_resp.get("nodes").cloned().unwrap_or(
        nodes_resp
            .as_array()
            .map(|a| Value::Array(a.clone()))
            .unwrap_or(Value::Array(vec![])),
    );

    match output {
        OutputFormat::Json => {
            let combined = serde_json::json!({
                "user": user,
                "services": services,
                "api_keys": api_keys,
                "nodes": nodes,
            });
            println!("{}", serde_json::to_string_pretty(&combined)?);
        }
        OutputFormat::Table => {
            print_table_output(&user, &services, &api_keys, &nodes, api.base_url_root());
        }
    }

    Ok(())
}

fn print_table_output(user: &Value, services: &Value, api_keys: &Value, nodes: &Value, base: &str) {
    let email = user["email"].as_str().unwrap_or("-");
    let role = user["role"].as_str().unwrap_or("-");

    eprintln!("Account: {email} ({role})");
    eprintln!("Server:  {base}");
    eprintln!();

    // Services
    let svc_list = services.as_array();
    let svc_count = svc_list.map_or(0, |v| v.len());
    eprintln!("AI Services ({svc_count})");

    if svc_count > 0 {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_header(["ID", "Slug", "Endpoint", "Status"]);

        for svc in svc_list.unwrap() {
            let id = svc["id"].as_str().or(svc["_id"].as_str()).unwrap_or("-");
            let slug = svc["slug"]
                .as_str()
                .or(svc["service_slug"].as_str())
                .unwrap_or("-");
            let endpoint = svc["endpoint_url"].as_str().unwrap_or("-");
            let status = svc["status"].as_str().unwrap_or("active");
            table.add_row([id, slug, endpoint, status]);
        }
        eprintln!("{table}");
    } else {
        eprintln!("  (none)");
    }
    eprintln!();

    // API Keys
    let key_list = api_keys.as_array();
    let key_count = key_list.map_or(0, |v| v.len());
    eprintln!("API Keys ({key_count})");

    if key_count > 0 {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_header(["ID", "Name", "Scopes", "Services", "Nodes"]);

        for key in key_list.unwrap() {
            let id = key["id"].as_str().or(key["_id"].as_str()).unwrap_or("-");
            let name = key["name"].as_str().unwrap_or("-");
            let scopes = key["scopes"].as_str().unwrap_or("-");
            let services = if key["allow_all_services"].as_bool().unwrap_or(true) {
                "all".to_string()
            } else {
                key["allowed_services"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|s| s["slug"].as_str().or(s["label"].as_str()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_else(|| "-".to_string())
            };
            let nodes_scope = if key["allow_all_nodes"].as_bool().unwrap_or(true) {
                "all".to_string()
            } else {
                key["allowed_nodes"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|n| n["name"].as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_else(|| "-".to_string())
            };
            table.add_row([id, name, scopes, &services, &nodes_scope]);
        }
        eprintln!("{table}");
    } else {
        eprintln!("  (none)");
    }
    eprintln!();

    // Nodes
    let node_list = nodes.as_array();
    let node_count = node_list.map_or(0, |v| v.len());
    eprintln!("Nodes ({node_count})");

    if node_count > 0 {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_header(["ID", "Name", "Status", "Last Seen"]);

        for node in node_list.unwrap() {
            let id = node["id"].as_str().or(node["_id"].as_str()).unwrap_or("-");
            let name = node["name"].as_str().unwrap_or("-");
            let status = node["status"].as_str().unwrap_or("-");
            let last_seen = node["last_heartbeat_at"].as_str().unwrap_or("-");
            table.add_row([id, name, status, last_seen]);
        }
        eprintln!("{table}");
    } else {
        eprintln!("  (none)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn mount_status_endpoints(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "email": "a@b.com", "role": "admin" })),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"id": "s1", "slug": "openai", "endpoint_url": "https://x", "status": "active"}]
            })))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{"id": "k1", "name": "agent", "scopes": "read write"}]
            })))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/nodes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "nodes": [{"id": "n1", "name": "box", "status": "online", "last_heartbeat_at": "2026"}]
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn status_aggregates_json() {
        let server = MockServer::start().await;
        mount_status_endpoints(&server).await;
        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        run(&mut api, OutputFormat::Json)
            .await
            .expect("status json should succeed");
    }

    #[tokio::test]
    async fn status_table_renders_all_sections() {
        let server = MockServer::start().await;
        mount_status_endpoints(&server).await;
        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        run(&mut api, OutputFormat::Table)
            .await
            .expect("status table should succeed");
    }
}
