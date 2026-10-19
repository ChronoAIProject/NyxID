use super::*;
use crate::cli::{AuthArgs, Cli, Commands, ManagedBillingArgs};
use crate::test_support::{env_lock, mock_auth};
use clap::Parser;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path, query_param},
};

const ID: &str = "00000000-0000-4000-8000-000000000001";
fn fixture(name: &str) -> Value {
    let raw = match name {
        "config" => include_str!("../../../tests/fixtures/billing/config.json"),
        "rates" => include_str!("../../../tests/fixtures/billing/rates.json"),
        "overview" => include_str!("../../../tests/fixtures/billing/overview.json"),
        "service" => include_str!("../../../tests/fixtures/billing/service.json"),
        "raw" => include_str!("../../../tests/fixtures/billing/catalog-service.json"),
        "priced" => include_str!("../../../tests/fixtures/billing/catalog-priced-service.json"),
        "endpoints" => include_str!("../../../tests/fixtures/billing/managed-endpoints.json"),
        "managed" => include_str!("../../../tests/fixtures/billing/managed-service.json"),
        _ => panic!("unknown fixture"),
    };
    serde_json::from_str(raw).unwrap()
}
async fn mount(server: &MockServer, verb: &str, url: &str, value: Value) {
    Mock::given(method(verb))
        .and(path(format!("/api/v1{url}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .expect(1)
        .mount(server)
        .await;
}
fn managed_args() -> ManagedBillingArgs {
    ManagedBillingArgs {
        model: "org/model:1".into(),
        max_input_tokens: 8192,
        max_output_tokens: 1024,
        max_request_bytes: 65536,
        max_response_bytes: 262144,
        tariff_version: "verified-test-tariff".into(),
        tariff_source: "https://telnyx.com/pricing".into(),
        tariff_verified_at: "2026-10-19T00:00:00Z".into(),
        input_price: "0.00000025".into(),
        output_price: "0.000001".into(),
        cache_read_price: "0.0000001".into(),
    }
}
fn probe() -> Value {
    json!({
        "id": "chatcmpl-test",
        "model": "org/model:1",
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 5,
            "total_tokens": 105,
            "prompt_tokens_details": {
                "cached_tokens": 10
            }
        },
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "OK"
                }
            }
        ]
    })
}
fn valid_state() -> (Value, Value, Vec<Value>) {
    let mut raw = fixture("managed");
    raw["billing"]["managed_execution"]["tariff_verified_at"] =
        json!({"$date":support::rfc3339(Utc::now()-Duration::hours(1))});
    let mut prices = vec![];
    let mut rates = vec![];
    for (_, row) in configured_rows(&raw["billing"]) {
        prices.push(json!({
            "metric": row["metric"],
            "credits_per_unit": row["credits_per_unit"],
            "lago_metric_code": row["lago_metric_code"],
            "sync_status": "synced",
            "cached_rate": {
                "credits_per_unit": row["credits_per_unit"],
                "stale": false,
                "retired": false
            }
        }));
        rates.push(json!({
            "lago_metric_code": row["lago_metric_code"],
            "model": null,
            "credits_per_unit": row["credits_per_unit"],
            "stale": false,
            "retired_at": null
        }));
    }
    let effective = json!({
        "id": ID,
        "slug": "telnyx-platform",
        "effective": {
            "classes": [
                {
                    "lane": "platform_key",
                    "credential_classes": ["nyxid_managed_master"],
                    "platform_charged": true,
                    "reason": "lane_synced",
                    "prices": prices,
                    "pending": []
                }
            ]
        }
    });
    (raw, effective, rates)
}

#[tokio::test]
async fn config_show_reads_sanitized_fixture_and_redacts_secret_values() {
    let server = MockServer::start().await;
    let mut config = fixture("config");
    config["environment"][0] = json!({
        "key": "LAGO_API_KEY",
        "secret_set": true,
        "value": "TOP_SECRET_VALUE"
    });
    let safe = safe_config(config.clone());
    assert!(
        !serde_json::to_string(&safe)
            .unwrap()
            .contains("TOP_SECRET_VALUE")
    );
    assert!(!support::fields(&safe).contains("TOP_SECRET_VALUE"));
    assert_eq!(safe["environment"][0]["secret_set"], true);
    mount(&server, "GET", "/admin/billing/config", config).await;
    run(AdminBillingCommands::Config {
        command: BillingConfigCommands::Show {
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn config_set_sends_exact_number_or_null_once() {
    for (raw, amount) in [
        ("0", json!(0)),
        ("1000000000000", json!(1_000_000_000_000u64)),
        ("default", Value::Null),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/admin/billing/config"))
            .and(body_json(json!({"default_overdraft_cap_credits":amount})))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("config")))
            .expect(1)
            .mount(&server)
            .await;
        run(AdminBillingCommands::Config {
            command: BillingConfigCommands::Set {
                default_overdraft_cap_credits: raw.into(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn config_set_rejects_out_of_range_locally() {
    let server = MockServer::start().await;
    for raw in ["-1", "1000000000001", "1.1", "1e3", "+1", ""] {
        assert!(
            run(AdminBillingCommands::Config {
                command: BillingConfigCommands::Set {
                    default_overdraft_cap_credits: raw.into(),
                    auth: mock_auth(server.uri())
                }
            })
            .await
            .is_err()
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn overview_renders_every_price_class_and_sync_field() {
    let server = MockServer::start().await;
    let overview = fixture("overview");
    let output = overview
        .as_array()
        .unwrap()
        .iter()
        .map(effective_table)
        .collect::<Vec<_>>()
        .join("\n");
    for expected in [
        "Platform charged",
        "Reason",
        "Credits / unit",
        "Sync error",
        "Cached rate",
        "Stale",
        "Retired",
        "lane_synced",
    ] {
        assert!(output.contains(expected), "{expected}");
    }
    mount(&server, "GET", "/admin/billing/overview", overview).await;
    run(AdminBillingCommands::Overview {
        auth: mock_auth(server.uri()),
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn service_show_joins_effective_and_raw_and_formats_bson_date() {
    let server = MockServer::start().await;
    let (raw, effective, _) = valid_state();
    mount(&server, "GET", &format!("/services/{ID}"), raw.clone()).await;
    mount(
        &server,
        "GET",
        &format!("/admin/billing/services/{ID}"),
        effective,
    )
    .await;
    run(AdminBillingCommands::Service {
        command: BillingServiceCommands::Show {
            service: ID.into(),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    let output = support::fields(&fixture("managed")["billing"]);
    assert!(output.contains("2026-10-19T00:00:00Z"));
    assert!(!output.contains("$numberLong"));
}

#[tokio::test]
async fn rates_filters_exact_configured_codes_and_retired_rows() {
    for include_retired in [false, true] {
        let server = MockServer::start().await;
        let (raw, _, mut rates) = valid_state();
        rates[1]["retired_at"] = json!("2026-10-18T00:00:00Z");
        rates.push(json!({"lago_metric_code":"unrelated","retired_at":null}));
        mount(&server, "GET", &format!("/services/{ID}"), raw.clone()).await;
        mount(&server, "GET", "/admin/billing/rates", json!(rates)).await;
        run(AdminBillingCommands::Rates {
            service: Some(ID.into()),
            include_retired,
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
        let codes = metric_codes(&raw["billing"]);
        assert_eq!(codes.len(), 3);
        assert!(codes.contains("platform_svc_telnyx-platform_pk_cache_read_tokens"));
    }
}

#[tokio::test]
async fn slug_fallback_lists_catalog_then_uses_uuid() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/services/telnyx-platform"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error":"not_found"})))
        .expect(1)
        .mount(&server)
        .await;
    mount(
        &server,
        "GET",
        "/services",
        json!({"services":[fixture("managed")]}),
    )
    .await;
    mount(
        &server,
        "GET",
        &format!("/admin/billing/services/{ID}"),
        fixture("service"),
    )
    .await;
    run(AdminBillingCommands::Service {
        command: BillingServiceCommands::Show {
            service: "telnyx-platform".into(),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[1].url.query(), Some("include_inactive=true"));
}

#[tokio::test]
async fn ambiguous_and_missing_slugs_fail_with_catalog_hint() {
    for rows in [vec![], vec![fixture("managed"), fixture("managed")]] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services/telnyx-platform"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error":"not_found"})))
            .mount(&server)
            .await;
        mount(&server, "GET", "/services", json!({"services":rows})).await;
        let error = run(AdminBillingCommands::Service {
            command: BillingServiceCommands::Show {
                service: "telnyx-platform".into(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("catalog") || error.contains("Admin"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn billing_updates_put_only_validated_patch_and_preview_merged_dry_run() {
    for wrapped in [false, true] {
        for dry_run in [false, true] {
            let server = MockServer::start().await;
            let current = fixture("managed");
            mount(&server, "GET", &format!("/services/{ID}"), current.clone()).await;
            let patch = json!({
                "platform_key_pricing": {
                    "metric": "input_tokens",
                    "credits_per_unit": "0.0000003",
                    "sync_status": "failed",
                    "lago_metric_code": "ignored"
                }
            });
            let document = if wrapped {
                json!({"billing":patch})
            } else {
                patch.clone()
            };
            let cleaned = billing_patch(document.clone()).unwrap();
            let merged = merged_billing(&current["billing"], &cleaned);
            assert_eq!(
                merged["platform_key_pricing"]["components"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert!(!merged.to_string().contains("sync_status"));
            if !dry_run {
                Mock::given(method("PUT"))
                    .and(path(format!("/api/v1/admin/billing/services/{ID}")))
                    .and(body_json(json!({"billing":cleaned})))
                    .respond_with(ResponseTemplate::new(200).set_body_json(fixture("service")))
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), document.to_string()).unwrap();
            run(AdminBillingCommands::Service {
                command: BillingServiceCommands::Update {
                    service: ID.into(),
                    file: file.path().into(),
                    dry_run,
                    auth: mock_auth(server.uri()),
                },
            })
            .await
            .unwrap();
            assert_eq!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| r.method == "PUT")
                    .count(),
                usize::from(!dry_run)
            );
        }
    }
}

#[tokio::test]
async fn billing_update_invalid_json_prices_and_secret_fields_fail_before_requests() {
    let server = MockServer::start().await;
    for raw in [
        "{",
        r#"{"billing":{},"credential":"SECRET_SENTINEL"}"#,
        r#"{"credential_env":"SECRET_SENTINEL"}"#,
        r#"{"platform_key_pricing":{"credits_per_unit":0.1}}"#,
        r#"{"platform_key_pricing":{"credits_per_unit":"1e-3"}}"#,
    ] {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), raw).unwrap();
        let error = run(AdminBillingCommands::Service {
            command: BillingServiceCommands::Update {
                service: ID.into(),
                file: file.path().into(),
                dry_run: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains("SECRET_SENTINEL"));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn set_managed_body_matches_golden_bytes_and_exact_ceiling() {
    let (body, ceiling) = managed_billing::body(
        &managed_args(),
        support::timestamp("2026-10-20T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&body).unwrap() + "\n",
        include_str!("../../../tests/fixtures/billing/set-managed.golden.json")
    );
    assert_eq!(ceiling, "0.0038912");
}

#[tokio::test]
async fn set_managed_posts_once_and_clears_incompatible_previous_operation_prices() {
    let server = MockServer::start().await;
    let mut current = fixture("managed");
    current["billing"]["platform_key_pricing"]["operations"] = json!([
        {
            "operation": "chat_completions",
            "credits_per_unit": "1",
            "sync_status": "synced"
        }
    ]);
    mount(&server, "GET", &format!("/services/{ID}"), current).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/admin/billing/services/{ID}")))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(
                body["billing"]["platform_key_pricing"]["operations"],
                json!([])
            );
            assert!(body["billing"]["byok_pricing"].is_null());
            assert!(body["billing"]["platform_pricing"].is_null());
            assert_eq!(body["billing"]["resale_billable"], false);
            ResponseTemplate::new(200).set_body_json(json!({}))
        })
        .expect(1)
        .mount(&server)
        .await;
    let mut managed = managed_args();
    managed.tariff_verified_at = support::rfc3339(Utc::now() - Duration::hours(1));
    run(AdminBillingCommands::Service {
        command: BillingServiceCommands::SetManaged {
            service: ID.into(),
            managed,
            probe_response: None,
            dry_run: false,
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn set_managed_every_server_rule_rejects_locally_without_requests() {
    let server = MockServer::start().await;
    let mut cases = vec![];
    for (field, low, high) in [
        ("input", 0, 2_000_001),
        ("output", 0, 32_769),
        ("request", 255, 1_048_577),
        ("response", 1023, 8_388_609),
    ] {
        for bound in [low, high] {
            let mut args = managed_args();
            match field {
                "input" => args.max_input_tokens = bound,
                "output" => args.max_output_tokens = bound,
                "request" => args.max_request_bytes = bound,
                _ => args.max_response_bytes = bound,
            };
            cases.push(args);
        }
    }
    let mut args = managed_args();
    args.max_input_tokens = args.max_output_tokens;
    cases.push(args);
    for source in [
        "http://telnyx.com/pricing",
        "https://evil.example/pricing",
        "https://user:SECRET_SENTINEL@telnyx.com/pricing",
        "https://telnyx.com/pricing?q=x",
        "https://telnyx.com/pricing#x",
    ] {
        let mut args = managed_args();
        args.tariff_source = source.into();
        cases.push(args);
    }
    for timestamp in [
        support::rfc3339(Utc::now() + Duration::days(1)),
        support::rfc3339(Utc::now() - Duration::days(31)),
    ] {
        let mut args = managed_args();
        args.tariff_verified_at = timestamp;
        cases.push(args);
    }
    for price in ["0", "-1", "0.1234567890123", "1e-12", "1000001"] {
        for output in [false, true] {
            let mut args = managed_args();
            if output {
                args.output_price = price.into();
            } else {
                args.input_price = price.into();
            }
            cases.push(args);
        }
    }
    for mut args in cases {
        if args.tariff_verified_at == "2026-10-19T00:00:00Z" {
            args.tariff_verified_at = support::rfc3339(Utc::now() - Duration::hours(1));
        }
        let error = run(AdminBillingCommands::Service {
            command: BillingServiceCommands::SetManaged {
                service: ID.into(),
                managed: args,
                probe_response: None,
                dry_run: false,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains("SECRET_SENTINEL"));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn probe_rejects_missing_cached_tokens_model_tools_totals_and_modalities() {
    managed_billing::validate_probe(&probe(), &managed_args()).unwrap();
    let mut invalid = vec![];
    let mut p = probe();
    p["usage"]["prompt_tokens_details"]
        .as_object_mut()
        .unwrap()
        .remove("cached_tokens");
    invalid.push(p);
    for (pointer, value) in [
        ("/model", json!("other")),
        ("/id", json!("bad/id")),
        ("/usage/total_tokens", json!(106)),
        ("/usage/prompt_tokens", json!(100.5)),
        ("/usage/prompt_tokens_details/cached_tokens", json!(101)),
        ("/choices/0/message/tool_calls", json!([])),
        ("/usage/prompt_tokens_details/audio_tokens", json!(1)),
        ("/usage/prompt_tokens_details/image_tokens", json!(1)),
        (
            "/usage/completion_tokens_details",
            json!({"audio_tokens":1}),
        ),
        ("/choices", json!([])),
    ] {
        let mut p = probe();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        p.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .map(|object| object.insert(key.into(), value.clone()))
            .unwrap_or_else(|| {
                *p.pointer_mut(pointer).unwrap() = value;
                None
            });
        invalid.push(p);
    }
    for p in invalid {
        assert!(
            managed_billing::validate_probe(&p, &managed_args()).is_err(),
            "{p}"
        );
    }
}

#[tokio::test]
async fn set_managed_dry_run_has_no_put_and_probe_failure_has_no_get() {
    let server = MockServer::start().await;
    let mut args = managed_args();
    args.tariff_verified_at = support::rfc3339(Utc::now() - Duration::hours(1));
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), json!({}).to_string()).unwrap();
    assert!(
        run(AdminBillingCommands::Service {
            command: BillingServiceCommands::SetManaged {
                service: ID.into(),
                managed: args.clone(),
                probe_response: Some(file.path().into()),
                dry_run: true,
                auth: mock_auth(server.uri())
            }
        })
        .await
        .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    mount(
        &server,
        "GET",
        &format!("/services/{ID}"),
        fixture("managed"),
    )
    .await;
    run(AdminBillingCommands::Service {
        command: BillingServiceCommands::SetManaged {
            service: ID.into(),
            managed: args,
            probe_response: None,
            dry_run: true,
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn resync_is_once_and_boolean_does_not_mask_unsynced_rows() {
    let server = MockServer::start().await;
    let (raw, mut effective, _) = valid_state();
    effective["effective"]["classes"][0]["prices"][1]["sync_status"] = json!("failed");
    effective["effective"]["classes"][0]["prices"][1]["sync_error"] = json!("Lago unavailable");
    mount(&server, "GET", &format!("/services/{ID}"), raw).await;
    mount(
        &server,
        "POST",
        &format!("/admin/billing/services/{ID}/resync"),
        json!({"synced":true,"service":effective}),
    )
    .await;
    let error = run(AdminBillingCommands::Service {
        command: BillingServiceCommands::Resync {
            service: ID.into(),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<super::super::admin_tools::PendingReadiness>()
            .unwrap()
            .exit_code,
        2
    );
}

#[test]
fn managed_readiness_accepts_disabled_access_and_exact_equivalent_rates() {
    let (raw, effective, mut rates) = valid_state();
    rates[0]["credits_per_unit"] = json!("0.000000250000");
    assert_eq!(
        readiness(
            &fixture("config"),
            &raw,
            &effective,
            &rates,
            true,
            &fixture("endpoints")
        )["configuration_ready"],
        true
    );
}

#[tokio::test]
async fn verify_reads_billing_and_published_operations_before_reporting_not_ready() {
    let server = MockServer::start().await;
    let (mut raw, effective, rates) = valid_state();
    raw["credential_configured"] = json!(false);
    mount(&server, "GET", &format!("/services/{ID}"), raw).await;
    mount(&server, "GET", "/admin/billing/config", fixture("config")).await;
    mount(
        &server,
        "GET",
        &format!("/admin/billing/services/{ID}"),
        effective,
    )
    .await;
    mount(&server, "GET", "/admin/billing/rates", json!(rates)).await;
    mount(
        &server,
        "GET",
        &format!("/services/{ID}/endpoints"),
        fixture("endpoints"),
    )
    .await;
    let error = run(AdminBillingCommands::Service {
        command: BillingServiceCommands::Verify {
            service: ID.into(),
            managed: true,
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<super::super::admin_tools::PendingReadiness>()
            .unwrap()
            .exit_code,
        2
    );
}

macro_rules! not_ready {
    ($name:ident,$target:ident,$pointer:expr,$value:expr) => {
        #[test]
        fn $name() {
            let (mut raw, mut effective, mut rates) = valid_state();
            let mut config = fixture("config");
            let target: &mut Value = match stringify!($target) {
                "raw" => &mut raw,
                "effective" => &mut effective,
                "config" => &mut config,
                _ => &mut rates[0],
            };
            *target.pointer_mut($pointer).unwrap() = $value;
            assert_eq!(
                readiness(
                    &config,
                    &raw,
                    &effective,
                    &rates,
                    true,
                    &fixture("endpoints")
                )["configuration_ready"],
                false
            );
        }
    };
}
not_ready!(
    verify_unsynced_component,
    raw,
    "/billing/platform_key_pricing/components/0/sync_status",
    json!("failed")
);
not_ready!(
    verify_cache_rate_mismatch,
    rate,
    "/credits_per_unit",
    json!("0.00000026")
);
not_ready!(verify_stale_cache, rate, "/stale", json!(true));
not_ready!(
    verify_retired_cache,
    rate,
    "/retired_at",
    json!("2026-01-01T00:00:00Z")
);
not_ready!(
    verify_stale_tariff,
    raw,
    "/billing/managed_execution/tariff_verified_at",
    json!({"$date":support::rfc3339(Utc::now()-Duration::days(31))})
);
not_ready!(
    verify_public_audience,
    raw,
    "/platform_key/audience",
    json!("public")
);
not_ready!(
    verify_missing_credential,
    raw,
    "/credential_configured",
    json!(false)
);
not_ready!(
    verify_missing_contract,
    raw,
    "/billing/managed_execution",
    Value::Null
);
not_ready!(
    verify_missing_prices,
    raw,
    "/billing/platform_key_pricing",
    Value::Null
);
not_ready!(
    verify_billing_disabled,
    config,
    "/billing_enabled",
    json!(false)
);
not_ready!(
    verify_lago_unconfigured,
    config,
    "/lago_configured",
    json!(false)
);
not_ready!(verify_inactive_service, raw, "/is_active", json!(false));
not_ready!(
    verify_non_internal_service,
    raw,
    "/service_category",
    json!("connection")
);
not_ready!(
    verify_requires_user_credential,
    raw,
    "/requires_user_credential",
    json!(true)
);
not_ready!(
    verify_uncharged_effective_class,
    effective,
    "/effective/classes/0/platform_charged",
    json!(false)
);
not_ready!(
    verify_wrong_effective_reason,
    effective,
    "/effective/classes/0/reason",
    json!("lane_unsynced_legacy")
);

#[tokio::test]
async fn ledger_all_follows_cursors_and_detects_broken_or_invalid_anchor() {
    for (status, anchor, expected) in [("ok", true, 0), ("broken", true, 2), ("ok", false, 2)] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/billing-ledger/verify"))
            .and(query_param("from_seq", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status":"ok","next_from_seq":3,"anchor_valid":true})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/billing-ledger/verify"))
            .and(query_param("from_seq", "3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status":status,"anchor_valid":anchor})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = run(AdminBillingCommands::Ledger {
            command: BillingLedgerCommands::Verify {
                from_seq: Some(1),
                to_seq: Some(5),
                limit: Some(2),
                all: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        if expected == 0 {
            result.unwrap();
        } else {
            assert_eq!(
                result
                    .unwrap_err()
                    .downcast_ref::<super::super::admin_tools::PendingReadiness>()
                    .unwrap()
                    .exit_code,
                expected
            );
        }
    }
}

#[tokio::test]
async fn server_json_and_axum_text_errors_are_http_errors_and_writes_not_retried() {
    for status in [401, 422, 503] {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/admin/billing/config"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string(if status == 422 {
                    "Failed to deserialize the JSON body"
                } else {
                    r#"{"error":"unavailable","error_code":1002,"message":"unavailable"}"#
                }),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = run(AdminBillingCommands::Config {
            command: BillingConfigCommands::Set {
                default_overdraft_cap_credits: "1".into(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap_err();
        let value: Value =
            serde_json::from_str(&crate::error_format::render_error(&error, true)).unwrap();
        assert_eq!(value["error"], "http_error");
        assert_eq!(value["status"], status);
        assert_eq!(value["path"], "/admin/billing/config");
        assert_eq!(value["body"].is_string(), status == 422);
    }
}

fn admin_command(server: &MockServer, args: &[&str], token: &str) -> crate::cli::AdminCommands {
    let mut args: Vec<String> = ["nyxid", "admin"]
        .into_iter()
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    args.extend([
        "--base-url".into(),
        server.uri(),
        "--access-token".into(),
        token.into(),
    ]);
    let Commands::Admin { command } = Cli::try_parse_from(args).unwrap().command else {
        panic!("admin")
    };
    command
}

#[tokio::test]
async fn agent_key_credentials_reject_admin_and_billing_leaves_before_any_request() {
    let server = MockServer::start().await;
    for args in [
        vec!["billing", "config", "show"],
        vec![
            "billing",
            "config",
            "set",
            "--default-overdraft-cap-credits",
            "1",
        ],
        vec!["billing", "overview"],
        vec!["billing", "rates"],
        vec!["billing", "service", "show", ID],
        vec!["billing", "service", "update", ID, "--file", "missing.json"],
        vec!["billing", "service", "resync", ID],
        vec!["billing", "service", "verify", ID],
        vec!["billing", "ledger", "verify"],
        vec!["usage"],
        vec!["credits", "grant", "list"],
        vec!["credits", "grant", "revoke", ID],
        vec![
            "credits",
            "grant",
            "issue",
            "--user",
            ID,
            "--amount",
            "1",
            "--all-services",
            "--reason",
            "test",
            "--yes",
        ],
    ] {
        let error =
            crate::commands::admin::run(admin_command(&server, &args, "nyxid_ag_SECRET_SENTINEL"))
                .await
                .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("nyxid login"), "{args:?}: {message}");
        assert!(!message.contains("SECRET_SENTINEL"));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn explicit_human_override_wins_over_agent_key_profile() {
    let _lock = env_lock().lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let old = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    let profile = dir.path().join(".nyxid/profiles/billing-test");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("auth_kind"), "agent_key").unwrap();
    let mut auth: AuthArgs = mock_auth("http://127.0.0.1:1");
    auth.profile = Some("billing-test".into());
    support::require_human(&auth).unwrap();
    unsafe {
        match old {
            Some(old) => std::env::set_var("HOME", old),
            None => std::env::remove_var("HOME"),
        }
    }
}

#[test]
fn configured_pricing_table_labels_contract_and_price_rows_in_stable_order() {
    let output = configured_pricing_table(&fixture("managed")["billing"]);
    let mut previous = 0;
    for label in [
        "Managed contract:",
        "model",
        "max_input_tokens",
        "tariff_version",
        "Configured price lanes:",
        "platform_key_pricing",
        "output_tokens",
        "cache_read_tokens",
    ] {
        let index = previous + output[previous..].find(label).unwrap();
        assert!(index >= previous, "{label}");
        previous = index;
    }
    assert!(output.contains("0.00000025"));
    assert!(output.contains("synced"));
}

#[test]
fn configured_pricing_table_excludes_fields_outside_contract_and_price_allowlists() {
    let mut configuration = fixture("managed")["billing"].clone();
    for pointer in [
        "",
        "/managed_execution",
        "/platform_key_pricing",
        "/platform_key_pricing/components/0",
    ] {
        let row = configuration.pointer_mut(pointer).unwrap();
        row["credential"] = json!("PRIVATE_CREDENTIAL_SENTINEL");
        row["recipient_user_id"] = json!("PRIVATE_RECIPIENT_SENTINEL");
        row["unrecognized_field"] = json!("PRIVATE_UNKNOWN_SENTINEL");
    }
    let output = configured_pricing_table(&configuration);
    assert!(output.contains("Managed contract:"));
    assert!(output.contains("Configured price lanes:"));
    assert!(output.contains("0.00000025"));
    assert!(!output.contains("PRIVATE_"));
    assert!(!output.contains("credential"));
    assert!(!output.contains("recipient_user_id"));
    assert!(!output.contains("unrecognized_field"));
}

macro_rules! unsafe_surface {
    ($name:ident, $target:ident, $pointer:expr, $value:expr, $check:expr) => {
        #[test]
        fn $name() {
            let (mut raw, effective, rates) = valid_state();
            let mut endpoints = fixture("endpoints");
            let target = if stringify!($target) == "raw" {
                &mut raw
            } else {
                &mut endpoints
            };
            *target.pointer_mut($pointer).unwrap() = $value;
            let report = readiness(
                &fixture("config"),
                &raw,
                &effective,
                &rates,
                true,
                &endpoints,
            );
            assert_eq!(report["configuration_ready"], false);
            assert_eq!(report["checks"][$check], false);
        }
    };
}
unsafe_surface!(
    verify_rejects_extra_active_operation,
    endpoints,
    "/endpoints",
    json!([
        fixture("endpoints")["endpoints"][0],
        fixture("endpoints")["endpoints"][0]
    ]),
    "one_active_operation"
);
unsafe_surface!(
    verify_rejects_wrong_operation_route,
    endpoints,
    "/endpoints/0/path",
    json!("/other"),
    "managed_operation_route"
);
unsafe_surface!(
    verify_rejects_unpinned_operation_model,
    endpoints,
    "/endpoints/0/request_body_schema/properties/model/enum",
    json!(["other"]),
    "operation_model_pinned"
);
unsafe_surface!(
    verify_rejects_wrong_operation_output_cap,
    endpoints,
    "/endpoints/0/request_body_schema/properties/max_tokens/maximum",
    json!(1025),
    "operation_output_cap_pinned"
);
unsafe_surface!(
    verify_rejects_broad_proxy_policy,
    raw,
    "/proxy_operation_policy/rules/0/path_template",
    json!("/**"),
    "proxy_only_managed_route"
);
unsafe_surface!(
    verify_rejects_disabled_platform_billing,
    raw,
    "/billing/platform_billable",
    json!(false),
    "platform_billable"
);
unsafe_surface!(
    verify_rejects_billing_user_credentials,
    raw,
    "/billing/platform_charge_nyxid_credentials_only",
    json!(false),
    "platform_charge_nyxid_credentials_only"
);
unsafe_surface!(
    verify_rejects_resale_billing,
    raw,
    "/billing/resale_billable",
    json!(true),
    "resale_disabled"
);

#[test]
fn all_new_command_leaf_arguments_have_help() {
    use clap::CommandFactory;
    fn walk(command: &clap::Command) {
        if command.get_subcommands().next().is_none() {
            for arg in command.get_arguments() {
                if !matches!(arg.get_id().as_str(), "help" | "version") && !arg.is_hide_set() {
                    assert!(
                        arg.get_help().is_some() || arg.get_long_help().is_some(),
                        "{}: {} has no help",
                        command.get_name(),
                        arg.get_id()
                    );
                }
            }
            let mut command = command.clone();
            assert!(!command.render_long_help().to_string().is_empty());
        } else {
            for sub in command.get_subcommands() {
                walk(sub);
            }
        }
    }
    let mut command = Cli::command();
    command.build();
    for path in [
        &["admin", "billing"][..],
        &["admin", "credits"],
        &["admin", "usage"],
        &["billing", "executions"],
        &["mcp", "tools"],
        &["mcp", "call"],
    ] {
        let mut leaf = &command;
        for name in path {
            leaf = leaf.find_subcommand(name).unwrap();
        }
        walk(leaf);
    }
}

#[test]
fn human_guard_honors_override_precedence_and_all_server_api_key_prefixes() {
    let _lock = env_lock().lock().unwrap();
    let old = std::env::var_os("NYXID_BILLING_TEST_TOKEN");
    let mut auth = mock_auth("http://127.0.0.1:1");
    auth.access_token_env = "NYXID_BILLING_TEST_TOKEN".into();
    for prefix in ["nyx_", "nyxid_", "nyxid_ag_", "nyxid_sk_"] {
        auth.access_token = Some(format!("{prefix}private-sentinel"));
        let error = support::require_human(&auth).unwrap_err().to_string();
        assert!(!error.contains("private-sentinel"));
        unsafe {
            std::env::set_var("NYXID_BILLING_TEST_TOKEN", "human-fixture");
        }
        assert!(
            support::require_human(&auth).is_err(),
            "explicit API key wins"
        );
        auth.access_token = None;
        support::require_human(&auth).unwrap();
        unsafe {
            std::env::set_var(
                "NYXID_BILLING_TEST_TOKEN",
                format!("{prefix}private-sentinel"),
            );
        }
        assert!(support::require_human(&auth).is_err());
    }
    unsafe {
        match old {
            Some(old) => std::env::set_var("NYXID_BILLING_TEST_TOKEN", old),
            None => std::env::remove_var("NYXID_BILLING_TEST_TOKEN"),
        }
    }
}

#[tokio::test]
async fn every_server_api_key_prefix_is_rejected_before_requests() {
    let server = MockServer::start().await;
    for prefix in ["nyx_", "nyxid_", "nyxid_ag_", "nyxid_sk_"] {
        let mut auth = mock_auth(server.uri());
        auth.access_token = Some(format!("{prefix}fixture"));
        let error = run(AdminBillingCommands::Config {
            command: BillingConfigCommands::Show { auth },
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("human account session"));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn billing_preview_matches_server_legacy_defaults_and_lane_preservation() {
    // handlers/services.rs BillingUpdate::preserve_omitted_fields (396-466)
    // retains the legacy full-block semantics unless a lane key is present.
    let current = json!({
        "platform_billable":true, "platform_metric":"requests",
        "platform_pricing":{"metric":"requests","credits_per_unit":"1"},
        "resale_billable":true,"resale_metric":"requests","lago_resale_metric_code":"resale",
        "platform_charge_nyxid_credentials_only":true,
        "managed_execution":{"model":"org/model:1"}
    });
    let legacy = merged_billing(&current, &json!({"platform_metric":"tokens"}));
    assert_eq!(legacy["platform_billable"], false);
    assert_eq!(legacy["platform_pricing"], Value::Null);
    assert_eq!(legacy["resale_billable"], false);
    assert_eq!(legacy["resale_metric"], "tokens");
    assert_eq!(legacy["lago_resale_metric_code"], Value::Null);
    assert_eq!(legacy["managed_execution"], current["managed_execution"]);
    assert_eq!(legacy["platform_charge_nyxid_credentials_only"], true);
    let lane = merged_billing(&current, &json!({"byok_pricing":null}));
    for key in current.as_object().unwrap().keys() {
        assert_eq!(lane[key], current[key], "{key}");
    }
}
