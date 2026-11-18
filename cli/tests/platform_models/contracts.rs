use super::*;
fn definition() -> Value {
    json!({"version":"openai.responses.v2","family":"openai","transports":["direct.responses"],"fixture_sha256":"a".repeat(64),
      "flags":{"tokens":"required","cache_read":"required","split_reasoning":"optional","bill_tool_input":"forbidden","images":"forbidden","requests":"optional"},
      "cache_write_ttls":["thirty_minutes","twelve_hours"],"input_modalities":[],"output_modalities":[],
      "default_contract":{"version":"openai.responses.v2","family":"openai","fixture_sha256":"a".repeat(64),"tokens":true,"cache_read":true,"split_reasoning":false,"bill_tool_input":false,"images":false,"requests":false,"cache_write_ttls":["thirty_minutes","twelve_hours"],"input_modalities":[],"output_modalities":[]}})
}
async fn metadata(f: &Fixture, revision: i64, family: &str) {
    f.get("/admin/platform-models/contracts", json!([definition()]), 1)
        .await;
    f.get(
        &format!("/admin/platform-models/{MODEL}"),
        json!({"public_id":PUBLIC,"revision":revision,"family":family}),
        1,
    )
    .await;
}
#[tokio::test]
async fn list_uses_human_dispatch_and_renders_json_and_table() {
    let f = Fixture::new().await;
    f.get("/admin/platform-models/contracts", json!([definition()]), 2)
        .await;
    assert_eq!(
        stdout_json(
            &f.invoke(&["admin", "models", "contracts", "list"], "json")
                .await
        ),
        json!([definition()])
    );
    let text = table(
        &f.invoke(&["admin", "models", "contracts", "list"], "table")
            .await,
    );
    for value in [
        "openai.responses.v2",
        "direct.responses",
        "twelve_hours",
        "Fixture SHA-256",
    ] {
        assert!(text.contains(value), "{text}");
    }
}
#[tokio::test]
async fn attested_create_fills_identity_and_defaults_and_preserves_prices_and_options() {
    for overrides in [
        None,
        Some(
            json!({"split_reasoning":true,"cache_write_ttls":[],"version":"old","fixture_sha256":"old"}),
        ),
    ] {
        let f = Fixture::new().await;
        metadata(&f, 7, "openai").await;
        let mut manifest = json!({"lane":"platform_key","bounds":{"max_output_tokens":100},"components":[{"credits_per_unit":"0.000000000001"}],"variant":{}});
        if let Some(value) = &overrides {
            manifest["contract"] = value.clone();
        }
        let file = f.manifest(&manifest);
        let mut expected = manifest;
        expected["revision"] = json!(7);
        expected["source"] = json!(format!(
            "contract-attested-v1:{PUBLIC} provider docs reviewed"
        ));
        expected["contract"] = definition()["default_contract"].clone();
        if overrides.is_some() {
            expected["contract"]["split_reasoning"] = json!(true);
            expected["contract"]["cache_write_ttls"] = json!([]);
        }
        f.write(
            "POST",
            &format!("/admin/platform-models/{MODEL}/tariffs"),
            expected,
            json!({"status":"pending"}),
        )
        .await;
        let result = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "prices",
                    "create",
                    MODEL,
                    "--revision",
                    "7",
                    "--file",
                    &file,
                    "--contract",
                    "openai.responses.v2",
                    "--attest",
                    "provider docs reviewed",
                ],
                "json",
            )
            .await;
        assert_eq!(stdout_json(&result)["status"], "pending");
    }
}
#[tokio::test]
async fn invalid_contract_family_revision_or_options_never_post() {
    for (revision, family, override_value) in [
        (8, "openai", json!({})),
        (7, "gemini", json!({})),
        (7, "openai", json!({"images":true})),
        (7, "openai", json!({"tokens":false})),
        (7, "openai", json!({"cache_write_ttls":["five_minutes"]})),
        (
            7,
            "openai",
            json!({"cache_write_ttls":["thirty_minutes","thirty_minutes"]}),
        ),
        (7, "openai", json!({"unexpected":true})),
        (7, "openai", json!({"tokens":"yes"})),
        (7, "openai", json!({"input_modalities":"image"})),
        (7, "openai", json!(null)),
    ] {
        let f = Fixture::new().await;
        metadata(&f, revision, family).await;
        let file = f.manifest(&json!({"contract":override_value}));
        let out = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "prices",
                    "create",
                    MODEL,
                    "--revision",
                    "7",
                    "--file",
                    &file,
                    "--contract",
                    "openai.responses.v2",
                    "--attest",
                    "reviewed",
                ],
                "json",
            )
            .await;
        let _ = failure(&out);
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }
}
#[tokio::test]
async fn unknown_contract_and_invalid_attestation_do_not_write() {
    let f = Fixture::new().await;
    f.get("/admin/platform-models/contracts", json!([definition()]), 1)
        .await;
    let file = f.manifest(&json!({}));
    let out = f
        .invoke(
            &[
                "admin",
                "models",
                "prices",
                "create",
                MODEL,
                "--revision",
                "7",
                "--file",
                &file,
                "--contract",
                "unknown.v2",
                "--attest",
                "reviewed",
            ],
            "json",
        )
        .await;
    assert!(
        failure(&out)
            .to_string()
            .contains("Unknown certified contract")
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    for reference in [" ", "invalid\nreference"] {
        let f = Fixture::new().await;
        let file = f.manifest(&json!({}));
        let out = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "prices",
                    "create",
                    MODEL,
                    "--revision",
                    "7",
                    "--file",
                    &file,
                    "--contract",
                    "openai.responses.v2",
                    "--attest",
                    reference,
                ],
                "json",
            )
            .await;
        let _ = failure(&out);
        assert!(f.server.received_requests().await.unwrap().is_empty());
    }
    let f = Fixture::new().await;
    metadata(&f, 7, "openai").await;
    let file = f.manifest(&json!({}));
    let out = f
        .invoke(
            &[
                "admin",
                "models",
                "prices",
                "create",
                MODEL,
                "--revision",
                "7",
                "--file",
                &file,
                "--contract",
                "openai.responses.v2",
                "--attest",
                &"x".repeat(1000),
            ],
            "json",
        )
        .await;
    assert!(failure(&out).to_string().contains("1000 UTF-8 bytes"));
}
#[tokio::test]
async fn stale_write_and_server_errors_are_not_retried() {
    for status in [409, 503] {
        let f = Fixture::new().await;
        metadata(&f, 7, "openai").await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/admin/platform-models/{MODEL}/tariffs"
            )))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(
                    json!({"error":"unavailable","message":"reload current revision"}),
                ),
            )
            .expect(1)
            .mount(&f.server)
            .await;
        let file = f.manifest(&json!({}));
        let out = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "prices",
                    "create",
                    MODEL,
                    "--revision",
                    "7",
                    "--file",
                    &file,
                    "--contract",
                    "openai.responses.v2",
                    "--attest",
                    "reviewed",
                ],
                "json",
            )
            .await;
        assert!(failure(&out).to_string().contains(&status.to_string()));
        assert_eq!(f.server.received_requests().await.unwrap().len(), 3);
    }
}
#[tokio::test]
async fn attestation_options_require_each_other() {
    let f = Fixture::new().await;
    for option in ["--contract", "--attest"] {
        let out = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "prices",
                    "create",
                    MODEL,
                    "--revision",
                    "7",
                    "--file",
                    "unused.json",
                    option,
                    "value",
                ],
                "json",
            )
            .await;
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("required"));
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn contracts_and_attested_prices_refuse_agent_keys_before_http() {
    let f = Fixture::new().await;
    let file = f.manifest(&json!({}));
    for args in [
        vec!["admin", "models", "contracts", "list"],
        vec![
            "admin",
            "models",
            "prices",
            "create",
            MODEL,
            "--revision",
            "7",
            "--file",
            &file,
            "--contract",
            "openai.responses.v2",
            "--attest",
            "reviewed",
        ],
    ] {
        let output = f
            .invoke_token(&args, "json", "nyx_agent_test_credential")
            .await;
        assert!(
            failure(&output)
                .to_string()
                .contains("human account session")
        );
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
}
