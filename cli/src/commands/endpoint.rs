use std::io::Write;

use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{EndpointCommands, OutputFormat};

pub async fn run(command: EndpointCommands) -> Result<()> {
    match command {
        EndpointCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let endpoints: Value = api.get("/endpoints").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&endpoints)?);
                }
                OutputFormat::Table => {
                    let items = endpoints
                        .get("endpoints")
                        .and_then(|v| v.as_array())
                        .or_else(|| endpoints.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No endpoints.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["ID", "Label", "URL"]);

                        for ep in items {
                            let id = ep["id"].as_str().or(ep["_id"].as_str()).unwrap_or("-");
                            let short_id = crate::commands::short_id(id);
                            let label = ep["label"].as_str().or(ep["name"].as_str()).unwrap_or("-");
                            let url = ep["url"]
                                .as_str()
                                .or(ep["base_url"].as_str())
                                .unwrap_or("-");
                            table.add_row([short_id, label, url]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        EndpointCommands::Update { id, url, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let body = serde_json::json!({ "url": url });
            let result: Value = api.put(&format!("/endpoints/{id}"), &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Endpoint {id} updated.");
                }
            }
            Ok(())
        }

        EndpointCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Delete endpoint {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/endpoints/{id}")).await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Endpoint deleted."),
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::EndpointCommands;
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_fetches_endpoints_json_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/endpoints"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "endpoints": [
                    {"id": "ep-abcdef12", "label": "Primary", "url": "https://api.example.com"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(EndpointCommands::List {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn list_empty_table_is_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/endpoints"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "endpoints": [] })),
            )
            .mount(&server)
            .await;

        run(EndpointCommands::List {
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("empty list should succeed");
    }

    #[tokio::test]
    async fn update_sends_url_in_body() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/endpoints/ep-1"))
            .and(body_json(
                serde_json::json!({ "url": "https://new.example.com" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ep-1",
                "url": "https://new.example.com"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(EndpointCommands::Update {
            id: "ep-1".to_string(),
            url: "https://new.example.com".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn delete_with_yes_issues_delete_request() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/endpoints/ep-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(EndpointCommands::Delete {
            id: "ep-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("delete should succeed");
    }

    #[tokio::test]
    async fn list_surfaces_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/endpoints"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let result = run(EndpointCommands::List {
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "5xx should surface as an error");
    }
}
