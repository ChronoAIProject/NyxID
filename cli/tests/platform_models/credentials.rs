use super::*;
const ROOT: &str = "/admin/platform-models/credentials";
fn row() -> Value {
    json!({"id":SERVICE,"name":"Provider key","kind":"openai","enabled":true,"created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-02T00:00:00Z","references":[],"credential":"never-show-this","fingerprint":"never-show-this"})
}
fn secret_file(f: &Fixture) -> String {
    let file = f.manifest(
        &json!({"type":"service_account","private_key":"fixture-only-not-a-provider-key"}),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    file
}
#[tokio::test]
async fn credential_list_paginates_and_allowlists_json_and_table_output() {
    for format in ["json", "table"] {
        let f = Fixture::new().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1{ROOT}")))
            .respond_with(|req: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(if req.url.query().is_none() {
                    json!({"records":[row()],"next_after":SERVICE})
                } else {
                    json!({"records":[],"next_after":null})
                })
            })
            .expect(2)
            .mount(&f.server)
            .await;
        let output = f
            .invoke(&["admin", "models", "credentials", "list"], format)
            .await;
        let text = if format == "json" {
            stdout_json(&output).to_string()
        } else {
            table(&output)
        };
        assert!(text.contains("Provider key"));
        assert!(!text.contains("never-show-this"));
        assert!(!text.contains("fingerprint"));
        assert_eq!(
            f.server.received_requests().await.unwrap()[1].url.query(),
            Some(format!("after={SERVICE}").as_str())
        );
    }
}
#[tokio::test]
async fn add_all_kinds_reads_secret_sources_and_dispatches_without_echo() {
    for kind in ["gcp-service-account", "openai", "anthropic"] {
        let f = Fixture::new().await;
        let file = secret_file(&f);
        Mock::given(method("POST"))
            .and(path(format!("/api/v1{ROOT}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(row()))
            .expect(1)
            .mount(&f.server)
            .await;
        let source = if kind == "gcp-service-account" {
            vec!["--file", &file]
        } else {
            vec!["--key-env", "NYXID_TEST_MODEL_CREDENTIAL"]
        };
        let mut args = vec![
            "admin",
            "models",
            "credentials",
            "add",
            "--kind",
            kind,
            "--name",
            "Provider key",
        ];
        args.extend(source);
        let output = f.invoke(&args, "json").await;
        assert_eq!(stdout_json(&output)["records"][0]["id"], SERVICE);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-only"));
        let request = f.server.received_requests().await.unwrap().pop().unwrap();
        let body: Value = request.body_json().unwrap();
        assert_eq!(body["kind"], kind);
        assert_eq!(body["name"], "Provider key");
        assert!(
            body["credential"]
                .as_str()
                .unwrap()
                .contains("fixture-only")
        );
    }
}
#[tokio::test]
async fn rotate_reads_revision_and_sanitizes_conflict_and_other_errors_without_retry() {
    for status in [200, 400, 403, 409, 500] {
        let f = Fixture::new().await;
        f.get(&format!("{ROOT}/{SERVICE}"), row(), 1).await;
        Mock::given(method("PUT")).and(path(format!("/api/v1{ROOT}/{SERVICE}"))).and(body_json(json!({"expected_updated_at":"2026-11-02T00:00:00Z","credential":"fixture-only-model-secret"}))).respond_with(ResponseTemplate::new(status).set_body_json(if status==200 {row()} else {json!({"error":"fixture-only-model-secret"})})).expect(1).mount(&f.server).await;
        let output = f
            .invoke(
                &[
                    "admin",
                    "models",
                    "credentials",
                    "rotate",
                    SERVICE,
                    "--key-env",
                    "NYXID_TEST_MODEL_CREDENTIAL",
                ],
                "json",
            )
            .await;
        if status == 200 {
            assert_eq!(stdout_json(&output)["records"][0]["id"], SERVICE);
        } else {
            assert_eq!(failure(&output)["status"], status);
        }
        assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture-only-model-secret"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-only-model-secret"));
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    }
}
#[tokio::test]
async fn rotate_service_account_from_private_file_and_create_offering() {
    let f = Fixture::new().await;
    let file = secret_file(&f);
    let mut gcp = row();
    gcp["kind"] = json!("gcp-service-account");
    f.get(&format!("{ROOT}/{SERVICE}"), gcp.clone(), 1).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1{ROOT}/{SERVICE}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(gcp))
        .expect(1)
        .mount(&f.server)
        .await;
    assert!(
        table(
            &f.invoke(
                &[
                    "admin",
                    "models",
                    "credentials",
                    "rotate",
                    SERVICE,
                    "--file",
                    &file
                ],
                "table"
            )
            .await
        )
        .contains("Google Cloud service account")
    );
    f.write(
        "POST",
        "/admin/platform-models/offerings",
        json!({"name":"Billing identity"}),
        row(),
    )
    .await;
    assert_eq!(
        stdout_json(
            &f.invoke(
                &[
                    "admin",
                    "models",
                    "offering",
                    "create",
                    "--name",
                    "Billing identity"
                ],
                "json"
            )
            .await
        )["id"],
        SERVICE
    );
}
#[tokio::test]
async fn disable_refuses_enabled_references_else_sends_revision_fenced_patch() {
    for referenced in [false, true] {
        let f = Fixture::new().await;
        let mut record = row();
        if referenced {
            record["references"] = json!([{"model_id":MODEL,"public_id":PUBLIC,"display_name":"Model","route_id":"route","enabled":true}]);
        }
        f.get(&format!("{ROOT}/{SERVICE}"), record, 1).await;
        if !referenced {
            let mut disabled = row();
            disabled["enabled"] = json!(false);
            f.write(
                "PUT",
                &format!("{ROOT}/{SERVICE}"),
                json!({"expected_updated_at":"2026-11-02T00:00:00Z","enabled":false}),
                disabled,
            )
            .await;
        }
        let result = f
            .invoke(
                &["admin", "models", "credentials", "disable", SERVICE],
                "json",
            )
            .await;
        if referenced {
            assert!(
                failure(&result)["message"]
                    .as_str()
                    .unwrap()
                    .contains("enabled routes")
            );
        } else {
            assert_eq!(stdout_json(&result)["records"][0]["enabled"], false);
        }
        assert_eq!(
            f.server.received_requests().await.unwrap().len(),
            if referenced { 1 } else { 2 }
        );
    }
}
#[tokio::test]
async fn credential_commands_require_human_and_reject_unsafe_or_missing_secret_sources() {
    let f = Fixture::new().await;
    let file = secret_file(&f);
    for args in [
        vec!["admin", "models", "credentials", "list"],
        vec![
            "admin",
            "models",
            "credentials",
            "add",
            "--kind",
            "openai",
            "--name",
            "K",
            "--key-env",
            "NYXID_TEST_MODEL_CREDENTIAL",
        ],
        vec![
            "admin",
            "models",
            "credentials",
            "rotate",
            SERVICE,
            "--file",
            &file,
        ],
        vec!["admin", "models", "credentials", "disable", SERVICE],
        vec!["admin", "models", "offering", "create", "--name", "Billing"],
    ] {
        assert!(
            failure(&f.invoke_token(&args, "json", "nyx_agent").await)["message"]
                .as_str()
                .unwrap()
                .contains("human account session")
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            failure(
                &f.invoke(
                    &[
                        "admin",
                        "models",
                        "credentials",
                        "add",
                        "--kind",
                        "gcp-service-account",
                        "--name",
                        "K",
                        "--file",
                        &file
                    ],
                    "json"
                )
                .await
            )["message"]
                .as_str()
                .unwrap()
                .contains("chmod 600")
        );
    }
    assert!(
        failure(
            &f.invoke(
                &[
                    "admin",
                    "models",
                    "credentials",
                    "add",
                    "--kind",
                    "openai",
                    "--name",
                    "K",
                    "--key-env",
                    "NYXID_TEST_MISSING_CREDENTIAL"
                ],
                "json"
            )
            .await
        )["message"]
            .as_str()
            .unwrap()
            .contains("missing")
    );
    for args in [
        vec![
            "admin",
            "models",
            "credentials",
            "add",
            "--kind",
            "openai",
            "--name",
            "K",
        ],
        vec![
            "admin",
            "models",
            "credentials",
            "rotate",
            SERVICE,
            "--file",
            &file,
            "--key-env",
            "KEY",
        ],
        vec![
            "admin",
            "models",
            "credentials",
            "add",
            "--kind",
            "unsupported",
            "--name",
            "K",
            "--file",
            &file,
        ],
    ] {
        assert_eq!(f.invoke(&args, "json").await.status.code(), Some(2));
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn offering_table_identifies_the_billing_identity_without_a_credential_label() {
    let f = Fixture::new().await;
    let mut offering = row();
    offering["kind"] = Value::Null;
    offering["name"] = json!("Model identity");
    f.write(
        "POST",
        "/admin/platform-models/offerings",
        json!({"name":"Model identity"}),
        offering,
    )
    .await;
    let output = table(
        &f.invoke(
            &[
                "admin",
                "models",
                "offering",
                "create",
                "--name",
                "Model identity",
            ],
            "table",
        )
        .await,
    );
    assert!(output.contains("Billing identity"));
    assert!(output.contains("Model identity"));
    assert!(!output.contains("Provider credential"));
    assert!(!output.contains("never-show-this"));
}
