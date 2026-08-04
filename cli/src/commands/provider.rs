use anyhow::Result;
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ProviderCommands};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: ProviderCommands) -> Result<()> {
    match command {
        ProviderCommands::Disconnect {
            provider_id,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let path = disconnect_path(&provider_id, org.as_deref());
            let result: Value = api.delete(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let message = result["message"]
                        .as_str()
                        .unwrap_or("Provider disconnected and credentials removed");
                    let status = result["status"].as_str().unwrap_or("disconnected");
                    eprintln!("{message}");
                    eprintln!("Status: {status}");
                    if let Some(org_id) = org.as_deref() {
                        eprintln!("Org:    {org_id}");
                    }
                }
            }

            Ok(())
        }
    }
}

fn disconnect_path(provider_id: &str, target_org_id: Option<&str>) -> String {
    let mut path = format!("/providers/{provider_id}/disconnect");
    if let Some(org_id) = target_org_id {
        path.push_str("?target_org_id=");
        path.push_str(&urlencoding::encode(org_id));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::disconnect_path;

    #[test]
    fn disconnect_path_omits_target_org_when_absent() {
        assert_eq!(
            disconnect_path("provider-1", None),
            "/providers/provider-1/disconnect"
        );
    }

    #[test]
    fn disconnect_path_appends_encoded_target_org() {
        assert_eq!(
            disconnect_path("provider-1", Some("org 1&2")),
            "/providers/provider-1/disconnect?target_org_id=org%201%262"
        );
    }
}

#[cfg(test)]
mod command_tests {
    use super::run;
    use crate::cli::ProviderCommands;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    // A UUID `org` resolves locally (no `/orgs` roundtrip), so it is safe to
    // hardcode and is returned verbatim as the `target_org_id` query param.
    const ORG_UUID: &str = "11111111-1111-1111-1111-111111111111";

    #[tokio::test]
    async fn disconnect_personal_issues_delete_without_org_query() {
        let server = MockServer::start().await;
        // Match only when `target_org_id` is absent: the personal path must
        // not leak an org query param.
        Mock::given(method("DELETE"))
            .and(path("/api/v1/providers/prov-1/disconnect"))
            .and(|req: &Request| !req.url.query_pairs().any(|(k, _)| k == "target_org_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "disconnected",
                "message": "Provider disconnected and credentials removed"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ProviderCommands::Disconnect {
            provider_id: "prov-1".to_string(),
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("personal disconnect should succeed");
    }

    #[tokio::test]
    async fn disconnect_with_org_uuid_sets_target_org_query() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/providers/prov-1/disconnect"))
            .and(query_param("target_org_id", ORG_UUID))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "status": "disconnected" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ProviderCommands::Disconnect {
            provider_id: "prov-1".to_string(),
            org: Some(ORG_UUID.to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("org disconnect should succeed");
    }

    #[tokio::test]
    async fn disconnect_surfaces_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/providers/prov-1/disconnect"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let result = run(ProviderCommands::Disconnect {
            provider_id: "prov-1".to_string(),
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "4xx should surface as an error");
    }
}
