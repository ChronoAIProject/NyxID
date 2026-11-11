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
    let mut ready = row;
    ready["availability"] = json!("request_dependent");
    let shown = render(&ready);
    assert!(shown.contains("request_dependent"));
    assert!(shown.contains("0.000000000001"));
}

#[test]
fn price_verification_preserves_execution_readiness_without_assistant_gate() {
    with_env(async {
        for (ready, reason) in [
            (false, "execution_fleet_not_ready"),
            (true, ""),
            (false, "execution_not_implemented"),
        ] {
            let server = MockServer::start().await;
            let readiness = json!({
                "public_id": "nyxid/test",
                "execution_ready": ready,
                "blocking_reason": reason,
                "required_readiness": [
                    "inference:platform-model-registry-v1",
                    "inference:platform-models-v1",
                    "billing:model-prices-v1",
                ],
            });
            let mut model = readiness.clone();
            model["billing_service_id"] = json!(ID);
            model["routes"] = json!([{"project": "private-project"}]);
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/admin/platform-models/{ID}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(model))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/admin/billing/services/{ID}")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"effective": {"model_prices": []}})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let (value, _) = billing(
                ModelArgs {
                    model: ID.into(),
                    auth: mock_auth(server.uri()),
                },
                false,
            )
            .await
            .unwrap();
            assert_eq!(value["model"], readiness);
            let shown = render(&value);
            assert_eq!(serde_json::from_str::<Value>(&shown).unwrap(), value);
            assert!(!shown.contains("assistant:platform-models-v1"));
            assert!(!shown.contains("private-project"));
            server.verify().await;
        }
    });
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
            command: PriceCommands::Create(contracts::PriceCreateArgs {
                contract: None,
                attest: None,
                args: SetArgs {
                    model: ID.into(),
                    revision: 8,
                    file: file.path().into(),
                    auth: mock_auth(server.uri()),
                },
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

// Check the typed parse result, not merely that clap accepted the spelling.
fn parsed_fields(command: &Commands) -> (&'static str, Value, &AuthArgs) {
    let model = |args: &ModelArgs| json!({"model":args.model});
    let page = |args: &PageArgs| json!({"limit":args.limit,"after":args.after});
    let set =
        |args: &SetArgs| json!({"model":args.model,"revision":args.revision,"file":args.file});
    match command {
        Commands::Models {
            command: ModelCommands::List(args),
        } => ("list", page(args), &args.auth),
        Commands::Models {
            command: ModelCommands::Show { model, auth },
        } => ("show", json!({"model":model}), auth),
        Commands::Admin {
            command: crate::cli::AdminCommands::Models { command },
        } => match command {
            AdminModelCommands::Contracts {
                command: contracts::ContractCommands::List { auth },
            } => ("contracts list", json!({}), auth),
            AdminModelCommands::Credentials { command } => match command {
                credentials::CredentialCommands::List { auth } => {
                    ("credentials list", json!({}), auth)
                }
                credentials::CredentialCommands::Add {
                    kind, name, auth, ..
                } => ("credentials add", json!({"kind":kind,"name":name}), auth),
                credentials::CredentialCommands::Rotate { id, auth, .. } => {
                    ("credentials rotate", json!({"id":id}), auth)
                }
                credentials::CredentialCommands::Disable { id, auth } => {
                    ("credentials disable", json!({"id":id}), auth)
                }
            },
            AdminModelCommands::Offering { command } => match command {
                credentials::OfferingCommands::Create { name, auth } => {
                    ("offering create", json!({"name":name}), auth)
                }
            },
            AdminModelCommands::List(args) => ("admin list", page(args), &args.auth),
            AdminModelCommands::Show(args) => ("admin show", model(args), &args.auth),
            AdminModelCommands::Create { file, auth } => {
                ("admin create", json!({"file":file}), auth)
            }
            AdminModelCommands::Update(args) => ("admin update", set(args), &args.auth),
            AdminModelCommands::Retire {
                model,
                revision,
                auth,
            } => (
                "admin retire",
                json!({"model":model,"revision":revision}),
                auth,
            ),
            AdminModelCommands::Routes { command } => match command {
                RoutesCommands::Show(args) => ("routes show", model(args), &args.auth),
                RoutesCommands::Set(args) => ("routes set", set(args), &args.auth),
            },
            AdminModelCommands::Access { command } => match command {
                AccessCommands::Show(args) => ("access show", model(args), &args.auth),
                AccessCommands::Set(args) => ("access set", set(args), &args.auth),
                AccessCommands::Explain { model, user, auth } => {
                    ("access explain", json!({"model":model,"user":user}), auth)
                }
            },
            AdminModelCommands::Prices { command } => match command {
                PriceCommands::List(args) => ("prices list", model(args), &args.auth),
                PriceCommands::Create(args) => ("prices create", set(&args.args), &args.args.auth),
                PriceCommands::Publish(args) => ("prices publish", model(args), &args.auth),
                PriceCommands::Resync(args) => ("prices resync", model(args), &args.auth),
                PriceCommands::Verify(args) => ("prices verify", model(args), &args.auth),
                PriceCommands::Retire {
                    model,
                    tariff,
                    revision,
                    auth,
                } => (
                    "prices retire",
                    json!({"model":model,"tariff":tariff,"revision":revision}),
                    auth,
                ),
            },
        },
        _ => panic!("expected a model command"),
    }
}

#[test]
fn every_model_subcommand_parses_its_arguments_and_flattened_auth() {
    let _lock = env_lock().lock().unwrap();
    let contract = "model contract.json";
    let target = json!({"model":ID});
    let set = json!({"model":ID,"revision":7,"file":contract});
    let cases = [
        (
            vec!["admin", "models", "contracts", "list"],
            "contracts list",
            json!({}),
        ),
        (
            vec!["admin", "models", "credentials", "list"],
            "credentials list",
            json!({}),
        ),
        (
            vec![
                "admin",
                "models",
                "credentials",
                "add",
                "--kind",
                "gcp-service-account",
                "--name",
                "Vertex",
                "--file",
                "key.json",
            ],
            "credentials add",
            json!({"kind":"gcp-service-account","name":"Vertex"}),
        ),
        (
            vec![
                "admin",
                "models",
                "credentials",
                "add",
                "--kind",
                "openai",
                "--name",
                "OpenAI",
                "--key-env",
                "MODEL_TEST_KEY",
            ],
            "credentials add",
            json!({"kind":"openai","name":"OpenAI"}),
        ),
        (
            vec![
                "admin",
                "models",
                "credentials",
                "add",
                "--kind",
                "anthropic",
                "--name",
                "Anthropic",
                "--key-env",
                "MODEL_TEST_KEY",
            ],
            "credentials add",
            json!({"kind":"anthropic","name":"Anthropic"}),
        ),
        (
            vec![
                "admin",
                "models",
                "credentials",
                "rotate",
                ID,
                "--file",
                "key.json",
            ],
            "credentials rotate",
            json!({"id":ID}),
        ),
        (
            vec![
                "admin",
                "models",
                "credentials",
                "rotate",
                ID,
                "--key-env",
                "MODEL_TEST_KEY",
            ],
            "credentials rotate",
            json!({"id":ID}),
        ),
        (
            vec!["admin", "models", "credentials", "disable", ID],
            "credentials disable",
            json!({"id":ID}),
        ),
        (
            vec![
                "admin",
                "models",
                "offering",
                "create",
                "--name",
                "Billing identity",
            ],
            "offering create",
            json!({"name":"Billing identity"}),
        ),
        (
            vec!["models", "list", "--limit", "100", "--after", "nyxid/last"],
            "list",
            json!({"limit":100,"after":"nyxid/last"}),
        ),
        (
            vec!["models", "show", "nyxid/test"],
            "show",
            json!({"model":"nyxid/test"}),
        ),
        (
            vec![
                "admin",
                "models",
                "list",
                "--limit",
                "1",
                "--after",
                "nyxid/previous",
            ],
            "admin list",
            json!({"limit":1,"after":"nyxid/previous"}),
        ),
        (
            vec!["admin", "models", "show", ID],
            "admin show",
            target.clone(),
        ),
        (
            vec!["admin", "models", "create", "--file", contract],
            "admin create",
            json!({"file":contract}),
        ),
        (
            vec![
                "admin",
                "models",
                "update",
                ID,
                "--revision",
                "7",
                "--file",
                contract,
            ],
            "admin update",
            set.clone(),
        ),
        (
            vec!["admin", "models", "retire", ID, "--revision", "7"],
            "admin retire",
            json!({"model":ID,"revision":7}),
        ),
        (
            vec!["admin", "models", "routes", "show", ID],
            "routes show",
            target.clone(),
        ),
        (
            vec![
                "admin",
                "models",
                "routes",
                "set",
                ID,
                "--revision",
                "7",
                "--file",
                contract,
            ],
            "routes set",
            set.clone(),
        ),
        (
            vec!["admin", "models", "access", "show", ID],
            "access show",
            target.clone(),
        ),
        (
            vec![
                "admin",
                "models",
                "access",
                "set",
                ID,
                "--revision",
                "7",
                "--file",
                contract,
            ],
            "access set",
            set.clone(),
        ),
        (
            vec![
                "admin",
                "models",
                "access",
                "explain",
                ID,
                "--user",
                "00000000-0000-4000-8000-000000000002",
            ],
            "access explain",
            json!({"model":ID,"user":"00000000-0000-4000-8000-000000000002"}),
        ),
        (
            vec!["admin", "models", "prices", "list", ID],
            "prices list",
            target.clone(),
        ),
        (
            vec![
                "admin",
                "models",
                "prices",
                "create",
                ID,
                "--revision",
                "7",
                "--file",
                contract,
            ],
            "prices create",
            set,
        ),
        (
            vec!["admin", "models", "prices", "publish", ID],
            "prices publish",
            target.clone(),
        ),
        (
            vec!["admin", "models", "prices", "resync", ID],
            "prices resync",
            target.clone(),
        ),
        (
            vec!["admin", "models", "prices", "verify", ID],
            "prices verify",
            target,
        ),
        (
            vec![
                "admin",
                "models",
                "prices",
                "retire",
                ID,
                "00000000-0000-4000-8000-000000000003",
                "--revision",
                "7",
            ],
            "prices retire",
            json!({"model":ID,"tariff":"00000000-0000-4000-8000-000000000003","revision":7}),
        ),
    ];
    for (args, name, expected) in cases {
        for output in ["json", "table"] {
            let cli =
                Cli::try_parse_from(std::iter::once("nyxid").chain(args.iter().copied()).chain([
                    "--base-url",
                    "http://127.0.0.1:7777",
                    "--access-token",
                    "explicit-session",
                    "--access-token-env",
                    "MODEL_TEST_TOKEN",
                    "--profile",
                    "model-test",
                    "--output",
                    output,
                ]))
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let (actual_name, fields, auth) = parsed_fields(&cli.command);
            assert_eq!(actual_name, name);
            assert_eq!(fields, expected, "{name}");
            assert_eq!(auth.base_url.as_deref(), Some("http://127.0.0.1:7777"));
            assert_eq!(auth.access_token.as_deref(), Some("explicit-session"));
            assert_eq!(auth.access_token_env, "MODEL_TEST_TOKEN");
            assert_eq!(auth.profile.as_deref(), Some("model-test"));
            assert_eq!(
                matches!(auth.output, crate::cli::OutputFormat::Json),
                output == "json"
            );
        }
    }
}

#[test]
fn model_pagination_defaults_and_required_arguments_are_stable() {
    let _lock = env_lock().lock().unwrap();
    for args in [vec!["models", "list"], vec!["admin", "models", "list"]] {
        let cli = Cli::try_parse_from(std::iter::once("nyxid").chain(args)).unwrap();
        let (_, fields, auth) = parsed_fields(&cli.command);
        assert_eq!(fields, json!({"limit":50,"after":null}));
        assert!(matches!(auth.output, crate::cli::OutputFormat::Table));
    }
    let cases = [
        vec!["models", "show"],
        vec!["admin", "models", "show"],
        vec!["admin", "models", "create"],
        vec!["admin", "models", "update", ID, "--file", "model.json"],
        vec!["admin", "models", "update", ID, "--revision", "7"],
        vec!["admin", "models", "retire", ID],
        vec!["admin", "models", "routes", "show"],
        vec![
            "admin",
            "models",
            "routes",
            "set",
            ID,
            "--file",
            "routes.json",
        ],
        vec!["admin", "models", "access", "show"],
        vec!["admin", "models", "access", "set", ID, "--revision", "7"],
        vec!["admin", "models", "access", "explain", ID],
        vec!["admin", "models", "prices", "list"],
        vec![
            "admin",
            "models",
            "prices",
            "create",
            ID,
            "--file",
            "price.json",
        ],
        vec!["admin", "models", "prices", "publish"],
        vec!["admin", "models", "prices", "resync"],
        vec!["admin", "models", "prices", "verify"],
        vec!["admin", "models", "prices", "retire", ID, ID],
        vec!["admin", "models", "prices", "retire", ID, "--revision", "7"],
    ];
    for args in cases {
        let Err(error) = Cli::try_parse_from(std::iter::once("nyxid").chain(args.clone())) else {
            panic!("accepted incomplete command: {args:?}");
        };
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument,
            "{args:?}"
        );
    }
    for root in [vec!["models"], vec!["admin", "models"]] {
        for limit in ["0", "101", "not-a-number"] {
            let Err(error) = Cli::try_parse_from(
                std::iter::once("nyxid")
                    .chain(root.iter().copied())
                    .chain(["list", "--limit", limit]),
            ) else {
                panic!("accepted invalid limit: {limit}");
            };
            assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
        }
    }
}

#[test]
fn model_command_help_advertises_actual_nested_commands_and_revision_flags() {
    let _lock = env_lock().lock().unwrap();
    for (args, expected) in [
        (vec!["models"], vec!["list", "show"]),
        (
            vec!["admin", "models"],
            vec!["create", "update", "retire", "routes", "access", "prices"],
        ),
        (vec!["admin", "models", "routes"], vec!["show", "set"]),
        (
            vec!["admin", "models", "access"],
            vec!["show", "set", "explain"],
        ),
        (
            vec!["admin", "models", "prices"],
            vec!["list", "create", "publish", "resync", "verify", "retire"],
        ),
        (
            vec!["admin", "models", "update"],
            vec!["--file", "--revision", "--profile", "--output"],
        ),
    ] {
        let Err(help) = Cli::try_parse_from(std::iter::once("nyxid").chain(args).chain(["--help"]))
        else {
            panic!("help must exit without dispatch");
        };
        assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
        let text = help.to_string();
        for required in expected {
            assert!(text.contains(required), "missing {required}: {text}");
        }
    }
}
