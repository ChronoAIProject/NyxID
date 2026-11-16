use super::*;
use crate::cli::Cli;
use crate::test_support::{env_lock, mock_auth};
use clap::Parser;
use std::io::Write as _;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path, query_param},
};

const LINE: &str = "0b8f4b44-3d73-4d0c-8f59-5d1c4f7b2a61";
const ACCOUNT: &str = "8f8a6e93-1f0c-4f3e-9d77-2a8b9b0b6c3a";
const OWNER: &str = "3f0b8a3e-6c55-4b1e-9a61-1c2f0e9a7d10";

fn file(contents: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f
}

#[tokio::test]
async fn underwriting_and_cost_policy_send_decimal_strings_and_rfc3339() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/api/v1/admin/managed-lines/accounts/{ACCOUNT}/underwriting"
        )))
        .and(body_json(json!({
            "revision": 2,
            "enabled": true,
            "components": [{"kind": "carrier_inbound", "credits_per_second": "0.0002",
                "increment_seconds": 60, "minimum_seconds": 60}],
            "rolling_budget": "500"
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/api/v1/admin/managed-lines/{LINE}/cost-policy"
        )))
        .and(body_json(json!({
            "revision": 0, "fx": [], "schemas": [],
            "inventory": {"cycle": "calendar_month", "cycle_anchor": "2026-11-01T00:00:00Z",
                "amount": "1.2", "currency": "USD", "source": "https://supplier.example/rental",
                "fx_policy_revision": "fx-1"}
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let underwriting = file(
        "enabled: true\ncomponents:\n  - kind: carrier_inbound\n    credits_per_second: \"0.0002\"\n    increment_seconds: 60\n    minimum_seconds: 60\nrolling_budget: \"500\"\n",
    );
    run(AdminLineCommands::Account {
        command: AccountCommands::Underwriting {
            account: ACCOUNT.into(),
            revision: 2,
            file: underwriting.path().into(),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    let policy = file(
        r#"{"fx":[],"schemas":[],"inventory":{"cycle":"calendar_month","cycle_anchor":"2026-11-01T00:00:00Z","amount":"1.2","currency":"USD","source":"https://supplier.example/rental","fx_policy_revision":"fx-1"}}"#,
    );
    run(AdminLineCommands::Account {
        command: AccountCommands::CostPolicy {
            line: LINE.into(),
            revision: 0,
            file: policy.path().into(),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    let mismatched = file(r#"{"revision": 7}"#);
    assert!(
        run(AdminLineCommands::Update {
            line: LINE.into(),
            revision: 3,
            file: mismatched.path().into(),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap_err()
        .to_string()
        .contains("--revision")
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn rotation_reads_the_secret_from_env_or_file_never_argv() {
    let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1/admin/managed-comms/accounts/{ACCOUNT}/rotate"
        )))
        .and(body_json(
            json!({"expected_revision": 4, "credential": "successor-key"}),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    let env = "NYXID_TEST_MANAGED_ROTATION_SECRET";
    unsafe {
        std::env::set_var(env, "successor-key");
    }
    let from_env = run(AdminLineCommands::Account {
        command: AccountCommands::Rotate {
            account: ACCOUNT.into(),
            revision: 4,
            credential_env: Some(env.into()),
            credential_file: None,
            auth: mock_auth(server.uri()),
        },
    })
    .await;
    unsafe {
        std::env::remove_var(env);
    }
    from_env.unwrap();
    let secret = file("successor-key\n");
    run(AdminLineCommands::Account {
        command: AccountCommands::Rotate {
            account: ACCOUNT.into(),
            revision: 4,
            credential_env: None,
            credential_file: Some(secret.path().into()),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    let base = [
        "nyxid",
        "admin",
        "line",
        "account",
        "rotate",
        ACCOUNT,
        "--revision",
        "4",
    ];
    assert!(
        Cli::try_parse_from(base).is_err(),
        "a credential source is required"
    );
    let mut argv = base.to_vec();
    argv.extend(["--credential", "raw-secret"]);
    assert!(
        Cli::try_parse_from(&argv).is_err(),
        "argv secrets are not accepted"
    );
}

#[tokio::test]
async fn eligibility_grant_resolves_people_and_update_keeps_current_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/admin/users"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"users": [{"id": OWNER, "email": "chef@example.com"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/admin/managed-comms/eligibility"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "eligibility": [{
                "owner_id": OWNER, "business_legal_name": "Jolly Chili Pte Ltd",
                "registration_id": "201912345K", "jurisdiction": "SG",
                "allowed_use_case": "restaurant bookings", "sms_sender_path": "long_code",
                "sms_registration_status": "approved",
                "required_terms": {"managed_line": "2026-11", "elevenlabs_eua": null},
                "premium_allowed": false, "max_lines": 1, "monthly_spend_limit": "100", "revision": 2
            }],
            "next_after_id": null
        })))
        .mount(&server)
        .await;
    let fields = |max_lines| EligibilityFields {
        business_name: Some("Jolly Chili Pte Ltd".into()),
        registration_id: Some("201912345K".into()),
        jurisdiction: Some("SG".into()),
        use_case: Some("restaurant bookings".into()),
        sms_sender_path: Some("long_code".into()),
        sms_registration_status: Some("approved".into()),
        terms_version: Some("2026-11".into()),
        max_lines,
        ..Default::default()
    };
    Mock::given(method("PUT"))
        .and(path(format!(
            "/api/v1/admin/managed-comms/eligibility/{OWNER}"
        )))
        .and(body_json(json!({
            "business_legal_name": "Jolly Chili Pte Ltd", "registration_id": "201912345K",
            "jurisdiction": "SG", "allowed_use_case": "restaurant bookings",
            "sms_sender_path": "long_code", "sms_registration_status": "approved",
            "required_terms": {"managed_line": "2026-11", "elevenlabs_eua": null},
            "premium_allowed": false, "max_lines": 1, "monthly_spend_limit": null,
            "expected_revision": null
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"owner_id": OWNER, "revision": 1})),
        )
        .expect(1)
        .mount(&server)
        .await;
    run(AdminLineCommands::Eligibility {
        command: EligibilityCommands::Grant {
            owner: "chef@example.com".into(),
            fields: fields(None),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    Mock::given(method("PUT"))
        .and(path(format!(
            "/api/v1/admin/managed-comms/eligibility/{OWNER}"
        )))
        .and(body_json(json!({
            "business_legal_name": "Jolly Chili Pte Ltd", "registration_id": "201912345K",
            "jurisdiction": "SG", "allowed_use_case": "restaurant bookings",
            "sms_sender_path": "long_code", "sms_registration_status": "approved",
            "required_terms": {"managed_line": "2026-11", "elevenlabs_eua": null},
            "premium_allowed": false, "max_lines": 3, "monthly_spend_limit": "100",
            "expected_revision": 2
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"owner_id": OWNER, "revision": 3})),
        )
        .expect(1)
        .mount(&server)
        .await;
    run(AdminLineCommands::Eligibility {
        command: EligibilityCommands::Update {
            owner: OWNER.into(),
            revision: 2,
            fields: EligibilityFields {
                max_lines: Some(3),
                ..Default::default()
            },
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
    assert!(
        run(AdminLineCommands::Eligibility {
            command: EligibilityCommands::Grant {
                owner: OWNER.into(),
                fields: EligibilityFields::default(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap_err()
        .to_string()
        .contains("--business-name")
    );
}

#[tokio::test]
async fn operations_report_and_release_use_their_routes() {
    let server = MockServer::start().await;
    for action in ["suspend", "resume"] {
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/admin/managed-lines/{LINE}/{action}")))
            .and(body_json(json!({"revision": 5})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "line_id": LINE, "claim_id": null, "revision": 6, "state": "suspended",
                "suspension": "admin", "live_calls": 1, "termination_pending": true
            })))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/api/v1/admin/managed-lines/report"))
        .and(query_param("from", "2026-11-01T00:00:00Z"))
        .and(query_param("until", "2026-11-08T00:00:00Z"))
        .and(query_param("line_id", LINE))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"lines": [], "next_after": null})),
        )
        .expect(1)
        .mount(&server)
        .await;
    for action in [AdminLineCommands::Suspend, AdminLineCommands::Resume] {
        run(action(RevisionArgs {
            line: LINE.into(),
            revision: 5,
            auth: mock_auth(server.uri()),
        }))
        .await
        .unwrap();
    }
    run(AdminLineCommands::Report {
        from: "2026-11-01".into(),
        until: "2026-11-08".into(),
        line: Some(LINE.into()),
        effects: false,
        after: None,
        auth: mock_auth(server.uri()),
    })
    .await
    .unwrap();
    assert!(
        run(AdminLineCommands::Release {
            line: LINE.into(),
            yes: false,
            request_id: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap_err()
        .to_string()
        .contains("--yes")
    );
    assert!(
        Cli::try_parse_from([
            "nyxid",
            "admin",
            "line",
            "report",
            "--from",
            "2026-11-01",
            "--until",
            "2026-11-08",
            "--effects"
        ])
        .is_err(),
        "effect detail needs a line"
    );
}

#[tokio::test]
async fn underwriting_refuses_unquoted_credit_numbers_before_sending() {
    let server = MockServer::start().await;
    for contents in [
        "components:\n  - kind: relay\n    credits_per_second: 0.0002\n    increment_seconds: 60\n    minimum_seconds: 60\nrolling_budget: \"500\"\n",
        "components: []\nrolling_budget: 500\n",
    ] {
        let f = file(contents);
        let error = run(AdminLineCommands::Account {
            command: AccountCommands::Underwriting {
                account: ACCOUNT.into(),
                revision: 1,
                file: f.path().into(),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("quoted decimal string"), "{error}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn credential_files_trim_only_trailing_line_endings_inside_the_zeroizing_buffer() {
    let read = |contents: &[u8]| {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents).unwrap();
        credential(None, Some(f.path().into())).map(|secret| secret.as_str().to_owned())
    };
    assert_eq!(read(b"AC123:secret\r\n\n").unwrap(), "AC123:secret");
    assert_eq!(read(b"  spaced secret \n").unwrap(), "  spaced secret ");
    assert_eq!(read(b"inner\nline").unwrap(), "inner\nline");
    assert!(read(b"\r\n").unwrap_err().to_string().contains("empty"));
    assert!(
        read(&[0xff, 0xfe, b'x'])
            .unwrap_err()
            .to_string()
            .contains("UTF-8")
    );
    let oversized = vec![b'a'; MAX_CREDENTIAL_BYTES as usize + 1];
    assert!(
        read(&oversized)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    let mut value = String::from("token\r\n");
    let capacity = value.capacity();
    trim_line_endings(&mut value);
    assert_eq!((value.as_str(), value.capacity()), ("token", capacity));
}

#[tokio::test]
async fn listings_follow_cursors_and_report_unreadable_records_once() {
    use crate::test_support::mock_auth_with_output;
    let server = MockServer::start().await;
    for (route, key) in [
        ("/api/v1/admin/managed-lines", "lines"),
        ("/api/v1/admin/managed-lines/accounts", "accounts"),
    ] {
        // The second page repeats one skipped ID and adds another.
        Mock::given(method("GET"))
            .and(path(route))
            .and(query_param("after", OWNER))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                key: [{"id": "second"}], "next_after_id": null,
                "premium_enabled": false, "unreadable_ids": [LINE, ACCOUNT],
            })))
            .with_priority(1)
            .expect(3)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                key: [{"id": "first"}], "next_after_id": OWNER,
                "premium_enabled": true, "unreadable_ids": [LINE],
            })))
            .expect(3)
            .mount(&server)
            .await;
    }
    let mut api = ApiClient::from_auth_checked(&mock_auth(server.uri()))
        .await
        .unwrap();
    for (route, key) in [
        ("/admin/managed-lines", "lines"),
        ("/admin/managed-lines/accounts", "accounts"),
    ] {
        let listing = admin_listing(&mut api, route, key).await.unwrap();
        assert_eq!(listing.rows.len(), 2);
        assert_eq!(listing.unreadable_ids, [LINE, ACCOUNT]);
        assert_eq!(listing.premium_enabled, json!(true));
        let output = listing.json(key);
        assert_eq!(output["unreadable_ids"], json!([LINE, ACCOUNT]));
        assert_eq!(output[key].as_array().unwrap().len(), 2);
        assert_eq!(
            unreadable_warning(&listing.unreadable_ids).unwrap(),
            format!(
                "Warning: 2 stored record(s) could not be read and were skipped; repair them: {LINE}, {ACCOUNT}"
            )
        );
    }
    assert!(unreadable_warning(&[]).is_none());
    // Both commands follow every cursor in JSON and table output.
    for output in [OutputFormat::Json, OutputFormat::Table] {
        run(AdminLineCommands::List(mock_auth_with_output(
            server.uri(),
            output,
        )))
        .await
        .unwrap();
        run(AdminLineCommands::Account {
            command: AccountCommands::List(mock_auth_with_output(server.uri(), output)),
        })
        .await
        .unwrap();
    }
}
