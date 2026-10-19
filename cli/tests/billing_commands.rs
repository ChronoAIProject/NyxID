use serde_json::{Value, json};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path, query_param},
};

const ID: &str = "00000000-0000-4000-8000-000000000001";
const SENTINEL: &str = "test-secret-sentinel-never-print";

async fn run(server: &MockServer, args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(args)
        .env("NYXID_URL", server.uri())
        .env("NYXID_ACCESS_TOKEN", SENTINEL)
        .env("HOME", home.path())
        .env("CI", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_TELEMETRY_DSN")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .unwrap()
}
fn output_text(out: &std::process::Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
fn json(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{}", output_text(out)))
}
async fn get(server: &MockServer, url: &str, value: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1{url}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(server)
        .await;
}
fn raw_service() -> Value {
    let mut raw: Value =
        serde_json::from_str(include_str!("fixtures/billing/managed-service.json")).unwrap();
    raw["billing"]["managed_execution"]["tariff_verified_at"] =
        json!({"$date":(chrono::Utc::now()-chrono::Duration::hours(1)).to_rfc3339()});
    raw
}
async fn readiness(server: &MockServer, change: Option<&str>) {
    let mut raw = raw_service();
    let mut rates = vec![];
    let lane = &raw["billing"]["platform_key_pricing"];
    for row in std::iter::once(lane).chain(lane["components"].as_array().unwrap().iter()) {
        rates.push(json!({"lago_metric_code":row["lago_metric_code"],"model":null,"credits_per_unit":row["credits_per_unit"],"stale":false,"retired_at":null}));
    }
    match change {
        Some("unsynced") => {
            raw["billing"]["platform_key_pricing"]["components"][0]["sync_status"] = json!("failed")
        }
        Some("mismatch") => rates[0]["credits_per_unit"] = json!("0.1"),
        Some("stale") => rates[0]["stale"] = json!(true),
        Some("retired") => rates[0]["retired_at"] = json!("2026-01-01T00:00:00Z"),
        Some("tariff") => {
            raw["billing"]["managed_execution"]["tariff_verified_at"] =
                json!({"$date":(chrono::Utc::now()-chrono::Duration::days(31)).to_rfc3339()})
        }
        Some("public") => raw["platform_key"]["audience"] = json!("public"),
        Some("credential") => raw["credential_configured"] = json!(false),
        _ => {}
    }
    get(server, &format!("/services/{ID}"), raw).await;
    get(
        server,
        "/admin/billing/config",
        json!({"billing_enabled":true,"lago_configured":true}),
    )
    .await;
    get(server,&format!("/admin/billing/services/{ID}"),json!({"effective":{"classes":[{"lane":"platform_key","platform_charged":true,"reason":"lane_synced"}]}})).await;
    get(server, "/admin/billing/rates", json!(rates)).await;
}

#[tokio::test]
async fn proxy_unchanged_stdout_is_byte_identical_and_execution_goes_to_stderr() {
    let server = MockServer::start().await;
    let body = " {\"response\": [1, 2]}\n \n";
    Mock::given(method("GET"))
        .and(path("/api/v1/proxy/s/test/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("x-nyxid-execution-id", ID),
        )
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        &server,
        &["proxy", "request", "test", "models", "--output", "json"],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    // The previous implementation used println!("{body}") for nonstreaming responses.
    assert_eq!(out.stdout, format!("{body}\n").as_bytes());
    assert!(String::from_utf8_lossy(&out.stderr).contains(&format!("Execution: {ID}")));
    assert!(!output_text(&out).contains(SENTINEL));
}

#[tokio::test]
async fn proxy_envelope_allowlists_headers_and_preserves_provider_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/proxy/s/test/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true}))
                .insert_header("x-nyxid-execution-id", ID)
                .insert_header("x-nyxid-secret", SENTINEL)
                .insert_header("set-cookie", SENTINEL),
        )
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        &server,
        &[
            "proxy",
            "request",
            "test",
            "models",
            "--output",
            "json",
            "--envelope",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    let value = json(&out);
    assert_eq!(value["status"], 200);
    assert_eq!(value["execution_id"], ID);
    assert_eq!(value["body"], json!({"ok":true}));
    assert_eq!(value["headers"], json!({"x-nyxid-execution-id":ID}));
    assert!(!output_text(&out).contains(SENTINEL));
}

#[tokio::test]
async fn proxy_pending_exits_two_and_http_errors_keep_status_path_and_body() {
    for status in [409, 422, 503] {
        let server = MockServer::start().await;
        let body = if status == 409 {
            json!({"error":"managed_execution_pending","error_code":12800,"message":format!("Managed execution {ID} is pending usage verification; inspect its status and do not retry automatically")}).to_string()
        } else if status == 422 {
            "Invalid JSON body".into()
        } else {
            json!({"error":"unavailable"}).to_string()
        };
        Mock::given(method("GET"))
            .and(path("/api/v1/proxy/s/test/models"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let out = run(
            &server,
            &["proxy", "request", "test", "models", "--output", "json"],
        )
        .await;
        assert_eq!(
            out.status.code(),
            Some(if status == 409 { 2 } else { 1 }),
            "{}",
            output_text(&out)
        );
        if status == 409 {
            assert_eq!(
                json(&out),
                json!({"error":"managed_execution_pending","execution_id":ID})
            );
        } else {
            let err: Value = serde_json::from_slice(&out.stderr).unwrap();
            assert_eq!(err["error"], "http_error");
            assert_eq!(err["status"], status);
            assert_eq!(err["path"], "/proxy/s/test/models");
            assert_eq!(err["body"].is_string(), status == 422);
        }
    }
}

#[tokio::test]
async fn managed_verify_ready_exits_zero_while_access_is_disabled() {
    let server = MockServer::start().await;
    readiness(&server, None).await;
    let out = run(
        &server,
        &[
            "admin",
            "billing",
            "service",
            "verify",
            ID,
            "--managed",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    assert_eq!(json(&out)["configuration_ready"], true);
}
macro_rules! verify_failure {
    ($name:ident,$case:expr) => {
        #[tokio::test]
        async fn $name() {
            let server = MockServer::start().await;
            readiness(&server, Some($case)).await;
            let out = run(
                &server,
                &[
                    "admin",
                    "billing",
                    "service",
                    "verify",
                    ID,
                    "--managed",
                    "--output",
                    "json",
                ],
            )
            .await;
            assert_eq!(out.status.code(), Some(2), "{}", output_text(&out));
            assert_eq!(json(&out)["configuration_ready"], false);
        }
    };
}
verify_failure!(verify_unsynced_component_exits_two, "unsynced");
verify_failure!(verify_rate_mismatch_exits_two, "mismatch");
verify_failure!(verify_stale_cache_exits_two, "stale");
verify_failure!(verify_retired_cache_exits_two, "retired");
verify_failure!(verify_old_tariff_exits_two, "tariff");
verify_failure!(verify_public_audience_exits_two, "public");
verify_failure!(verify_missing_credential_exits_two, "credential");

#[tokio::test]
async fn config_json_and_table_redact_secret_values_and_support_saved_profile() {
    let server = MockServer::start().await;
    get(&server,"/admin/billing/config",json!({"billing_enabled":true,"lago_configured":true,"default_overdraft_cap_credits":{"effective":0,"source":"env","env":0},"environment":[{"key":"LAGO_API_KEY","secret_set":true,"value":SENTINEL}]})).await;
    for format in ["json", "table"] {
        let out = run(
            &server,
            &[
                "admin",
                "billing",
                "config",
                "show",
                "--profile",
                "billing-test",
                "--output",
                format,
            ],
        )
        .await;
        assert!(out.status.success(), "{}", output_text(&out));
        assert!(!output_text(&out).contains(SENTINEL));
        if format == "json" {
            assert!(out.stderr.is_empty());
            assert_eq!(json(&out)["environment"][0]["value"], Value::Null);
        } else {
            assert!(out.stdout.is_empty());
            assert!(String::from_utf8_lossy(&out.stderr).contains("secret_set"));
        }
    }
}

#[tokio::test]
async fn rates_output_has_exactly_the_services_nonretired_codes() {
    let server = MockServer::start().await;
    get(&server, &format!("/services/{ID}"), raw_service()).await;
    let codes = [
        "platform_svc_telnyx-platform_pk",
        "platform_svc_telnyx-platform_pk_output_tokens",
        "platform_svc_telnyx-platform_pk_cache_read_tokens",
    ];
    let mut rows=codes.iter().map(|code|json!({"lago_metric_code":code,"credits_per_unit":"0.000000000001","retired_at":null})).collect::<Vec<_>>();
    rows[1]["retired_at"] = json!("2026-01-01T00:00:00Z");
    rows.push(json!({"lago_metric_code":"other","retired_at":null}));
    get(&server, "/admin/billing/rates", json!(rows)).await;
    for include in [false, true] {
        let mut args = vec![
            "admin",
            "billing",
            "rates",
            "--service",
            ID,
            "--output",
            "json",
        ];
        if include {
            args.push("--include-retired");
        }
        let out = run(&server, &args).await;
        assert!(out.status.success(), "{}", output_text(&out));
        let value = json(&out);
        let rows = value.as_array().unwrap();
        assert_eq!(rows.len(), if include { 3 } else { 2 });
        assert!(
            rows.iter()
                .all(|row| codes.contains(&row["lago_metric_code"].as_str().unwrap()))
        );
    }
}

#[tokio::test]
async fn execution_since_filters_stdout_and_wait_timeout_has_exit_two() {
    let server = MockServer::start().await;
    get(&server,"/billing/executions",json!([{"id":ID,"created_at":"2026-10-20T00:00:00Z"},{"id":"old","created_at":"2026-10-18T00:00:00Z"}])).await;
    get(&server,&format!("/billing/executions/{ID}"),json!({"id":ID,"state":"pending","maximum_credits":"0.000000000001","pending_reason":"usage"})).await;
    let out = run(
        &server,
        &[
            "billing",
            "executions",
            "list",
            "--since",
            "2026-10-19T00:00:00Z",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(out.status.success());
    assert_eq!(json(&out).as_array().unwrap().len(), 1);
    let out = run(
        &server,
        &[
            "billing",
            "executions",
            "show",
            ID,
            "--wait",
            "--timeout-secs",
            "0",
            "--output",
            "json",
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(json(&out)["state"], "pending");
}

#[tokio::test]
async fn execution_wait_polls_until_settled_and_ledger_broken_exits_two() {
    let server = MockServer::start().await;
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::given(method("GET")).and(path(format!("/api/v1/billing/executions/{ID}"))).respond_with(move|_:&Request|ResponseTemplate::new(200).set_body_json(json!({"id":ID,"state":if count.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0 {"pending"}else{"settled"}}))).expect(2).mount(&server).await;
    let out = run(
        &server,
        &[
            "billing",
            "executions",
            "show",
            ID,
            "--wait",
            "--timeout-secs",
            "3",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    assert_eq!(json(&out)["state"], "settled");
    get(
        &server,
        "/admin/billing-ledger/verify",
        json!({"status":"broken","anchor_valid":true}),
    )
    .await;
    let out = run(
        &server,
        &[
            "admin", "billing", "ledger", "verify", "--all", "--output", "json",
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(json(&out)["status"], "broken");
}

#[tokio::test]
async fn grant_issue_json_prints_batch_id_and_each_activation_state() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/v1/admin/credits/grants")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"batch_id":"test-batch","recipients":[{"recipient_user_id":ID,"activation_state":"active"},{"recipient_user_id":"second","activation_state":"pending_activation"}]}))).expect(1).mount(&server).await;
    let out = run(
        &server,
        &[
            "admin",
            "credits",
            "grant",
            "issue",
            "--user",
            ID,
            "--amount",
            "5",
            "--all-services",
            "--reason",
            "Pilot",
            "--yes",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    let value = json(&out);
    assert_eq!(value["batch_id"], "test-batch");
    assert_eq!(
        value["recipients"][1]["activation_state"],
        "pending_activation"
    );
    assert!(!output_text(&out).contains(SENTINEL));
}

#[tokio::test]
async fn catalog_apply_same_manifest_twice_is_unchanged_with_server_sync_and_bson_dates() {
    let server = MockServer::start().await;
    let raw: Value =
        serde_json::from_str(include_str!("fixtures/billing/managed-service.json")).unwrap();
    get(&server, "/services", json!({"services":[raw]})).await;
    get(&server, "/providers", json!({"providers":[]})).await;
    let file = tempfile::NamedTempFile::new().unwrap();
    let billing: Value =
        serde_json::from_str(include_str!("fixtures/billing/set-managed.golden.json")).unwrap();
    std::fs::write(
        file.path(),
        json!({"services":[{"slug":"telnyx-platform","billing":billing["billing"]}]}).to_string(),
    )
    .unwrap();
    for _ in 0..2 {
        let out = run(
            &server,
            &[
                "admin",
                "catalog",
                "apply",
                "--file",
                file.path().to_str().unwrap(),
                "--output",
                "json",
            ],
        )
        .await;
        assert!(out.status.success(), "{}", output_text(&out));
        assert_eq!(json(&out)["results"][0]["action"], "unchanged");
    }
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
}

#[tokio::test]
async fn mcp_json_and_sse_errors_have_binary_exit_one_or_two() {
    for pending in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/mcp")).respond_with(move|request:&Request|{
            let body:Value=serde_json::from_slice(&request.body).unwrap();let id=body["id"].clone();
            match body["method"].as_str().unwrap() {
                "initialize"=>ResponseTemplate::new(200).insert_header("Mcp-Session-Id","s").set_body_json(json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}})),
                "notifications/initialized"=>ResponseTemplate::new(202),
                "tools/call"=>{let text=if pending {format!("Managed execution {ID} is pending usage verification; inspect its status and do not retry automatically")}else{"Rejected".into()};let message=json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":text}]}});ResponseTemplate::new(200).set_body_raw(format!("data: {message}\n\n"),"text/event-stream")},
                _=>panic!("unexpected method"),
            }
        }).expect(3).mount(&server).await;
        let out = run(
            &server,
            &[
                "mcp",
                "call",
                "tool",
                "--args",
                "{}",
                "--envelope",
                "--output",
                "json",
            ],
        )
        .await;
        assert_eq!(
            out.status.code(),
            Some(if pending { 2 } else { 1 }),
            "{}",
            output_text(&out)
        );
        if pending {
            assert_eq!(json(&out)["execution_id"], ID);
        } else {
            assert_eq!(json(&out)["result"]["isError"], true);
        }
    }
}

#[tokio::test]
async fn wallet_owner_id_is_forwarded_on_get_in_the_binary() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/billing/wallet"))
        .and(query_param("owner_id", ID))
        .respond_with(ResponseTemplate::new(422).set_body_string("fixture rejection"))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        &server,
        &["billing", "wallet", "--owner-id", ID, "--output", "json"],
    )
    .await;
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"], "http_error");
    assert_eq!(err["status"], 422);
}

#[tokio::test]
async fn saved_agent_profile_rejects_every_admin_billing_leaf_with_exit_one_and_zero_requests() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join(".nyxid/profiles/billing-agent");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("auth_kind"), "agent_key").unwrap();
    std::fs::write(profile.join("token"), "nyxid_ag_SECRET_SENTINEL").unwrap();
    let commands = [
        vec!["admin", "billing", "config", "show"],
        vec![
            "admin",
            "billing",
            "config",
            "set",
            "--default-overdraft-cap-credits",
            "1",
        ],
        vec!["admin", "billing", "overview"],
        vec!["admin", "billing", "rates"],
        vec!["admin", "billing", "service", "show", ID],
        vec![
            "admin",
            "billing",
            "service",
            "update",
            ID,
            "--file",
            "missing.json",
        ],
        vec![
            "admin",
            "billing",
            "service",
            "set-managed",
            ID,
            "--model",
            "model",
            "--max-input-tokens",
            "8192",
            "--max-output-tokens",
            "1024",
            "--max-request-bytes",
            "65536",
            "--max-response-bytes",
            "262144",
            "--tariff-version",
            "v1",
            "--tariff-source",
            "https://telnyx.com/pricing",
            "--tariff-verified-at",
            "2026-10-20T00:00:00Z",
            "--input-price",
            "0.1",
            "--output-price",
            "0.2",
            "--cache-read-price",
            "0",
        ],
        vec!["admin", "billing", "service", "resync", ID],
        vec!["admin", "billing", "service", "verify", ID, "--managed"],
        vec!["admin", "billing", "ledger", "verify"],
        vec!["admin", "usage"],
        vec![
            "admin",
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
        vec!["admin", "credits", "grant", "list"],
        vec!["admin", "credits", "grant", "revoke", ID],
        vec![
            "admin",
            "catalog",
            "add",
            "--template",
            "telnyx-platform",
            "--dry-run",
        ],
        vec![
            "admin",
            "catalog",
            "apply",
            "--file",
            "missing.json",
            "--dry-run",
        ],
        vec!["billing", "executions", "list"],
        vec!["billing", "executions", "show", ID],
        vec!["billing", "wallet"],
        vec!["billing", "wallet", "--provision"],
        vec!["billing", "usage"],
        vec!["billing", "topup", "--amount-credits", "1"],
        vec!["billing", "verify-topup-flow"],
    ];
    for args in commands {
        let out = Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args(&args)
            .args(["--profile", "billing-agent", "--output", "json"])
            .env("NYXID_URL", server.uri())
            .env("HOME", home.path())
            .env("CI", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env_remove("NYXID_ACCESS_TOKEN")
            .env_remove("NYXID_API_KEY")
            .env_remove("NYXID_PROFILE")
            .env_remove("NYXID_TELEMETRY_DSN")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{args:?}: {}",
            output_text(&out)
        );
        assert!(
            output_text(&out).contains("nyxid login"),
            "{args:?}: {}",
            output_text(&out)
        );
        assert!(!output_text(&out).contains("SECRET_SENTINEL"));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn saved_human_profile_and_explicit_base_url_read_config_without_token_override() {
    let server = MockServer::start().await;
    get(
        &server,
        "/admin/billing/config",
        json!({"billing_enabled":true,"lago_configured":true}),
    )
    .await;
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join(".nyxid/profiles/billing-human");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("access_token"), SENTINEL).unwrap();
    std::fs::write(profile.join("base_url"), server.uri()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args([
            "admin",
            "billing",
            "config",
            "show",
            "--profile",
            "billing-human",
            "--base-url",
            &server.uri(),
            "--output",
            "json",
        ])
        .env("HOME", home.path())
        .env("CI", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_URL")
        .env_remove("NYXID_ACCESS_TOKEN")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_TELEMETRY_DSN")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{}", output_text(&out));
    assert_eq!(json(&out)["billing_enabled"], true);
    assert!(!output_text(&out).contains(SENTINEL));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn usage_table_renders_flat_service_and_credential_class_exact_costs() {
    let server = MockServer::start().await;
    get(&server, "/admin/usage", json!({"totals":{},"by_service":[{"service_slug":"telnyx-platform","quantities":{"input_tokens":91,"output_tokens":5,"cache_read_tokens":10},"gross_cost":"0.123456789012","wallet_cost":"0.000000000001","grant_cost":"0.000000000002","allowance_cost":"0.000000000003","by_credential_class":[{"credential_class":"nyxid_managed_master","quantities":{"input_tokens":91,"output_tokens":5,"cache_read_tokens":10},"gross_cost":"0.987654321098","wallet_cost":"0.000000000004","grant_cost":"0.000000000005","allowance_cost":"0.000000000006"}]}]})).await;
    let out = run(&server, &["admin", "usage"]).await;
    assert!(out.status.success(), "{}", output_text(&out));
    assert!(out.stdout.is_empty());
    for expected in [
        "telnyx-platform",
        "nyxid_managed_master",
        "91",
        "0.123456789012",
        "0.987654321098",
        "0.000000000001",
        "0.000000000006",
    ] {
        assert!(
            output_text(&out).contains(expected),
            "{expected}: {}",
            output_text(&out)
        );
    }
}
