//! Exercise the shipped entry point: clap -> main/admin dispatch -> HTTP -> output.
//! Only localhost mock servers are used. Explicit credentials and a fresh profile
//! keep subprocesses independent of the developer's saved account.
use serde_json::{Value, json};
use std::{process::Output, time::Duration};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

const MODEL: &str = "00000000-0000-4000-8000-000000000001";
const SERVICE: &str = "00000000-0000-4000-8000-000000000002";
const TARIFF: &str = "00000000-0000-4000-8000-000000000003";
const PERSON: &str = "00000000-0000-4000-8000-000000000004";
const TOKEN: &str = "test-model-session";
const PUBLIC: &str = "nyxid/test-model";

struct Fixture {
    server: MockServer,
    files: tempfile::TempDir,
    profile: String,
}
impl Fixture {
    async fn new() -> Self {
        Self {
            server: MockServer::start().await,
            files: tempfile::tempdir().unwrap(),
            profile: format!("model-test-{}", uuid::Uuid::new_v4()),
        }
    }
    fn manifest(&self, value: &Value) -> String {
        let file = self.files.path().join("manifest.json");
        std::fs::write(&file, serde_json::to_vec(value).unwrap()).unwrap();
        file.to_str().unwrap().into()
    }
    async fn invoke(&self, args: &[&str], format: &str) -> Output {
        self.invoke_token(args, format, TOKEN).await
    }
    async fn invoke_token(&self, args: &[&str], format: &str, token: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
        command
            .args(args)
            .args(["--base-url", &self.server.uri(), "--access-token", token])
            .args(["--profile", &self.profile, "--output", format])
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env("NYXID_TELEMETRY", "off")
            .env("DO_NOT_TRACK", "1")
            .env_remove("NYXID_CA_CERT")
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(20), command.output())
            .await
            .expect("CLI must complete without a prompt")
            .expect("run CLI")
    }
    async fn get(&self, url: &str, response: Value, times: u64) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1{url}")))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .expect(times)
            .mount(&self.server)
            .await;
    }
    async fn write(&self, verb: &str, url: &str, body: Value, response: Value) {
        Mock::given(method(verb))
            .and(path(format!("/api/v1{url}")))
            .and(header("authorization", format!("Bearer {TOKEN}")))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .expect(1)
            .mount(&self.server)
            .await;
    }
}
fn stdout_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "JSON success must not print a table"
    );
    serde_json::from_slice(&output.stdout).expect("one JSON document on stdout")
}
fn table(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "human output belongs on stderr");
    String::from_utf8(output.stderr.clone()).unwrap()
}
fn failure(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "failure must not print a success result"
    );
    serde_json::from_slice(&output.stderr).expect("structured error on stderr")
}
fn public_model() -> Value {
    json!({"id":PUBLIC,"display_name":"Test model","description":"Public description",
        "availability":"available","capabilities":{"tools":true,"context_tokens":1000,"project":"PRIVATE_PROJECT"},
        "retail_prices":[{"available":true,"variant":{},"active_tariff_id":"PRIVATE_TARIFF",
            "components":[{"metric":"input_tokens","selector":{"kind":"input","project":"PRIVATE_PROJECT"},"credits_per_unit":"0.000000000001","explicit_free":false,"lago_metric_code":"PRIVATE_METRIC"},
                {"metric":"requests","selector":{"kind":"requests"},"credits_per_unit":"0","explicit_free":true}]}],
        "routes":[{"project":"PRIVATE_PROJECT","credential_service_id":"PRIVATE_CREDENTIAL"}]})
}
fn admin_model() -> Value {
    json!({"id":MODEL,"public_id":PUBLIC,"display_name":"Test model","description":"Description",
        "revision":7,"lifecycle":"active","enabled":true,"capabilities":{"tools":true},
        "billing_service_id":SERVICE,"routing_revision_id":"route-revision",
        "routes":[{"id":"private-route","project":"admin-project"}],
        "entitlements":{"all_people":false,"rules":[]},"execution_ready":false,
        "blocking_reason":"model_prices_pending","required_readiness":["billing:model-prices-v1"]})
}
fn model_path() -> String {
    format!("/admin/platform-models/{MODEL}")
}

#[tokio::test]
async fn public_list_dispatches_default_page_and_redacts_json() {
    let f = Fixture::new().await;
    f.get(
        "/llm/models",
        json!({"models":[public_model()],"next_after":PUBLIC}),
        1,
    )
    .await;
    let shown = stdout_json(&f.invoke(&["models", "list"], "json").await);
    assert_eq!(shown["models"][0]["id"], PUBLIC);
    assert_eq!(shown["next_after"], PUBLIC);
    assert_eq!(
        shown["models"][0]["retail_prices"][0]["components"][0]["credits_per_unit"],
        "0.000000000001"
    );
    assert!(!shown.to_string().contains("PRIVATE_"));
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), Some("limit=50"));
}

#[tokio::test]
async fn public_list_table_preserves_prices_and_encodes_cursor_as_one_parameter() {
    let f = Fixture::new().await;
    let cursor = "nyxid/one & limit=1";
    Mock::given(method("GET"))
        .and(path("/api/v1/llm/models"))
        .and(query_param("limit", "100"))
        .and(query_param("after", cursor))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models":[public_model()],"next_after":PUBLIC})),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    let shown = table(
        &f.invoke(
            &["models", "list", "--limit", "100", "--after", cursor],
            "table",
        )
        .await,
    );
    for expected in [
        PUBLIC,
        "Public description",
        "0.000000000001 credits / unit",
        "Free",
        "Next cursor",
        "context_tokens",
    ] {
        assert!(shown.contains(expected), "{expected}: {shown}");
    }
    assert!(!shown.contains("PRIVATE_"));
    assert_eq!(
        f.server.received_requests().await.unwrap()[0]
            .url
            .query_pairs()
            .count(),
        2
    );
}

#[tokio::test]
async fn public_show_dispatches_both_formats_and_follows_authorized_pages() {
    for format in ["json", "table"] {
        let f = Fixture::new().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/llm/models"))
            .and(|r: &wiremock::Request| !r.url.query_pairs().any(|(k, _)| k == "after"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"models":[],"next_after":"nyxid/first"})),
            )
            .expect(1)
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/llm/models"))
            .and(query_param("after", "nyxid/first"))
            .and(query_param("limit", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"models":[public_model()],"next_after":null})),
            )
            .expect(1)
            .mount(&f.server)
            .await;
        let result = f.invoke(&["models", "show", PUBLIC], format).await;
        if format == "json" {
            let shown = stdout_json(&result);
            assert_eq!(shown["id"], PUBLIC);
            assert!(!shown.to_string().contains("PRIVATE_"));
        } else {
            let shown = table(&result);
            assert!(shown.contains(PUBLIC));
            assert!(shown.contains("0.000000000001"));
            assert!(!shown.contains("PRIVATE_"));
        }
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn admin_list_and_show_dispatch_both_output_formats() {
    let f = Fixture::new().await;
    let page = json!({"models":[admin_model()],"next_after":PUBLIC});
    f.get("/admin/platform-models", page.clone(), 2).await;
    f.get(&model_path(), admin_model(), 2).await;
    for format in ["json", "table"] {
        let listed = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "list",
                    "--limit",
                    "1",
                    "--after",
                    "nyxid/earlier",
                ],
                format,
            )
            .await;
        let detail = f.invoke(&["admin", "models", "show", MODEL], format).await;
        if format == "json" {
            assert_eq!(stdout_json(&listed), page);
            assert_eq!(stdout_json(&detail), admin_model());
        } else {
            let text = table(&listed);
            for expected in [MODEL, PUBLIC, "Revision", "7", "Next cursor"] {
                assert!(text.contains(expected), "{text}");
            }
            assert!(table(&detail).contains("admin-project"));
        }
    }
    for request in f
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("platform-models"))
    {
        assert_eq!(request.url.query(), Some("limit=1&after=nyxid%2Fearlier"));
    }
}

#[tokio::test]
async fn admin_create_and_update_send_the_manifest_and_revision_without_private_field_invention() {
    let f = Fixture::new().await;
    let mut body = json!({"public_id":PUBLIC,"display_name":"Draft","family":"gemini","description":"","capabilities":{"tools":true},"billing_service_id":SERVICE});
    let file = f.manifest(&body);
    f.write(
        "POST",
        "/admin/platform-models",
        body.clone(),
        admin_model(),
    )
    .await;
    assert_eq!(
        stdout_json(
            &f.invoke(&["admin", "models", "create", "--file", &file], "json")
                .await
        ),
        admin_model()
    );
    body.as_object_mut().unwrap().remove("public_id");
    body.as_object_mut().unwrap().remove("family");
    body["enabled"] = json!(false);
    body["lifecycle"] = json!("active");
    let file = f.manifest(&body);
    body["expected_revision"] = json!(7);
    f.write(
        "PUT",
        &model_path(),
        body,
        json!({"revision":8,"enabled":false}),
    )
    .await;
    let result = stdout_json(
        &f.invoke(
            &[
                "admin",
                "models",
                "update",
                MODEL,
                "--revision",
                "7",
                "--file",
                &file,
            ],
            "json",
        )
        .await,
    );
    assert_eq!(result, json!({"revision":8,"enabled":false}));
}

#[tokio::test]
async fn admin_retire_preserves_contract_and_fences_the_read_revision() {
    let f = Fixture::new().await;
    f.get(&model_path(), admin_model(), 1).await;
    f.write("PUT", &model_path(), json!({"display_name":"Test model","description":"Description","capabilities":{"tools":true},"billing_service_id":SERVICE,"expected_revision":7,"lifecycle":"retired","enabled":false}), json!({"lifecycle":"retired","revision":8})).await;
    assert_eq!(
        stdout_json(
            &f.invoke(
                &["admin", "models", "retire", MODEL, "--revision", "7"],
                "json"
            )
            .await
        )["lifecycle"],
        "retired"
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn stale_retire_refuses_before_any_mutation() {
    let f = Fixture::new().await;
    f.get(&model_path(), admin_model(), 1).await;
    let error = failure(
        &f.invoke(
            &["admin", "models", "retire", MODEL, "--revision", "6"],
            "json",
        )
        .await,
    );
    assert!(error["message"].as_str().unwrap().contains("Model changed"));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn route_and_access_show_project_only_the_requested_admin_fields() {
    let f = Fixture::new().await;
    f.get(&model_path(), admin_model(), 2).await;
    let routes = stdout_json(
        &f.invoke(&["admin", "models", "routes", "show", MODEL], "json")
            .await,
    );
    assert_eq!(
        routes,
        json!({"id":MODEL,"public_id":PUBLIC,"revision":7,"routing_revision_id":"route-revision","routes":[{"id":"private-route","project":"admin-project"}]})
    );
    let access = stdout_json(
        &f.invoke(&["admin", "models", "access", "show", MODEL], "json")
            .await,
    );
    assert_eq!(
        access,
        json!({"id":MODEL,"public_id":PUBLIC,"revision":7,"entitlements":{"all_people":false,"rules":[]}})
    );
}

#[tokio::test]
async fn route_and_access_set_dispatch_to_distinct_revision_fenced_endpoints() {
    for (subcommand, suffix, field, value) in [
        ("routes", "routes", "routes", json!([])),
        (
            "access",
            "entitlements",
            "entitlements",
            json!({"all_people":false,"rules":[{"subject_kind":"person","subject_id":PERSON,"effect":"deny"}]}),
        ),
    ] {
        let f = Fixture::new().await;
        let mut body = json!({field:value});
        let file = f.manifest(&body);
        body["expected_revision"] = json!(7);
        f.write(
            "PUT",
            &format!("{}/{suffix}", model_path()),
            body,
            json!({"revision":8}),
        )
        .await;
        assert_eq!(
            stdout_json(
                &f.invoke(
                    &[
                        "admin",
                        "models",
                        subcommand,
                        "set",
                        MODEL,
                        "--revision",
                        "7",
                        "--file",
                        &file
                    ],
                    "json"
                )
                .await
            ),
            json!({"revision":8})
        );
    }
}

#[tokio::test]
async fn access_explain_uses_requested_person_and_preserves_deny_reason() {
    let f = Fixture::new().await;
    let response = json!({"policy_reason":"explicit_deny","executable":false});
    Mock::given(method("GET"))
        .and(path(format!("/api/v1{}/access", model_path())))
        .and(query_param("user_id", PERSON))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(
        stdout_json(
            &f.invoke(
                &[
                    "admin", "models", "access", "explain", MODEL, "--user", PERSON
                ],
                "json"
            )
            .await
        ),
        response
    );
}

#[tokio::test]
async fn price_list_and_create_preserve_exact_decimal_strings() {
    let f = Fixture::new().await;
    let tariffs = json!({"tariffs":[{"id":TARIFF,"status":"pending","components":[{"metric":"future_unit","credits_per_unit":"0.000000000001"}]}]});
    f.get(&format!("{}/tariffs", model_path()), tariffs.clone(), 1)
        .await;
    assert_eq!(
        stdout_json(
            &f.invoke(&["admin", "models", "prices", "list", MODEL], "json")
                .await
        ),
        tariffs
    );
    let mut body = json!({"lane":"platform_key","variant":{},"source":"certified","contract":{},"bounds":{},"components":[{"metric":"future_unit","credits_per_unit":"0.000000000001"}]});
    let file = f.manifest(&body);
    body["revision"] = json!(7);
    f.write(
        "POST",
        &format!("{}/tariffs", model_path()),
        body,
        json!({"id":TARIFF,"status":"pending"}),
    )
    .await;
    assert_eq!(
        stdout_json(
            &f.invoke(
                &[
                    "admin",
                    "models",
                    "prices",
                    "create",
                    MODEL,
                    "--revision",
                    "7",
                    "--file",
                    &file
                ],
                "json"
            )
            .await
        )["id"],
        TARIFF
    );
}

#[tokio::test]
async fn price_publish_and_resync_target_the_models_logical_service() {
    for verb in ["publish", "resync"] {
        let f = Fixture::new().await;
        f.get(&model_path(), admin_model(), 1).await;
        f.write(
            "POST",
            &format!("/admin/billing/services/{SERVICE}/resync"),
            json!({}),
            json!({"status":"pending"}),
        )
        .await;
        let value = stdout_json(
            &f.invoke(&["admin", "models", "prices", verb, MODEL], "json")
                .await,
        );
        assert_eq!(value["billing"]["status"], "pending");
        assert_eq!(value["model"]["public_id"], PUBLIC);
        assert_eq!(value["model"]["execution_ready"], false);
        assert!(value["model"].get("routes").is_none());
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn price_verify_is_read_only_and_shows_readiness_in_table_output() {
    let f = Fixture::new().await;
    f.get(&model_path(), admin_model(), 1).await;
    f.get(
        &format!("/admin/billing/services/{SERVICE}"),
        json!({"pending":["price_unsynced"]}),
        1,
    )
    .await;
    let shown = table(
        &f.invoke(&["admin", "models", "prices", "verify", MODEL], "table")
            .await,
    );
    for expected in [
        PUBLIC,
        "model_prices_pending",
        "billing:model-prices-v1",
        "price_unsynced",
    ] {
        assert!(shown.contains(expected), "{shown}");
    }
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.method == "GET"));
}

#[tokio::test]
async fn price_retire_accepts_empty_204_and_reports_success() {
    let f = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1{}/tariffs/{TARIFF}/retire",
            model_path()
        )))
        .and(body_json(json!({"revision":7})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(
        stdout_json(
            &f.invoke(
                &[
                    "admin",
                    "models",
                    "prices",
                    "retire",
                    MODEL,
                    TARIFF,
                    "--revision",
                    "7"
                ],
                "json"
            )
            .await
        ),
        json!({"retired":true})
    );
}

#[tokio::test]
async fn write_conflicts_denials_and_server_errors_are_not_replayed() {
    for status in [401, 403, 409, 500] {
        let f = Fixture::new().await;
        let file = f.manifest(&json!({"routes":[]}));
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1{}/routes", model_path())))
            .and(body_json(json!({"routes":[],"expected_revision":7})))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"error":"write refused","code":status})),
            )
            .expect(1)
            .mount(&f.server)
            .await;
        let result = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "routes",
                    "set",
                    MODEL,
                    "--revision",
                    "7",
                    "--file",
                    &file,
                ],
                "json",
            )
            .await;
        assert_eq!(failure(&result)["status"], status);
        assert_eq!(
            f.server.received_requests().await.unwrap().len(),
            1,
            "No refresh/retry after a failed write"
        );
    }
}

#[tokio::test]
async fn discovery_http_errors_reach_json_and_human_error_output_without_admin_fallback() {
    for format in ["json", "table"] {
        let f = Fixture::new().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/llm/models"))
            .respond_with(ResponseTemplate::new(503).set_body_json(json!({"error":"unavailable"})))
            .expect(1)
            .mount(&f.server)
            .await;
        let result = f.invoke(&["models", "list"], format).await;
        if format == "json" {
            assert_eq!(failure(&result)["status"], 503);
        } else {
            assert_eq!(result.status.code(), Some(1));
            assert!(result.stdout.is_empty());
            assert!(String::from_utf8_lossy(&result.stderr).contains("503"));
        }
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn public_show_refuses_missing_models_and_repeated_cursors_without_private_reads() {
    for (next, count, reason) in [
        (Value::Null, 1, "Model unavailable"),
        (json!("nyxid/repeated"), 2, "repeated cursor"),
    ] {
        let f = Fixture::new().await;
        f.get("/llm/models", json!({"models":[],"next_after":next}), count)
            .await;
        let error = failure(&f.invoke(&["models", "show", PUBLIC], "json").await);
        assert!(error["message"].as_str().unwrap().contains(reason));
        assert_eq!(
            f.server.received_requests().await.unwrap().len() as u64,
            count
        );
    }
}

#[tokio::test]
async fn identifiers_are_validated_before_any_http_request() {
    for args in [
        vec!["models", "show", MODEL],
        vec!["admin", "models", "show", "invalid"],
        vec![
            "admin", "models", "access", "explain", MODEL, "--user", "invalid",
        ],
        vec![
            "admin",
            "models",
            "prices",
            "retire",
            MODEL,
            "invalid",
            "--revision",
            "7",
        ],
    ] {
        let f = Fixture::new().await;
        let error = failure(&f.invoke(&args, "json").await);
        assert_eq!(error["error"], "cli_error");
        let reason = error["message"].as_str().unwrap();
        assert!(
            reason.contains("UUID") || reason.contains("public nyxid/model-name"),
            "{error}"
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn invalid_manifests_and_revision_overrides_never_send_a_write() {
    for (contents, revision, reason) in [
        ("{", "7", "Parse model manifest JSON"),
        ("[]", "7", "Manifest must be a JSON object"),
        ("{}", "0", "Revision must be positive"),
        (
            r#"{"revision":3}"#,
            "7",
            "Pass the revision only via --revision",
        ),
        (
            r#"{"expected_revision":3}"#,
            "7",
            "Pass the revision only via --revision",
        ),
    ] {
        let f = Fixture::new().await;
        let file = f.manifest(&json!({}));
        std::fs::write(&file, contents).unwrap();
        let error = failure(
            &f.invoke(
                &[
                    "admin",
                    "models",
                    "update",
                    MODEL,
                    "--revision",
                    revision,
                    "--file",
                    &file,
                ],
                "json",
            )
            .await,
        );
        assert!(
            error["message"].as_str().unwrap().contains(reason),
            "{error}"
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
    }
    let f = Fixture::new().await;
    let missing = f.files.path().join("missing.json");
    let error = failure(
        &f.invoke(
            &[
                "admin",
                "models",
                "create",
                "--file",
                missing.to_str().unwrap(),
            ],
            "json",
        )
        .await,
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Read model manifest")
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn billing_commands_refuse_missing_or_invalid_service_bindings() {
    for (service, reason) in [
        (Value::Null, "Model has no logical billing service"),
        (json!("bad-id"), "Expected a UUID"),
    ] {
        let f = Fixture::new().await;
        f.get(&model_path(), json!({"billing_service_id":service}), 1)
            .await;
        let error = failure(
            &f.invoke(&["admin", "models", "prices", "publish", MODEL], "json")
                .await,
        );
        assert!(
            error["message"].as_str().unwrap().contains(reason),
            "{error}"
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn admin_agent_key_cannot_read_or_mutate_models() {
    let f = Fixture::new().await;
    let file = f.manifest(&json!({"routes":[]}));
    for args in [
        vec!["admin", "models", "list"],
        vec!["admin", "models", "show", MODEL],
        vec!["admin", "models", "create", "--file", &file],
        vec![
            "admin",
            "models",
            "routes",
            "set",
            MODEL,
            "--revision",
            "7",
            "--file",
            &file,
        ],
        vec![
            "admin",
            "models",
            "prices",
            "retire",
            MODEL,
            TARIFF,
            "--revision",
            "7",
        ],
    ] {
        let error = failure(&f.invoke_token(&args, "json", "nyx_agent_key").await);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("human account session")
        );
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
}
