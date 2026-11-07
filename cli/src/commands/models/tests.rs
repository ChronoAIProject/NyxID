use super::*;
use crate::{
    cli::{Cli, Commands},
    test_support::{env_lock, mock_auth},
};
use clap::Parser;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path, query_param},
};
// CLI auth/telemetry can inspect environment-backed profile state. Acquire the
// suite's shared environment lock before driving async work on this test thread.
fn with_env(run: impl std::future::Future<Output = ()>) {
    let _lock = env_lock().lock().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run);
}

const ID: &str = "00000000-0000-4000-8000-000000000001";
#[test]
fn parses_model_commands_and_requires_revision() {
    for args in [
        vec!["models", "list", "--after", "nyxid/one"],
        vec!["models", "show", "nyxid/test"],
        vec![
            "admin",
            "models",
            "routes",
            "set",
            ID,
            "--revision",
            "2",
            "--file",
            "routes.json",
        ],
        vec!["admin", "models", "access", "explain", ID, "--user", ID],
        vec!["admin", "models", "prices", "publish", ID],
        vec!["admin", "models", "prices", "verify", ID],
        vec!["admin", "models", "retire", ID, "--revision", "2"],
    ] {
        let mut all = vec!["nyxid"];
        all.extend(args);
        assert!(Cli::try_parse_from(all).is_ok());
    }
    assert!(
        Cli::try_parse_from([
            "nyxid",
            "admin",
            "models",
            "update",
            ID,
            "--file",
            "model.json"
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["nyxid", "models", "list", "--limit", "101"]).is_err());
    assert!(matches!(
        Cli::try_parse_from(["nyxid", "models", "list"])
            .unwrap()
            .command,
        Commands::Models { .. }
    ));
}
#[test]
fn public_output_strips_private_fields_and_preserves_exact_unknown_units() {
    let row = public_model(
        &json!({"id":"nyxid/test","display_name":"Test","availability":"unavailable","routes":[{"project":"secret"}],"capabilities":{"project":"secret","tools":true},"retail_prices":[{"available":true,"variant":{},"active_tariff_id":"secret","components":[{"metric":"future_unit","selector":{"kind":"future_unit","project":"secret"},"credits_per_unit":"0.000000000001","explicit_free":false,"lago_metric_code":"secret"}]}]}),
    );
    let text = serde_json::to_string(&row).unwrap();
    assert!(!text.contains("secret"));
    let shown = render(&row);
    assert!(shown.contains("future_unit"));
    assert!(shown.contains("0.000000000001"));
    assert!(!shown.contains("Free"));
    assert!(render(&public_model(&json!({"id":"nyxid/test","retail_prices":[{"available":false,"variant":{},"components":[]}]}))).contains("Retail prices unavailable"));
}
#[test]
fn show_paginates_public_endpoint_only_and_does_not_fetch_private_routes() {
    with_env(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/llm/models"))
            .and(query_param("limit", "100"))
            .and(|request: &wiremock::Request| {
                !request.url.query_pairs().any(|(key, _)| key == "after")
            })
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"models":[],"next_after":"nyxid/first"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/llm/models"))
            .and(query_param("after", "nyxid/first"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"models":[{"id":"nyxid/test","routes":"hidden"}],"next_after":null}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let mut api = ApiClient::from_auth_checked(&mock_auth(server.uri()))
            .await
            .unwrap();
        let model = find_public(&mut api, "nyxid/test").await.unwrap();
        assert!(model.get("routes").is_none());
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    });
}
#[test]
fn route_update_is_revision_fenced_and_conflict_is_not_retried() {
    with_env(async {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/admin/platform-models/{ID}/routes")))
            .and(body_json(json!({"expected_revision":7,"routes":[]})))
            .respond_with(
                ResponseTemplate::new(409)
                    .set_body_json(json!({"error":"Model changed","code":409})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), r#"{"routes":[]}"#).unwrap();
        let error = run_admin(AdminModelCommands::Routes {
            command: RoutesCommands::Set(SetArgs {
                model: ID.into(),
                revision: 7,
                file: file.path().into(),
                auth: mock_auth(server.uri()),
            }),
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("409") || error.to_string().contains("Model changed"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    });
}
#[test]
fn access_explain_and_price_publish_use_existing_admin_apis() {
    with_env(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/admin/platform-models/{ID}/access")))
            .and(query_param("user_id", ID))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"policy_reason":"explicit_deny","executable":false})),
            )
            .expect(1)
            .mount(&server)
            .await;
        run_admin(AdminModelCommands::Access {
            command: AccessCommands::Explain {
                model: ID.into(),
                user: ID.into(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/admin/platform-models/{ID}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"billing_service_id":ID,"execution_ready":false})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/admin/billing/services/{ID}/resync")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"pending"})))
            .expect(1)
            .mount(&server)
            .await;
        run_admin(AdminModelCommands::Prices {
            command: PriceCommands::Publish(ModelArgs {
                model: ID.into(),
                auth: mock_auth(server.uri()),
            }),
        })
        .await
        .unwrap();
    });
}
#[test]
fn admin_models_reject_agent_key_before_request() {
    with_env(async {
        let server = MockServer::start().await;
        let mut auth = mock_auth(server.uri());
        auth.access_token = Some("nyx_agent_key".into());
        assert!(
            run_admin(AdminModelCommands::Show(ModelArgs {
                model: ID.into(),
                auth
            }))
            .await
            .unwrap_err()
            .to_string()
            .contains("human account session")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    });
}
#[test]
fn manifest_preserves_decimal_strings_and_rejects_revision_override() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        r#"{"components":[{"credits_per_unit":"0.000000000001"}]}"#,
    )
    .unwrap();
    let value = manifest(file.path(), Some(("revision", 8))).unwrap();
    assert_eq!(value["components"][0]["credits_per_unit"], "0.000000000001");
    assert_eq!(value["revision"], 8);
    std::fs::write(file.path(), r#"{"revision":2}"#).unwrap();
    assert!(manifest(file.path(), Some(("revision", 8))).is_err());
}

#[test]
fn tariff_create_and_retire_keep_exact_prices_and_model_revision() {
    with_env(async {
        let server = MockServer::start().await;
        let manifest_body = json!({"lane":"platform_key","variant":{},"source":"certified-source","contract":{},"bounds":{},"components":[{"key":"input","metric":"input_tokens","selector":{"kind":"input"},"credits_per_unit":"0.000000000001","explicit_free":false,"max_quantity":100}]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&manifest_body).unwrap()).unwrap();
        let mut expected = manifest_body;
        expected["revision"] = json!(8);
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/admin/platform-models/{ID}/tariffs")))
            .and(body_json(expected))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id":ID,"status":"pending"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        run_admin(AdminModelCommands::Prices {
            command: PriceCommands::Create(SetArgs {
                model: ID.into(),
                revision: 8,
                file: file.path().into(),
                auth: mock_auth(server.uri()),
            }),
        })
        .await
        .unwrap();
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/admin/platform-models/{ID}/tariffs/{ID}/retire"
            )))
            .and(body_json(json!({"revision":9})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        run_admin(AdminModelCommands::Prices {
            command: PriceCommands::Retire {
                model: ID.into(),
                tariff: ID.into(),
                revision: 9,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap();
    });
}

#[test]
fn inaccessible_public_model_has_generic_error_without_admin_fallback() {
    with_env(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/llm/models"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"models":[],"next_after":null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut api = ApiClient::from_auth_checked(&mock_auth(server.uri()))
            .await
            .unwrap();
        assert_eq!(
            find_public(&mut api, "nyxid/private")
                .await
                .unwrap_err()
                .to_string(),
            "Model unavailable"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    });
}

#[test]
fn admin_list_shows_uuid_and_revision_needed_for_subsequent_cas_edits() {
    let output = render(
        &json!({"models":[{"id":ID,"public_id":"nyxid/test","display_name":"Test","lifecycle":"draft","revision":7}],"next_after":null}),
    );
    assert!(output.contains(ID));
    assert!(output.contains("nyxid/test"));
    assert!(output.contains("Revision"));
    assert!(output.contains('7'));
}
