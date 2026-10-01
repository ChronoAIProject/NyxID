use super::*;
use crate::api::ApiError;
use crate::cli::{Cli, Commands};
use clap::Parser;
use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const POOL_ID: &str = "22222222-2222-4222-8222-222222222222";
const ORG_ID: &str = "11111111-1111-4111-8111-111111111111";

fn parse(server: &MockServer, arguments: &[&str]) -> PoolCommands {
    let mut args = vec!["nyxid".to_owned(), "pool".to_owned()];
    args.extend(arguments.iter().map(|arg| (*arg).to_owned()));
    args.extend([
        "--base-url".into(),
        server.uri(),
        "--access-token".into(),
        "test-access-token".into(),
        "--output".into(),
        "json".into(),
    ]);
    match Cli::try_parse_from(args)
        .expect("pool command should parse")
        .command
    {
        Commands::Pool { command } => command,
        _ => panic!("expected pool command"),
    }
}

fn custom_policy() -> Value {
    json!({
        "max_attempts":5,
        "per_attempt_timeout_ms":19000,
        "overall_deadline_ms":95000,
        "max_replay_body_bytes":2048,
        "retry_on":["connect_error"],
        "retry_ambiguous_dispatch":false,
        "future_setting":{"preserve":true},
        "cooldown":{"base_ms":2000,"max_ms":90000,"failures_to_open":4,"honor_retry_after":true,"future_setting":7}
    })
}

async fn snapshot(server: &MockServer, identifier: &str, current: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/service-pools/{identifier}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(current))
        .expect(1)
        .mount(server)
        .await;
}

async fn save(server: &MockServer, policy: Value, revision: u64) {
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/service-pools/{POOL_ID}")))
        .and(body_json(
            json!({"failover":policy,"expected_revision":revision}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":POOL_ID})))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn pool_issue_1680_sketch_parses_and_merges_canonical_retry_policy_without_replay_opt_in() {
    let server = MockServer::start().await;
    let mut policy = custom_policy();
    snapshot(
        &server,
        "my-llm",
        json!({"id":POOL_ID,"config_revision":17,"failover":policy}),
    )
    .await;
    policy["max_attempts"] = 3.into();
    policy["retry_on"] = json!([
        "http_429",
        "http_500",
        "http_502",
        "http_503",
        "http_504",
        "http_529",
        "timeout",
        "node_offline"
    ]);
    save(&server, policy, 17).await;
    run(parse(
        &server,
        &[
            "set-failover",
            "my-llm",
            "--retry-on",
            "429,5xx,timeout,node_offline",
            "--max-attempts",
            "3",
        ],
    ))
    .await
    .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn pool_inline_cooldown_edits_preserve_nested_fields_and_all_other_controls_are_available() {
    let server = MockServer::start().await;
    let mut policy = custom_policy();
    snapshot(
        &server,
        "my-llm",
        json!({"id":POOL_ID,"config_revision":7,"failover":policy}),
    )
    .await;
    policy["per_attempt_timeout_ms"] = 20000.into();
    policy["overall_deadline_ms"] = 40000.into();
    policy["max_replay_body_bytes"] = 4096.into();
    policy["retry_ambiguous_dispatch"] = true.into();
    policy["cooldown"]["base_ms"] = 3000.into();
    policy["cooldown"]["honor_retry_after"] = false.into();
    save(&server, policy, 7).await;
    run(parse(
        &server,
        &[
            "set-failover",
            "my-llm",
            "--per-attempt-timeout-ms",
            "20000",
            "--overall-deadline-ms",
            "40000",
            "--max-replay-body-bytes",
            "4096",
            "--retry-non-idempotent",
            "--cooldown-base-ms",
            "3000",
            "--honor-retry-after=false",
        ],
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn pool_retry_aliases_deduplicate_and_remaining_cooldown_flags_preserve_base() {
    let server = MockServer::start().await;
    let mut policy = custom_policy();
    snapshot(
        &server,
        "my-llm",
        json!({"id":POOL_ID,"config_revision":8,"failover":policy}),
    )
    .await;
    policy["retry_on"] = json!([
        "http_401",
        "http_403",
        "http_408",
        "http_429",
        "http_500",
        "http_502",
        "http_503",
        "http_504",
        "http_529",
        "connect_error",
        "node_offline",
        "transport_error",
        "timeout"
    ]);
    policy["cooldown"]["max_ms"] = 300000.into();
    policy["cooldown"]["failures_to_open"] = 2.into();
    policy["retry_ambiguous_dispatch"] = false.into();
    save(&server, policy, 8).await;
    run(parse(&server, &["set-failover","my-llm","--retry-on","401,403,408,429,500,502,503,504,529,5xx",
        "--retry-on","http_401,http_403,http_408,http_429,http_500,http_502,http_503,http_504,http_529,connect_error,node_offline,transport_error,timeout",
        "--retry-ambiguous-dispatch=false","--cooldown-max-ms","300000","--cooldown-failures-to-open","2"]))
        .await.unwrap();
}

#[tokio::test]
async fn pool_inline_null_or_missing_policy_uses_defaults_and_none_can_clear_retry_causes() {
    for missing in [true, false] {
        let server = MockServer::start().await;
        let mut current = json!({"id":POOL_ID});
        if !missing {
            current["config_revision"] = 0.into();
            current["failover"] = Value::Null;
        }
        snapshot(&server, "my-llm", current).await;
        save(
            &server,
            json!({"retry_on":[],"cooldown":{"failures_to_open":2}}),
            0,
        )
        .await;
        run(parse(
            &server,
            &[
                "set-failover",
                "my-llm",
                "--retry-on",
                "none",
                "--cooldown-failures-to-open",
                "2",
            ],
        ))
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn pool_inline_org_conflict_uses_one_policy_revision_snapshot_and_never_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/acme"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":ORG_ID,"slug":"acme","display_name":"Acme"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/service-pools")).and(query_param("org_id",ORG_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"pools":[{"id":POOL_ID,"slug":"my-llm","config_revision":999,"failover":{"max_attempts":1}}]})))
        .expect(1).mount(&server).await;
    let mut policy = custom_policy();
    snapshot(
        &server,
        POOL_ID,
        json!({"id":POOL_ID,"config_revision":12,"failover":policy}),
    )
    .await;
    policy["max_attempts"] = 3.into();
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/service-pools/{POOL_ID}")))
        .and(body_json(json!({"failover":policy,"expected_revision":12})))
        .respond_with(ResponseTemplate::new(409).set_body_json(
            json!({"error":"conflict","message":"Reload the pool","error_code":1009}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let error = run(parse(
        &server,
        &[
            "set-failover",
            "my-llm",
            "--org",
            "acme",
            "--max-attempts",
            "3",
        ],
    ))
    .await
    .expect_err("conflict must be returned");
    assert_eq!(
        error.downcast_ref::<ApiError>().unwrap().status(),
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
async fn pool_file_defaults_and_disable_replace_instead_of_merging_saved_policy() {
    for (flag, contents, expected) in [
        ("--defaults", "", Value::Null),
        ("--disable", "", json!({"max_attempts":1})),
        ("--file", r#"{"max_attempts":2}"#, json!({"max_attempts":2})),
        ("--file", "null", Value::Null),
    ] {
        let server = MockServer::start().await;
        snapshot(
            &server,
            "my-llm",
            json!({"id":POOL_ID,"config_revision":11,"failover":custom_policy()}),
        )
        .await;
        save(&server, expected, 11).await;
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), contents).unwrap();
        let mut args = vec!["set-failover", "my-llm", flag];
        if flag == "--file" {
            args.push(file.path().to_str().unwrap());
        }
        run(parse(&server, &args)).await.unwrap();
    }
}

#[test]
fn pool_failover_clap_rejects_missing_invalid_and_conflicting_modes() {
    for flags in [
        vec![],
        vec!["--retry-on", "501"],
        vec!["--retry-on", "http_505"],
        vec!["--retry-on", "unknown"],
        vec!["--retry-on", "429,,timeout"],
        vec!["--max-attempts", "0"],
        vec!["--max-attempts", "6"],
        vec!["--per-attempt-timeout-ms", "999"],
        vec!["--overall-deadline-ms", "600001"],
        vec!["--max-replay-body-bytes", "0"],
        vec!["--cooldown-max-ms", "3600001"],
        vec!["--file", "policy.json", "--max-attempts", "3"],
        vec!["--defaults", "--retry-ambiguous-dispatch=false"],
        vec!["--disable", "--honor-retry-after=false"],
        vec!["--disable", "--defaults"],
        vec!["--file", "policy.json", "--disable"],
    ] {
        let error = Cli::try_parse_from(
            ["nyxid", "pool", "set-failover", "my-llm"]
                .into_iter()
                .chain(flags.iter().copied()),
        )
        .err()
        .unwrap_or_else(|| panic!("invalid flags accepted: {flags:?}"));
        assert!(
            matches!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
                    | clap::error::ErrorKind::InvalidValue
                    | clap::error::ErrorKind::ValueValidation
                    | clap::error::ErrorKind::ArgumentConflict
            ),
            "{error}"
        );
    }
}

#[tokio::test]
async fn pool_invalid_local_policy_inputs_never_contact_owner_or_pool() {
    let server = MockServer::start().await;
    let invalid_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(invalid_file.path(), "[]").unwrap();
    for flags in [
        vec!["--retry-on", "none,429"],
        vec!["--file", invalid_file.path().to_str().unwrap()],
    ] {
        let mut args = vec!["set-failover", "my-llm", "--org", "acme"];
        args.extend(flags);
        assert!(run(parse(&server, &args)).await.is_err());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn pool_invalid_saved_policy_or_revision_never_overwrites() {
    for current in [
        json!({"id":POOL_ID,"config_revision":1,"failover":[]}),
        json!({"id":POOL_ID,"config_revision":null,"failover":{}}),
        json!({"id":POOL_ID,"config_revision":1,"failover":{"cooldown":false}}),
    ] {
        let server = MockServer::start().await;
        snapshot(&server, "my-llm", current).await;
        assert!(
            run(parse(
                &server,
                &["set-failover", "my-llm", "--cooldown-base-ms", "3000"]
            ))
            .await
            .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn pool_set_strategy_can_edit_tier_balancing_with_a_revision() {
    let server = MockServer::start().await;
    snapshot(&server, "my-llm", json!({"id":POOL_ID,"config_revision":6})).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/service-pools/{POOL_ID}")))
        .and(body_json(
            json!({"strategy":"priority","tier_balance":"weighted","expected_revision":6}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":POOL_ID})))
        .expect(1)
        .mount(&server)
        .await;
    run(parse(
        &server,
        &[
            "set-strategy",
            "my-llm",
            "priority",
            "--tier-balance",
            "weighted",
        ],
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn pool_member_flags_parse_and_send_all_settings_and_explicit_model_clear() {
    let server = MockServer::start().await;
    for (flags, body) in [
        (
            vec![
                "--priority",
                "10",
                "--weight",
                "3",
                "--enabled",
                "false",
                "--model",
                "native-model",
                "--same-api-compatible",
                "true",
            ],
            json!({"user_service_id":"backup","priority":10,"weight":3,"enabled":false,"model":"native-model","same_api_compatible":true}),
        ),
        (
            vec!["--clear-model"],
            json!({"user_service_id":"backup","model":null}),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path("/api/v1/service-pools/my-llm/members"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":POOL_ID})))
            .expect(1)
            .mount(&server)
            .await;
        let mut args = vec!["add-member", "my-llm", "--service", "backup"];
        args.extend(flags);
        run(parse(&server, &args)).await.unwrap();
    }
}

#[tokio::test]
async fn pool_update_pins_snapshot_id_and_preserves_explicit_revision() {
    for supplied_revision in [None, Some(4)] {
        let server = MockServer::start().await;
        snapshot(
            &server,
            "my-llm",
            json!({"id":POOL_ID,"config_revision":19}),
        )
        .await;
        let mut body = json!({"name":"Updated","failover":null});
        if let Some(revision) = supplied_revision {
            body["expected_revision"] = revision.into();
        }
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&body).unwrap()).unwrap();
        body["expected_revision"] = supplied_revision.unwrap_or(19).into();
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/service-pools/{POOL_ID}")))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":POOL_ID})))
            .expect(1)
            .mount(&server)
            .await;
        run(parse(
            &server,
            &["update", "my-llm", "--file", file.path().to_str().unwrap()],
        ))
        .await
        .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}
