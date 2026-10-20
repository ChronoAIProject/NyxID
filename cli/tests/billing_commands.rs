use serde_json::{Value, json};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path, query_param},
};

const ID: &str = "00000000-0000-4000-8000-000000000001";
const GRANT_REQUEST_ID: &str = "00000000-0000-4000-8000-000000000002";
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
    raw["billing"]["managed_execution"]["tariff_verified_at"] = json!({
        "$date": (chrono::Utc::now()-chrono::Duration::hours(1)).to_rfc3339()
    });
    raw
}
async fn readiness(server: &MockServer, change: Option<&str>) {
    get(
        server,
        &format!("/services/{ID}/endpoints"),
        serde_json::from_str(include_str!("fixtures/billing/managed-endpoints.json")).unwrap(),
    )
    .await;
    let mut raw = raw_service();
    let mut rates = vec![];
    let lane = &raw["billing"]["platform_key_pricing"];
    for row in std::iter::once(lane).chain(lane["components"].as_array().unwrap().iter()) {
        rates.push(json!({
            "lago_metric_code": row["lago_metric_code"],
            "model": null,
            "credits_per_unit": row["credits_per_unit"],
            "stale": false,
            "retired_at": null
        }));
    }
    match change {
        Some("unsynced") => {
            raw["billing"]["platform_key_pricing"]["components"][0]["sync_status"] = json!("failed")
        }
        Some("mismatch") => rates[0]["credits_per_unit"] = json!("0.1"),
        Some("stale") => rates[0]["stale"] = json!(true),
        Some("retired") => rates[0]["retired_at"] = json!("2026-01-01T00:00:00Z"),
        Some("tariff") => {
            raw["billing"]["managed_execution"]["tariff_verified_at"] = json!({
                "$date": (chrono::Utc::now()-chrono::Duration::days(31)).to_rfc3339()
            })
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
    get(
        server,
        &format!("/admin/billing/services/{ID}"),
        json!({
            "effective": {
                "classes": [
                    {
                        "lane": "platform_key",
                        "platform_charged": true,
                        "reason": "lane_synced"
                    }
                ]
            }
        }),
    )
    .await;
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
                .insert_header("x-request-id", "request-test")
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
    assert_eq!(
        value["headers"],
        json!({"x-nyxid-execution-id":ID,"x-request-id":"request-test"})
    );
    assert!(!output_text(&out).contains(SENTINEL));
}

#[tokio::test]
async fn proxy_pending_exits_two_and_http_errors_keep_status_path_and_body() {
    for status in [409, 422, 503] {
        let server = MockServer::start().await;
        let body = if status == 409 {
            json!({
                "error": "managed_execution_pending",
                "error_code": 12800,
                "message": format!(concat!(
                    "Managed execution {} is pending usage verification; ",
                    "inspect its status and do not retry automatically"
                ), ID)
            })
            .to_string()
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
    get(
        &server,
        "/admin/billing/config",
        json!({
            "billing_enabled": true,
            "lago_configured": true,
            "default_overdraft_cap_credits": {
                "effective": 0,
                "source": "env",
                "env": 0
            },
            "environment": [
                {
                    "key": "LAGO_API_KEY",
                    "secret_set": true,
                    "value": SENTINEL
                }
            ]
        }),
    )
    .await;
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
    let mut rows = codes
        .iter()
        .map(|code| {
            json!({
                "lago_metric_code": code,
                "credits_per_unit": "0.000000000001",
                "retired_at": null
            })
        })
        .collect::<Vec<_>>();
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
    get(
        &server,
        "/billing/executions",
        json!([
            {
                "id": ID,
                "created_at": "2026-10-20T00:00:00Z"
            },
            {
                "id": "old",
                "created_at": "2026-10-18T00:00:00Z"
            }
        ]),
    )
    .await;
    get(
        &server,
        &format!("/billing/executions/{ID}"),
        json!({
            "id": ID,
            "state": "pending",
            "maximum_credits": "0.000000000001",
            "pending_reason": "usage"
        }),
    )
    .await;
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
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/billing/executions/{ID}")))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(200).set_body_json(json!({
                "id": ID,
                "state": if count.fetch_add(1,
                std::sync::atomic::Ordering::SeqCst)==0 {
                    "pending"
                }else{
                    "settled"
                }
            }))
        })
        .expect(2)
        .mount(&server)
        .await;
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
    Mock::given(method("POST"))
        .and(path("/api/v1/admin/credits/grants"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batch_id": "test-batch",
            "recipients": [
                {
                    "recipient_user_id": ID,
                    "activation_state": "active"
                },
                {
                    "recipient_user_id": "second",
                    "activation_state": "pending_activation"
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
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
    let request_id = value["request_id"].as_str().unwrap();
    assert_eq!(
        uuid::Uuid::parse_str(request_id).unwrap().get_version_num(),
        4
    );
    let receipt: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(receipt, json!({"request_id": request_id}));
    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["request_id"], request_id);
    assert_eq!(body["amount"], "5");
    assert!(body.get("amount_credits").is_none());
    assert_eq!(value["batch_id"], "test-batch");
    assert_eq!(
        value["recipients"][1]["activation_state"],
        "pending_activation"
    );
    assert!(!output_text(&out).contains(SENTINEL));
}

fn grant_issue_args(amount: &str) -> Vec<&str> {
    vec![
        "admin",
        "credits",
        "grant",
        "issue",
        "--user",
        ID,
        "--all-services",
        "--amount",
        amount,
        "--reason",
        "Pilot",
        "--yes",
    ]
}

fn grant_diagnostics(out: &std::process::Output) -> Vec<Value> {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("{}", output_text(out))))
        .collect()
}

#[tokio::test]
async fn grant_issue_reuses_request_id_and_exact_decimal_body_across_invocations() {
    let server = MockServer::start().await;
    let expected = json!({
        "request_id": GRANT_REQUEST_ID,
        "amount": "0.123456789012",
        "target_kind": "selected_users",
        "target_user_ids": [ID],
        "all_services": true,
        "service_refs": [],
        "expires_at": null,
        "reason": "Pilot"
    });
    Mock::given(method("POST"))
        .and(path("/api/v1/admin/credits/grants"))
        .and(wiremock::matchers::body_json(expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"batch_id": ID})))
        .expect(2)
        .mount(&server)
        .await;
    let mut args = grant_issue_args("0.123456789012");
    args.extend(["--request-id", GRANT_REQUEST_ID, "--output", "json"]);
    for _ in 0..2 {
        let out = run(&server, &args).await;
        assert!(out.status.success(), "{}", output_text(&out));
        assert_eq!(json(&out)["request_id"], GRANT_REQUEST_ID);
        assert_eq!(json(&out)["batch_id"], ID);
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].body, requests[1].body);
}

#[tokio::test]
async fn grant_request_id_is_available_before_the_server_finishes_in_both_formats() {
    use tokio::io::{AsyncBufReadExt, BufReader};

    for format in ["table", "json"] {
        let server = MockServer::start().await;
        let dispatched = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = dispatched.clone();
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/credits/grants"))
            .and(wiremock::matchers::body_partial_json(json!({
                "request_id": GRANT_REQUEST_ID
            })))
            .respond_with(move |_: &Request| {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .set_body_json(json!({"batch_id": ID}))
                    .set_delay(std::time::Duration::from_secs(30))
            })
            .mount(&server)
            .await;
        let home = tempfile::tempdir().unwrap();
        let mut args = grant_issue_args("5");
        args.extend(["--request-id", GRANT_REQUEST_ID, "--output", format]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_nyxid"))
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
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut diagnostics = BufReader::new(child.stderr.take().unwrap()).lines();
        let receipt =
            tokio::time::timeout(std::time::Duration::from_secs(10), diagnostics.next_line())
                .await
                .expect("request ID must be available while the POST is pending")
                .unwrap()
                .unwrap();
        if format == "json" {
            let receipt: Value = serde_json::from_str(&receipt).unwrap();
            assert_eq!(receipt, json!({"request_id": GRANT_REQUEST_ID}));
        } else {
            assert_eq!(receipt, format!("Request ID: {GRANT_REQUEST_ID}"));
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !dispatched.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("grant POST should be in flight with the printed request ID");
        assert!(child.try_wait().unwrap().is_none());
        child.kill().await.unwrap();
    }
}

#[tokio::test]
async fn grant_issue_help_and_table_conflict_explain_retry_identity() {
    let server = MockServer::start().await;
    let help = run(&server, &["admin", "credits", "grant", "issue", "--help"]).await;
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for text in [
        "--request-id <UUID>",
        "12 fractional digits",
        "identical settings",
        "Without --request-id, each invocation creates a new batch",
        "all server replicas at NyxID 0.72.0 or later (credit allocations, #1950)",
    ] {
        assert!(help.contains(text), "Missing {text}: {help}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    Mock::given(method("POST"))
        .and(path("/api/v1/admin/credits/grants"))
        .respond_with(ResponseTemplate::new(409).set_body_string("different settings"))
        .expect(1)
        .mount(&server)
        .await;
    let mut args = grant_issue_args("5");
    args.extend(["--request-id", GRANT_REQUEST_ID]);
    let out = run(&server, &args).await;
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let error = String::from_utf8_lossy(&out.stderr);
    let expected = format!(
        "Request ID: {GRANT_REQUEST_ID}\n\
         Error: /admin/credits/grants failed (HTTP 409 Conflict): different settings\n\
         409 Conflict: a reused request ID requires identical settings. \
         Retry with --request-id {GRANT_REQUEST_ID} and identical settings using the same account \
         to return the original batch. A new invocation without --request-id creates a new batch.\n"
    );
    assert_eq!(error, expected);
}

#[tokio::test]
async fn grant_issue_errors_preserve_request_id_http_details_and_retry_guidance() {
    for (status, response) in [
        (
            409,
            "This allocation request was already used with different settings",
        ),
        (503, "unavailable"),
        (200, "invalid JSON after issuance"),
        (200, "[]"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/credits/grants"))
            .respond_with(ResponseTemplate::new(status).set_body_string(response))
            .expect(1)
            .mount(&server)
            .await;
        let mut args = grant_issue_args("5");
        args.extend(["--request-id", GRANT_REQUEST_ID, "--output", "json"]);
        let out = run(&server, &args).await;
        assert_eq!(out.status.code(), Some(1), "{}", output_text(&out));
        assert!(out.stdout.is_empty());
        let diagnostics = grant_diagnostics(&out);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0], json!({"request_id": GRANT_REQUEST_ID}));
        let error = &diagnostics[1];
        assert_eq!(error["request_id"], GRANT_REQUEST_ID);
        let guidance = error["retry_guidance"].as_str().unwrap();
        let prefix = if status == 409 {
            "409 Conflict: a reused request ID requires identical settings. "
        } else {
            ""
        };
        let expected_guidance = format!(
            "{prefix}Retry with --request-id {GRANT_REQUEST_ID} and identical settings using the same account \
             to return the original batch. A new invocation without --request-id creates a new batch."
        );
        assert_eq!(guidance, expected_guidance);
        if status != 200 {
            assert_eq!(error["error"], "http_error");
            assert_eq!(error["status"], status);
            assert_eq!(error["path"], "/admin/credits/grants");
            assert_eq!(error["body"], response);
        }
        if status == 409 {
            assert!(guidance.contains("409 Conflict"));
            assert!(guidance.contains("reused request ID requires identical settings"));
        }
        if response == "[]" {
            assert_eq!(
                error["message"],
                format!(
                    "Grant request {GRANT_REQUEST_ID}: Grant issuance returned a non-object response"
                )
            );
        }
        assert!(!output_text(&out).contains(SENTINEL));
    }
}

#[tokio::test]
async fn grant_issue_invalid_request_id_and_amount_are_local_binary_errors() {
    let server = MockServer::start().await;
    for amount in [
        "0",
        "-1",
        "1000001",
        "0.0000000000001",
        "1.0000000000000",
        "1e2",
    ] {
        let flag = format!("--amount={amount}");
        let out = run(
            &server,
            &[
                "admin",
                "credits",
                "grant",
                "issue",
                "--user",
                ID,
                "--all-services",
                &flag,
                "--reason",
                "Pilot",
                "--yes",
                "--output",
                "json",
            ],
        )
        .await;
        assert_eq!(out.status.code(), Some(1), "{}", output_text(&out));
        assert!(out.stdout.is_empty());
    }
    let mut args = grant_issue_args("5");
    args.extend(["--request-id", "not-a-uuid", "--output", "json"]);
    let out = run(&server, &args).await;
    assert_eq!(out.status.code(), Some(1));
    assert!(output_text(&out).contains("--request-id must be a UUID"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn grant_issue_refreshes_proactively_and_replays_401_once_with_identical_body() {
    for (proactive, final_status) in [(false, 200), (true, 200), (false, 401)] {
        let server = MockServer::start().await;
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/credits/grants"))
            .respond_with(move |_: &Request| {
                let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if attempt == 0 || final_status == 401 {
                    ResponseTemplate::new(401)
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({"batch_id": ID}))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let refreshes = std::sync::atomic::AtomicUsize::new(0);
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/refresh"))
            .respond_with(move |_: &Request| {
                let refresh = refreshes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": format!("fresh-session-fixture-{refresh}"),
                    "refresh_token": format!("rotated-refresh-fixture-{refresh}")
                }))
            })
            .expect(if proactive { 2 } else { 1 })
            .mount(&server)
            .await;
        let mut args = grant_issue_args("0.000000000001");
        if proactive {
            args.extend(["--request-id", GRANT_REQUEST_ID]);
        }
        let token = if proactive {
            "eyJhbGciOiJub25lIn0.eyJleHAiOjF9.signature"
        } else {
            "stale-session-fixture"
        };
        let out = run_saved_session(&server, &args, token).await;
        assert_eq!(
            out.status.success(),
            final_status == 200,
            "{}",
            output_text(&out)
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].url.path().ends_with("/auth/refresh"), proactive);
        let posts: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path().ends_with("/credits/grants"))
            .collect();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].body, posts[1].body);
        let initial_token = if proactive {
            "Bearer fresh-session-fixture-0"
        } else {
            "Bearer stale-session-fixture"
        };
        let replay_token = if proactive {
            "Bearer fresh-session-fixture-1"
        } else {
            "Bearer fresh-session-fixture-0"
        };
        assert_eq!(posts[0].headers["authorization"], initial_token);
        assert_eq!(posts[1].headers["authorization"], replay_token);
        let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
        let request_id = body["request_id"].as_str().unwrap();
        uuid::Uuid::parse_str(request_id).unwrap();
        assert_eq!(body["amount"], "0.000000000001");
        assert!(body.get("amount_credits").is_none());
        if proactive {
            assert_eq!(request_id, GRANT_REQUEST_ID);
        }
        let diagnostics = grant_diagnostics(&out);
        assert_eq!(diagnostics[0]["request_id"], request_id);
        if final_status == 200 {
            assert_eq!(json(&out)["request_id"], request_id);
        } else {
            assert_eq!(diagnostics[1]["request_id"], request_id);
            assert_eq!(out.status.code(), Some(1));
        }
        server.verify().await;
    }
}

#[tokio::test]
async fn grant_list_and_revoke_tables_show_exact_balances_limits_and_revoke_state() {
    let server = MockServer::start().await;
    let list: Value = serde_json::from_str(include_str!("fixtures/billing/grants.json")).unwrap();
    let mut grant = list["grants"][0].clone();
    grant
        .as_object_mut()
        .unwrap()
        .remove("recipient_billing_enabled");
    Mock::given(method("GET"))
        .and(path("/api/v1/admin/credits/grants"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&list))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/v1/admin/credits/grants/{ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(&grant))
        .expect(2)
        .mount(&server)
        .await;
    for action in ["list", "revoke"] {
        let mut args = vec!["admin", "credits", "grant", action];
        if action == "revoke" {
            args.push(ID);
        }
        let out = run(&server, &args).await;
        assert!(out.status.success(), "{}", output_text(&out));
        assert!(out.stdout.is_empty());
        let table = String::from_utf8_lossy(&out.stderr);
        for text in [
            "Amount",
            "Remaining",
            "Reserved",
            "Held",
            "Reclaimed",
            "Status",
            "Activation",
            "Revoke state",
            "5.123456789012",
            "4.123456789012",
            "0.000000000001",
            "0.000000000002",
            "requested 2026-10-20T00:01:00Z",
            "Service limits",
            "Pilot ceiling",
            "2.123456789012",
        ] {
            assert!(table.contains(text), "Missing {text}: {table}");
        }
        if action == "list" {
            for text in ["Total: 2", "3.987654321098", "revoked", "complete"] {
                assert!(table.contains(text), "Missing {text}: {table}");
            }
        }
        args.extend(["--output", "json"]);
        let out = run(&server, &args).await;
        assert!(out.status.success(), "{}", output_text(&out));
        assert_eq!(
            json(&out),
            if action == "list" {
                list.clone()
            } else {
                grant.clone()
            }
        );
    }
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
        json!({
            "services": [
                {
                    "slug": "telnyx-platform",
                    "billing": billing["billing"]
                }
            ]
        })
        .to_string(),
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
        let respond = move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let id = body["id"].clone();
            match body["method"].as_str().unwrap() {
                "initialize" => ResponseTemplate::new(200)
                    .insert_header("Mcp-Session-Id", "s")
                    .set_body_json(json!({
                        "jsonrpc":"2.0", "id":id,
                        "result":{"protocolVersion":"2025-06-18", "capabilities":{"tools":{}}}
                    })),
                "notifications/initialized" => ResponseTemplate::new(202),
                "tools/call" => {
                    let text = if pending {
                        format!(
                            concat!(
                                "Managed execution {} is pending usage verification; ",
                                "inspect its status and do not retry automatically"
                            ),
                            ID
                        )
                    } else {
                        "Rejected".into()
                    };
                    let message = json!({
                        "jsonrpc":"2.0", "id":id,
                        "result":{"isError":true, "content":[{"type":"text", "text":text}]}
                    });
                    ResponseTemplate::new(200)
                        .set_body_raw(format!("data: {message}\n\n"), "text/event-stream")
                }
                _ => panic!("unexpected method"),
            }
        };
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(respond)
            .expect(3)
            .mount(&server)
            .await;
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
async fn wallet_owner_scope_uses_post_without_provision_in_binary() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/billing/wallet"))
        .and(wiremock::matchers::body_json(json!({"owner_id":ID})))
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
    get(
        &server,
        "/admin/usage",
        json!({
            "totals": {},
            "by_service": [
                {
                    "service_slug": "telnyx-platform",
                    "quantities": {
                        "input_tokens": 91,
                        "output_tokens": 5,
                        "cache_read_tokens": 10
                    },
                    "gross_cost": "0.123456789012",
                    "wallet_cost": "0.000000000001",
                    "grant_cost": "0.000000000002",
                    "allowance_cost": "0.000000000003",
                    "by_credential_class": [
                        {
                            "credential_class": "nyxid_managed_master",
                            "quantities": {
                                "input_tokens": 91,
                                "output_tokens": 5,
                                "cache_read_tokens": 10
                            },
                            "gross_cost": "0.987654321098",
                            "wallet_cost": "0.000000000004",
                            "grant_cost": "0.000000000005",
                            "allowance_cost": "0.000000000006"
                        }
                    ]
                }
            ]
        }),
    )
    .await;
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

async fn run_saved_session(
    server: &MockServer,
    args: &[&str],
    token: &str,
) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join(".nyxid/profiles/refresh-test");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("access_token"), token).unwrap();
    std::fs::write(profile.join("refresh_token"), "local-refresh-fixture").unwrap();
    Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(args)
        .args(["--profile", "refresh-test", "--output", "json"])
        .env("HOME", home.path())
        .env("NYXID_URL", server.uri())
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
        .unwrap()
}

#[tokio::test]
async fn read_commands_refresh_proactively_and_replay_401_with_saved_human_session() {
    let cases = [
        (vec!["billing", "usage"], "GET", "/api/v1/billing/usage"),
        (vec!["billing", "wallet"], "GET", "/api/v1/billing/wallet"),
        (
            vec!["billing", "verify-topup-flow"],
            "GET",
            "/api/v1/billing/wallet",
        ),
        (vec!["mcp", "tools"], "POST", "/mcp"),
        (
            vec!["mcp", "tools", "--service", "test"],
            "GET",
            "/api/v1/mcp/config",
        ),
        (
            vec!["proxy", "request", "test", "ping"],
            "GET",
            "/api/v1/proxy/s/test/ping",
        ),
        (
            vec!["proxy", "request", "test", "ping", "--method", "HEAD"],
            "HEAD",
            "/api/v1/proxy/s/test/ping",
        ),
    ];
    for (args, verb, route) in cases {
        for proactive in [false, true] {
            let server = MockServer::start().await;
            let refreshed_session = json!({
                "access_token": "fresh-session-fixture",
                "refresh_token": "rotated-refresh-fixture"
            });
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/refresh"))
                .respond_with(ResponseTemplate::new(200).set_body_json(refreshed_session))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method(verb))
                .and(path(route))
                .and(wiremock::matchers::header(
                    "authorization",
                    "Bearer stale-session-fixture",
                ))
                .respond_with(ResponseTemplate::new(401))
                .expect(if proactive { 0 } else { 1 })
                .mount(&server)
                .await;
            Mock::given(method(verb))
                .and(path(route))
                .and(wiremock::matchers::header(
                    "authorization",
                    "Bearer fresh-session-fixture",
                ))
                .respond_with(
                    ResponseTemplate::new(422).set_body_string("read reached with refreshed token"),
                )
                .expect(1)
                .mount(&server)
                .await;
            let token = if proactive {
                "eyJhbGciOiJub25lIn0.eyJleHAiOjF9.signature"
            } else {
                "stale-session-fixture"
            };
            let out = run_saved_session(&server, &args, token).await;
            assert_eq!(
                out.status.code(),
                Some(1),
                "{args:?}: {}",
                output_text(&out)
            );
            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests[usize::from(!proactive)].url.path(),
                "/api/v1/auth/refresh",
                "{args:?}"
            );
            server.verify().await;
        }
    }
}

#[tokio::test]
async fn non_idempotent_commands_refresh_before_work_and_never_replay_a_401_write() {
    for args in [
        vec![
            "admin",
            "billing",
            "config",
            "set",
            "--default-overdraft-cap-credits",
            "5",
        ],
        vec!["admin", "credits", "grant", "revoke", ID],
        vec!["proxy", "request", "test", "ping", "--method", "POST"],
        vec!["mcp", "call", "test__call", "--args", "{}"],
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token":"fresh-session-fixture", "refresh_token":"rotated-refresh-fixture"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(|request: &Request| request.url.path() != "/api/v1/auth/refresh")
            .respond_with(|request: &Request| {
                if request.url.path() == "/mcp" {
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    match body["method"].as_str().unwrap() {
                        "initialize" => {
                            return ResponseTemplate::new(200)
                                .insert_header("Mcp-Session-Id", "session")
                                .set_body_json(
                                    json!({"jsonrpc":"2.0", "id":body["id"], "result":{
                                        "protocolVersion":"2025-06-18", "capabilities":{"tools":{}}
                                    }}),
                                );
                        }
                        "notifications/initialized" => return ResponseTemplate::new(202),
                        _ => {}
                    }
                }
                ResponseTemplate::new(401).set_body_string("write refused")
            })
            .mount(&server)
            .await;
        let out =
            run_saved_session(&server, &args, "eyJhbGciOiJub25lIn0.eyJleHAiOjF9.signature").await;
        assert_eq!(
            out.status.code(),
            Some(1),
            "{args:?}: {}",
            output_text(&out)
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].url.path(), "/api/v1/auth/refresh");
        assert_eq!(
            requests.len(),
            if args[0] == "mcp" { 4 } else { 2 },
            "{args:?}"
        );
    }
}

#[tokio::test]
async fn plain_execution_show_pending_is_success_but_wait_timeout_exits_two() {
    let server = MockServer::start().await;
    get(
        &server,
        &format!("/billing/executions/{ID}"),
        json!({"id":ID,"state":"pending"}),
    )
    .await;
    let out = run(
        &server,
        &["billing", "executions", "show", ID, "--output", "json"],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
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
}

#[tokio::test]
async fn mcp_timeout_returns_uncertain_bounds_and_exit_two_without_replay() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let result = match body["method"].as_str().unwrap() {
                "initialize" => {
                    json!({
                        "protocolVersion": "2025-06-18",
                         "capabilities": {
                            "tools": {}
                        }
                    })
                }
                "notifications/initialized" => return ResponseTemplate::new(202),
                "tools/call" => {
                    return ResponseTemplate::new(200)
                        .set_delay(std::time::Duration::from_secs(3))
                        .set_body_json(json!({
                            "jsonrpc":"2.0", "id":body["id"], "result":{"content":[]}
                        }));
                }
                _ => panic!("unexpected method"),
            };
            ResponseTemplate::new(200)
                .insert_header("Mcp-Session-Id", "session")
                .set_body_json(json!({"jsonrpc":"2.0", "id":body["id"], "result":result}))
        })
        .expect(3)
        .mount(&server)
        .await;
    let out = run(
        &server,
        &[
            "mcp",
            "call",
            "test__call",
            "--args",
            "{}",
            "--timeout-secs",
            "1",
            "--output",
            "json",
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(2), "{}", output_text(&out));
    let value = json(&out);
    assert_eq!(value["error"], "managed_outcome_uncertain");
    for key in ["started_at", "finished_at"] {
        chrono::DateTime::parse_from_rfc3339(value[key].as_str().unwrap()).unwrap();
    }
    assert!(value.get("execution_id").is_none());
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("before retrying")
    );
}

#[tokio::test]
async fn full_telnyx_template_apply_twice_leaves_service_and_operations_unchanged() {
    // Shapes: handlers/services.rs ServiceResponse and handlers/endpoints.rs
    // EndpointListResponse / EndpointResponse. Operations are separate from services.
    let server = MockServer::start().await;
    let mut service: Value = serde_json::from_str(include_str!(
        "fixtures/billing/telnyx-template-service.json"
    ))
    .unwrap();
    let mut endpoints: Value = serde_json::from_str(include_str!(
        "fixtures/billing/telnyx-template-endpoints.json"
    ))
    .unwrap();
    service["name"] = json!("Old label");
    endpoints["endpoints"][0]["description"] = json!("Old description");
    let state = std::sync::Arc::new(std::sync::Mutex::new((service, endpoints)));
    let mock_state = state.clone();
    Mock::given(|_: &Request| true)
        .respond_with(move |request: &Request| {
            let mut state = mock_state.lock().unwrap();
            let route = request.url.path();
            let value = match (request.method.as_str(), route) {
                ("GET", "/api/v1/providers") => json!({"providers":[]}),
                ("GET", "/api/v1/services") => json!({"services":[state.0]}),
                ("GET", route) if route.ends_with("/endpoints") => state.1.clone(),
                ("PUT", route) => {
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    let row = if route.contains("/endpoints/") {
                        &mut state.1["endpoints"][0]
                    } else {
                        &mut state.0
                    };
                    for (key, value) in body.as_object().unwrap() {
                        row[key] = value.clone();
                    }
                    row.clone()
                }
                _ => panic!("Unexpected request: {} {route}", request.method),
            };
            ResponseTemplate::new(200).set_body_json(value)
        })
        .mount(&server)
        .await;
    let manifest = tempfile::NamedTempFile::new().unwrap();
    let out = run(
        &server,
        &[
            "admin",
            "catalog",
            "add",
            "--template",
            "telnyx-platform",
            "--set",
            "model=org/model:1",
            "--set",
            "max_output_tokens=1024",
            "--save",
            manifest.path().to_str().unwrap(),
            "--dry-run",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    for expected in ["updated", "unchanged"] {
        let out = run(
            &server,
            &[
                "admin",
                "catalog",
                "apply",
                "--file",
                manifest.path().to_str().unwrap(),
                "--output",
                "json",
            ],
        )
        .await;
        assert!(out.status.success(), "{}", output_text(&out));
        let value = json(&out);
        let results = value["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        for kind in ["service", "operation"] {
            let row = results.iter().find(|row| row["kind"] == kind).unwrap();
            assert_eq!(row["action"], expected, "{value}");
        }
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "PUT")
            .count(),
        2
    );
}

#[tokio::test]
async fn catalog_access_change_preflight_refuses_before_writes_and_allows_flag_or_dry_run() {
    for (extra, allowed) in [
        (None, false),
        (Some("--allow-access-change"), true),
        (Some("--dry-run"), true),
    ] {
        let server = MockServer::start().await;
        get(&server, "/providers", json!({"providers":[]})).await;
        get(
            &server,
            "/services",
            json!({"services":[{
                "id":ID,"slug":"telnyx-platform","is_active":true,
                "platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[ID]}
            }]}),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/services/{ID}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ID})))
            .expect(usize::from(extra == Some("--allow-access-change")) as u64)
            .mount(&server)
            .await;
        let manifest = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            manifest.path(),
            json!({
                "services": [
                    {
                        "slug": "telnyx-platform",
                        "platform_key": {
                            "enabled": false,
                            "audience": "restricted",
                            "allowed_owner_ids": []
                        }
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let mut args = vec![
            "admin",
            "catalog",
            "apply",
            "--file",
            manifest.path().to_str().unwrap(),
            "--output",
            "json",
        ];
        if let Some(extra) = extra {
            args.push(extra);
        }
        let out = run(&server, &args).await;
        assert_eq!(out.status.success(), allowed, "{}", output_text(&out));
        assert!(String::from_utf8_lossy(&out.stderr).contains("Warning:"));
        if !allowed {
            assert!(output_text(&out).contains("--allow-access-change"));
        }
    }
}

#[tokio::test]
async fn catalog_apply_refreshes_saved_session_before_the_first_write() {
    for proactive in [false, true] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token":"fresh-session-fixture", "refresh_token":"rotated-refresh-fixture"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/providers"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer stale-session-fixture",
            ))
            .respond_with(ResponseTemplate::new(401))
            .expect(if proactive { 0 } else { 1 })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/providers"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer fresh-session-fixture",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"providers":[]})))
            .expect(1)
            .mount(&server)
            .await;
        get(
            &server,
            "/services",
            json!({"services":[{"id":ID,"slug":"test","name":"old"}]}),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/services/{ID}")))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer fresh-session-fixture",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ID})))
            .expect(1)
            .mount(&server)
            .await;
        let manifest = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            manifest.path(),
            json!({"services":[{"slug":"test","name":"new"}]}).to_string(),
        )
        .unwrap();
        let token = if proactive {
            "eyJhbGciOiJub25lIn0.eyJleHAiOjF9.signature"
        } else {
            "stale-session-fixture"
        };
        let out = run_saved_session(
            &server,
            &[
                "admin",
                "catalog",
                "apply",
                "--file",
                manifest.path().to_str().unwrap(),
            ],
            token,
        )
        .await;
        assert!(out.status.success(), "{}", output_text(&out));
        let requests = server.received_requests().await.unwrap();
        let refresh = requests
            .iter()
            .position(|request| request.url.path().ends_with("/auth/refresh"))
            .unwrap();
        let write = requests
            .iter()
            .position(|request| request.method == "PUT")
            .unwrap();
        assert!(refresh < write);
    }
}

#[tokio::test]
async fn mcp_success_envelope_has_rfc3339_bounds_and_initialize_404_has_http_error() {
    for missing in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                if missing {
                    return ResponseTemplate::new(404).set_body_string("No MCP route");
                }
                let result = match body["method"].as_str().unwrap() {
                    "initialize" => {
                        json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}})
                    }
                    "notifications/initialized" => return ResponseTemplate::new(202),
                    "tools/call" => {
                        json!({"content":[{"type":"text","text":"OK"}],"isError":false})
                    }
                    _ => panic!("unexpected method"),
                };
                ResponseTemplate::new(200)
                    .insert_header("Mcp-Session-Id", "session")
                    .set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
            })
            .expect(if missing { 1 } else { 3 })
            .mount(&server)
            .await;
        let out = run(
            &server,
            &[
                "mcp",
                "call",
                "test__call",
                "--args",
                "{}",
                "--envelope",
                "--output",
                "json",
            ],
        )
        .await;
        if missing {
            assert_eq!(out.status.code(), Some(1));
            let value: Value = serde_json::from_slice(&out.stderr).unwrap();
            assert_eq!(value["error"], "http_error");
            assert_eq!(value["path"], "/mcp");
            assert_eq!(value["status"], 404);
            assert_eq!(value["body"], "No MCP route");
        } else {
            assert!(out.status.success(), "{}", output_text(&out));
            let value = json(&out);
            let start = chrono::DateTime::parse_from_rfc3339(value["started_at"].as_str().unwrap())
                .unwrap();
            let end = chrono::DateTime::parse_from_rfc3339(value["finished_at"].as_str().unwrap())
                .unwrap();
            assert!(start <= end);
            assert!(value.get("execution_id").is_none());
        }
    }
}

#[tokio::test]
async fn grant_list_pagination_exposes_total_and_rejects_invalid_bounds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/admin/credits/grants"))
        .and(query_param("page", "2"))
        .and(query_param("per_page", "500"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"grants":[],"total":700,"page":2,"per_page":500})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        &server,
        &[
            "admin",
            "credits",
            "grant",
            "list",
            "--page",
            "2",
            "--per-page",
            "500",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    assert_eq!(json(&out)["total"], 700);
    for (flag, bad) in [("--page", "0"), ("--per-page", "0"), ("--per-page", "501")] {
        let out = run(&server, &["admin", "credits", "grant", "list", flag, bad]).await;
        assert!(!out.status.success());
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn billing_service_and_grant_revoke_writes_do_not_refresh_or_replay_after_401() {
    let patch = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(patch.path(), r#"{"platform_billable":true}"#).unwrap();
    let verified = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    for args in [
        vec!["admin", "billing", "service", "resync", ID],
        vec![
            "admin",
            "billing",
            "service",
            "update",
            ID,
            "--file",
            patch.path().to_str().unwrap(),
        ],
        vec![
            "admin",
            "billing",
            "service",
            "set-managed",
            ID,
            "--model",
            "org/model:1",
            "--max-input-tokens",
            "8192",
            "--max-output-tokens",
            "1024",
            "--max-request-bytes",
            "65536",
            "--max-response-bytes",
            "262144",
            "--tariff-version",
            "fixture",
            "--tariff-source",
            "https://telnyx.com/pricing",
            "--tariff-verified-at",
            &verified,
            "--input-price",
            "0.000001",
            "--output-price",
            "0.000002",
            "--cache-read-price",
            "0",
        ],
        vec!["admin", "credits", "grant", "revoke", ID],
    ] {
        let server = MockServer::start().await;
        get(&server, &format!("/services/{ID}"), raw_service()).await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(|request: &Request| request.method != "GET")
            .respond_with(ResponseTemplate::new(401).set_body_string("write refused"))
            .expect(1)
            .mount(&server)
            .await;
        let out = run_saved_session(&server, &args, "unexpired-session-fixture").await;
        assert_eq!(
            out.status.code(),
            Some(1),
            "{args:?}: {}",
            output_text(&out)
        );
        server.verify().await;
    }
}

#[tokio::test]
async fn explicit_and_environment_human_credentials_override_saved_agent_profile() {
    for source in ["explicit", "default_env", "named_env"] {
        let server = MockServer::start().await;
        get(
            &server,
            "/admin/billing/config",
            json!({"billing_enabled":false}),
        )
        .await;
        let home = tempfile::tempdir().unwrap();
        let profile = home.path().join(".nyxid/profiles/agent-fixture");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("auth_kind"), "agent_key").unwrap();
        std::fs::write(profile.join("token"), "nyxid_ag_profile-fixture").unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
        command
            .args([
                "admin",
                "billing",
                "config",
                "show",
                "--profile",
                "agent-fixture",
                "--output",
                "json",
            ])
            .env("HOME", home.path())
            .env("NYXID_URL", server.uri())
            .env("CI", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env_remove("NYXID_API_KEY")
            .env_remove("NYXID_PROFILE")
            .env_remove("NYXID_ACCESS_TOKEN");
        match source {
            // A dummy fixture token is intentionally nonsecret. Real credentials never belong in argv.
            "explicit" => {
                command.args(["--access-token", "human-override-fixture"]);
            }
            "default_env" => {
                command.env("NYXID_ACCESS_TOKEN", "human-override-fixture");
            }
            "named_env" => {
                command
                    .args(["--access-token-env", "NYXID_BILLING_TOKEN"])
                    .env("NYXID_BILLING_TOKEN", "human-override-fixture");
            }
            _ => unreachable!(),
        }
        let out = command
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .unwrap();
        assert!(out.status.success(), "{source}: {}", output_text(&out));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn verify_topup_flow_refreshes_during_polling_without_repeating_checkout() {
    let server = MockServer::start().await;
    let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/api/v1/billing/wallet"))
        .respond_with(move |request: &Request| {
            if reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                return ResponseTemplate::new(401);
            }
            let mut wallet: Value =
                serde_json::from_str(include_str!("fixtures/billing/wallet.json")).unwrap();
            if request.headers["authorization"] == "Bearer fresh-session-fixture" {
                wallet["balance_credits"] = json!(1);
            }
            ResponseTemplate::new(200).set_body_json(wallet)
        })
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/api/v1/billing/topup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "owner_id":ID,"amount_credits":1,"idempotency_key":"fixture",
            "checkout_url":"https://checkout.stripe.test/fixture","payment_provider":"stripe",
            "lago_wallet_transaction_id":"fixture","lago_invoice_id":null,"status":"checkout_created","reused":false
        }))).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token":"fresh-session-fixture","refresh_token":"rotated-refresh-fixture"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let out = run_saved_session(
        &server,
        &[
            "billing",
            "verify-topup-flow",
            "--amount-credits",
            "1",
            "--timeout-secs",
            "5",
            "--poll-interval-secs",
            "1",
        ],
        "stale-session-fixture",
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    assert_eq!(json(&out)["verified"], true);
}

#[tokio::test]
async fn telnyx_template_create_uses_only_backend_create_request_fields() {
    let server = MockServer::start().await;
    get(&server, "/providers", json!({"providers":[]})).await;
    get(&server, "/services", json!({"services":[]})).await;
    get(
        &server,
        &format!("/services/{ID}/endpoints"),
        json!({"endpoints":[]}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/services"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            for key in [
                "identity_propagation_mode",
                "inject_delegation_token",
                "requires_user_credential",
            ] {
                assert!(
                    body.get(key).is_none(),
                    "CreateServiceRequest rejects {key}"
                );
            }
            assert_eq!(body["service_category"], "internal");
            assert_eq!(
                body["proxy_operation_policy"]["rules"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            ResponseTemplate::new(200).set_body_json(
                serde_json::from_str::<Value>(include_str!(
                    "fixtures/billing/telnyx-template-service.json"
                ))
                .unwrap(),
            )
        })
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/services/{ID}/endpoints")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":ID})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let manifest = tempfile::NamedTempFile::new().unwrap();
    let out = run(
        &server,
        &[
            "admin",
            "catalog",
            "add",
            "--template",
            "telnyx-platform",
            "--set",
            "model=org/model:1",
            "--set",
            "max_output_tokens=1024",
            "--save",
            manifest.path().to_str().unwrap(),
            "--dry-run",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    let out = run(
        &server,
        &[
            "admin",
            "catalog",
            "apply",
            "--file",
            manifest.path().to_str().unwrap(),
            "--output",
            "json",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", output_text(&out));
    assert!(
        json(&out)["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["action"] == "created")
    );
}

#[tokio::test]
async fn mcp_timeout_also_bounds_a_stalled_sse_response_body() {
    use axum::{Json, Router, body::Body, http::Response, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = calls.clone();
    let app = Router::new().route(
        "/mcp",
        post(move |Json(body): Json<Value>| {
            let calls = handler_calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let result = match body["method"].as_str().unwrap() {
                    "initialize" => {
                        json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}})
                    }
                    "notifications/initialized" => {
                        return Response::builder().status(202).body(Body::empty()).unwrap();
                    }
                    "tools/call" => {
                        let stream = futures::stream::once(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                            Ok::<_, std::convert::Infallible>(format!(
                                "data: {}\n\n",
                                json!({
                                    "jsonrpc":"2.0","id":body["id"],"result":{"content":[]}
                                })
                            ))
                        });
                        return Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(Body::from_stream(stream))
                            .unwrap();
                    }
                    _ => panic!("unexpected method"),
                };
                Response::builder()
                    .header("content-type", "application/json")
                    .header("Mcp-Session-Id", "session")
                    .body(Body::from(
                        json!({
                            "jsonrpc":"2.0","id":body["id"],"result":result
                        })
                        .to_string(),
                    ))
                    .unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args([
            "mcp",
            "call",
            "test__call",
            "--args",
            "{}",
            "--timeout-secs",
            "1",
            "--output",
            "json",
        ])
        .env("NYXID_URL", uri)
        .env("NYXID_ACCESS_TOKEN", SENTINEL)
        .env("HOME", home.path())
        .env("CI", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_PROFILE")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", output_text(&out));
    assert_eq!(json(&out)["error"], "managed_outcome_uncertain");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    server.abort();
}

#[tokio::test]
async fn catalog_create_uses_server_truth_for_derived_credential_requirements() {
    for requires in [false, true] {
        let server = MockServer::start().await;
        get(&server, "/providers", json!({"providers":[]})).await;
        get(&server, "/services", json!({"services":[]})).await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services"))
            .respond_with(|request: &Request| {
                let mut body: Value = serde_json::from_slice(&request.body).unwrap();
                for field in [
                    "requires_user_credential",
                    "identity_propagation_mode",
                    "inject_delegation_token",
                ] {
                    assert!(
                        body.get(field).is_none(),
                        "{field} is not accepted on create"
                    );
                }
                // The client must compare this response rather than derive its
                // own answer from auth_method or service_category.
                body["id"] = json!(ID);
                body["requires_user_credential"] = json!(false);
                body["identity_propagation_mode"] = json!("none");
                body["inject_delegation_token"] = json!(false);
                ResponseTemplate::new(200).set_body_json(body)
            })
            .expect(1)
            .mount(&server)
            .await;
        let manifest = tempfile::NamedTempFile::new().unwrap();
        let desired = json!({"services":[{
            "slug":"server-truth", "name":"Server truth", "auth_method":"bearer",
            "service_category":"connection", "base_url":"https://example.test",
            "requires_user_credential":requires,
            "identity_propagation_mode":"none", "inject_delegation_token":false
        }]});
        std::fs::write(manifest.path(), desired.to_string()).unwrap();
        let out = run(
            &server,
            &[
                "admin",
                "catalog",
                "apply",
                "--file",
                manifest.path().to_str().unwrap(),
                "--output",
                "json",
            ],
        )
        .await;
        assert_eq!(
            out.status.code(),
            Some(i32::from(requires)),
            "{}",
            output_text(&out)
        );
        let result = json(&out);
        let row = &result["results"][0];
        assert_eq!(row["action"], "created");
        assert_eq!(row["id"], ID);
        if requires {
            let error = row["error"].as_str().unwrap();
            assert!(error.contains("requires_user_credential"));
            assert!(error.contains("cannot be set directly"));
        } else {
            assert!(row.get("error").is_none());
        }
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method != "PUT")
        );
    }
}

#[tokio::test]
async fn catalog_updates_check_derived_requirement_against_the_update_response() {
    for server_requires in [false, true] {
        let server = MockServer::start().await;
        get(&server, "/providers", json!({"providers":[]})).await;
        get(
            &server,
            "/services",
            json!({"services":[{
                "id":ID, "slug":"server-truth", "is_active":true,
                "service_category":"connection", "requires_user_credential":true
            }]}),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/services/{ID}")))
            .and(wiremock::matchers::body_json(
                json!({"service_category":"internal"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id":ID, "service_category":"internal", "requires_user_credential":server_requires
            })))
            .expect(1)
            .mount(&server)
            .await;
        let manifest = tempfile::NamedTempFile::new().unwrap();
        let desired = json!({"services":[{
            "slug":"server-truth", "service_category":"internal", "requires_user_credential":false
        }]});
        std::fs::write(manifest.path(), desired.to_string()).unwrap();
        let out = run(
            &server,
            &[
                "admin",
                "catalog",
                "apply",
                "--file",
                manifest.path().to_str().unwrap(),
                "--output",
                "json",
            ],
        )
        .await;
        assert_eq!(
            out.status.code(),
            Some(i32::from(server_requires)),
            "{}",
            output_text(&out)
        );
        let result = json(&out);
        let row = &result["results"][0];
        assert_eq!(row["action"], "updated");
        assert_eq!(row["id"], ID);
        assert_eq!(row.get("error").is_some(), server_requires);
        if server_requires {
            assert!(
                row["error"]
                    .as_str()
                    .unwrap()
                    .contains("requires_user_credential")
            );
        }
    }
}

#[tokio::test]
async fn partial_service_creation_preserves_id_and_rerun_updates_without_recreating() {
    for format in ["json", "table"] {
        let server = MockServer::start().await;
        let state = std::sync::Arc::new(std::sync::Mutex::new((None::<Value>, 0)));
        let mock_state = state.clone();
        Mock::given(|_: &Request| true)
            .respond_with(move |request: &Request| {
                let mut state = mock_state.lock().unwrap();
                let route = request.url.path();
                let value = match (request.method.as_str(), route) {
                    ("GET", "/api/v1/providers") => json!({"providers":[]}),
                    ("GET", "/api/v1/services") => {
                        json!({"services":state.0.iter().collect::<Vec<_>>()})
                    }
                    ("POST", "/api/v1/services") => {
                        assert!(state.0.is_none(), "a rerun must not recreate the service");
                        let mut body: Value = serde_json::from_slice(&request.body).unwrap();
                        for field in [
                            "requires_user_credential",
                            "identity_propagation_mode",
                            "inject_delegation_token",
                        ] {
                            assert!(body.get(field).is_none());
                        }
                        body["id"] = json!(ID);
                        body["is_active"] = json!(true);
                        body["requires_user_credential"] = json!(false);
                        body["identity_propagation_mode"] = json!("none");
                        body["inject_delegation_token"] = json!(false);
                        state.0 = Some(body.clone());
                        body
                    }
                    ("PUT", route) if route == format!("/api/v1/services/{ID}") => {
                        let body: Value = serde_json::from_slice(&request.body).unwrap();
                        assert_eq!(body, json!({
                            "identity_propagation_mode":"headers", "inject_delegation_token":true
                        }));
                        state.1 += 1;
                        if state.1 == 1 {
                            return ResponseTemplate::new(503).set_body_string("follow-up unavailable");
                        }
                        let created = state.0.as_mut().unwrap();
                        for (key, value) in body.as_object().unwrap() {
                            created[key] = value.clone();
                        }
                        created.clone()
                    }
                    _ => panic!("Unexpected request: {} {route}", request.method),
                };
                ResponseTemplate::new(200).set_body_json(value)
            })
            .mount(&server)
            .await;
        let manifest = tempfile::NamedTempFile::new().unwrap();
        let desired = json!({"services":[{
            "slug":"partial-create", "name":"Partial create", "auth_method":"bearer",
            "service_category":"internal", "base_url":"https://example.test",
            "requires_user_credential":false,
            "identity_propagation_mode":"headers", "inject_delegation_token":true
        }]});
        std::fs::write(manifest.path(), desired.to_string()).unwrap();
        let args = [
            "admin",
            "catalog",
            "apply",
            "--file",
            manifest.path().to_str().unwrap(),
            "--output",
            format,
        ];
        let out = run(&server, &args).await;
        assert_eq!(out.status.code(), Some(1), "{}", output_text(&out));
        let text = output_text(&out);
        for expected in [
            ID,
            "created",
            "follow-up update of",
            "identity_propagation_mode",
            "inject_delegation_token",
            "failed",
            "rerun apply",
        ] {
            assert!(text.contains(expected), "Missing {expected}: {text}");
        }
        if format == "json" {
            let value = json(&out);
            assert_eq!(value["results"][0]["action"], "created");
            assert_eq!(value["results"][0]["id"], ID);
        }
        let out = run(&server, &args).await;
        assert!(out.status.success(), "{}", output_text(&out));
        assert!(output_text(&out).contains("updated"));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 1);
        assert_eq!(requests.iter().filter(|r| r.method == "PUT").count(), 2);
    }
}

#[tokio::test]
async fn idempotent_wallet_and_checkout_posts_refresh_and_replay_the_same_body_once() {
    for args in [
        vec!["billing", "wallet", "--provision"],
        vec!["billing", "wallet", "--owner-id", ID],
        vec!["billing", "topup", "--amount-credits", "1"],
        vec![
            "billing",
            "topup",
            "--amount-credits",
            "1",
            "--idempotency-key",
            "provided-key",
        ],
        vec!["billing", "verify-topup-flow", "--amount-credits", "1"],
        vec![
            "billing",
            "verify-topup-flow",
            "--amount-credits",
            "1",
            "--idempotency-key",
            "provided-key",
        ],
    ] {
        let server = MockServer::start().await;
        let wallet: Value =
            serde_json::from_str(include_str!("fixtures/billing/wallet.json")).unwrap();
        let route = if args[1] == "wallet" {
            "/api/v1/billing/wallet"
        } else {
            "/api/v1/billing/topup"
        };
        if args[1] == "verify-topup-flow" {
            let reads = std::sync::atomic::AtomicUsize::new(0);
            let wallet = wallet.clone();
            Mock::given(method("GET"))
                .and(path("/api/v1/billing/wallet"))
                .respond_with(move |_: &Request| {
                    let balance = reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut wallet = wallet.clone();
                    wallet["balance_credits"] = json!(balance);
                    ResponseTemplate::new(200).set_body_json(wallet)
                })
                .expect(2)
                .mount(&server)
                .await;
        }
        Mock::given(method("POST"))
            .and(path(route))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer stale-session-fixture",
            ))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(route))
            .and(wiremock::matchers::header("authorization", "Bearer fresh-session-fixture"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let response = if request.url.path().ends_with("/wallet") {
                    wallet.clone()
                } else {
                    json!({
                        "owner_id":ID, "amount_credits":1, "idempotency_key":body["idempotency_key"],
                        "checkout_url":"https://checkout.stripe.test/fixture", "payment_provider":"stripe",
                        "lago_wallet_transaction_id":"fixture", "lago_invoice_id":null,
                        "status":"checkout_created", "reused":true
                    })
                };
                ResponseTemplate::new(200).set_body_json(response)
            })
            .expect(1)
            .mount(&server)
            .await;
        let refreshed = json!({
            "access_token":"fresh-session-fixture", "refresh_token":"rotated-refresh-fixture"
        });
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(refreshed))
            .expect(1)
            .mount(&server)
            .await;
        let out = run_saved_session(&server, &args, "stale-session-fixture").await;
        assert!(out.status.success(), "{args:?}: {}", output_text(&out));
        let requests = server.received_requests().await.unwrap();
        let posts: Vec<_> = requests
            .iter()
            .filter(|r| r.method == "POST" && r.url.path() == route)
            .collect();
        assert_eq!(posts.len(), 2);
        assert_eq!(
            posts[0].body, posts[1].body,
            "replay must preserve the complete body"
        );
        if args[1] != "wallet" {
            let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
            let key = body["idempotency_key"].as_str().unwrap();
            assert!(!key.is_empty());
            if args.contains(&"provided-key") {
                assert_eq!(key, "provided-key");
            }
        }
        server.verify().await;
    }
}

#[tokio::test]
async fn mcp_timeout_before_call_dispatch_exits_one_without_uncertain_outcome() {
    for stalled_method in ["initialize", "notifications/initialized"] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let method = body["method"].as_str().unwrap();
                assert_ne!(
                    method, "tools/call",
                    "a timed-out handshake cannot execute a tool"
                );
                let response = if method == "notifications/initialized" {
                    ResponseTemplate::new(202)
                } else {
                    let result =
                        json!({"protocolVersion":"2025-06-18", "capabilities":{"tools":{}}});
                    ResponseTemplate::new(200)
                        .insert_header("Mcp-Session-Id", "session")
                        .set_body_json(json!({"jsonrpc":"2.0", "id":body["id"], "result":result}))
                };
                if method == stalled_method {
                    response.set_delay(std::time::Duration::from_secs(3))
                } else {
                    response
                }
            })
            .expect(if stalled_method == "initialize" { 1 } else { 2 })
            .mount(&server)
            .await;
        let out = run(
            &server,
            &[
                "mcp",
                "call",
                "test__call",
                "--args",
                "{}",
                "--timeout-secs",
                "1",
                "--output",
                "json",
            ],
        )
        .await;
        assert_eq!(out.status.code(), Some(1), "{}", output_text(&out));
        let text = output_text(&out);
        assert!(text.contains("before tools/call was dispatched; nothing was executed"));
        assert!(!text.contains("managed_outcome_uncertain"));
        assert!(!text.contains("may have completed and charged"));
    }
}

#[tokio::test]
async fn catalog_add_live_access_refusal_names_the_apply_command_and_its_flag() {
    let server = MockServer::start().await;
    let mut service = raw_service();
    service["platform_key"] =
        json!({"enabled":true, "audience":"restricted", "allowed_owner_ids":[ID]});
    get(&server, "/providers", json!({"providers":[]})).await;
    get(&server, "/services", json!({"services":[service]})).await;
    get(
        &server,
        &format!("/services/{ID}/endpoints"),
        json!({"endpoints":[]}),
    )
    .await;
    let out = run(
        &server,
        &[
            "admin",
            "catalog",
            "add",
            "--template",
            "telnyx-platform",
            "--set",
            "model=org/model:1",
            "--set",
            "max_output_tokens=1024",
            "--yes",
            "--skip-credentials",
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(1), "{}", output_text(&out));
    assert!(
        output_text(&out)
            .contains("use `nyxid admin catalog apply --allow-access-change --file <manifest>`")
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
}
