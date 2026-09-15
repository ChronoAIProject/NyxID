use base64::Engine;
use serde_json::{Value, json};
use std::{path::Path, process::Output};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

const APP: &str = "23456789-1234-4234-9234-123456789abc";
const ORG: &str = "34567890-1234-4234-9234-123456789abc";

fn seed_profile(root: &Path, server: &MockServer) -> String {
    let profile = root.join(".nyxid/profiles/app-connect-tests");
    std::fs::create_dir_all(&profile).unwrap();
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({
            "sub": "fixture-person", "exp": chrono::Utc::now().timestamp() + 3600,
        })
        .to_string(),
    );
    let token = format!("e30.{payload}.fixture-signature");
    std::fs::write(profile.join("access_token"), &token).unwrap();
    std::fs::write(profile.join("base_url"), server.uri()).unwrap();
    token
}

async fn run(root: &Path, args: &[&str], output: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
    command
        .env("HOME", root)
        .env("CI", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_URL")
        .env_remove("NYXID_BASE_URL")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_ACCESS_TOKEN")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_TELEMETRY_DSN")
        .env_remove("NYXID_SHARE_ANALYTICS")
        .args(args)
        .args(["--profile", "app-connect-tests", "--output", output])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    tokio::time::timeout(std::time::Duration::from_secs(45), command.output())
        .await
        .expect("CLI timed out or prompted")
        .expect("run CLI")
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON response")
}
fn manifest() -> Value {
    json!({ "enforcement": "advise", "requirements": [{
        "id": "source", "label": "Source code", "any_of_catalog_slugs": ["api-github"],
        "any_of_catalog_prefix": null, "owner_policy": "personal_only",
        "accepted_credential_types": ["oauth2"], "allow_master_credential": false,
        "allow_no_credential": false, "required_downstream_scopes": ["repo"],
        "validator": { "kind": "profile", "id": "github_user_v1" }, "optional": false,
    }] })
}

#[tokio::test]
async fn app_connect_cli_manifest_publish_json_roundtrip_and_enforcement_override() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let token = seed_profile(root.path(), &server);
    let file = root.path().join("manifest.json");
    std::fs::write(&file, manifest().to_string()).unwrap();
    for (index, enforcement) in ["advise", "gate"].iter().enumerate() {
        let mut body = manifest();
        body["enforcement"] = json!(enforcement);
        let response = json!({ "version": index + 1, "enforcement": enforcement,
            "requirements": body["requirements"], "compiled": {
                "catalog_service_ids": { "api-github": "catalog-id" },
                "validator_versions": { "github_user_v1": 1 },
            } });
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/developer/oauth-clients/{APP}/requirements"
            )))
            .and(header("authorization", format!("Bearer {token}")))
            .and(body_json(&body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .expect(1)
            .mount(&server)
            .await;
        let mut args = vec![
            "developer-app",
            "requirements",
            "publish",
            APP,
            "--file",
            file.to_str().unwrap(),
        ];
        if *enforcement == "gate" {
            args.extend(["--enforcement", "gate"]);
        }
        assert_eq!(success(run(root.path(), &args, "json").await), response);
    }
}

#[tokio::test]
async fn app_connect_cli_requirements_list_resolves_org_and_name_and_uses_stderr_table() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    seed_profile(root.path(), &server);
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/team"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "id": ORG, "slug": "team", "display_name": "Team" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/developer/oauth-clients"))
        .and(query_param("org_id", ORG))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "clients": [{ "id": APP, "client_name": "Sample App" }] })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path(format!("/api/v1/developer/oauth-clients/{APP}/requirements")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "versions": [{ "version": 3, "enforcement": "gate", "requirements": [], "published_at": "2026-09-15T00:00:00Z" }], "validator_profiles": [] })))
        .expect(1).mount(&server).await;
    let result = run(
        root.path(),
        &[
            "developer-app",
            "requirements",
            "list",
            "Sample App",
            "--org",
            "team",
        ],
        "table",
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let table = String::from_utf8_lossy(&result.stderr);
    assert!(table.contains("Version"));
    assert!(table.contains("gate"));
}

#[tokio::test]
async fn app_connect_cli_handoff_homepage_and_multipart_logo_use_exact_api_contracts() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    seed_profile(root.path(), &server);
    Mock::given(method("PATCH"))
        .and(path(format!(
            "/api/v1/developer/oauth-clients/{APP}/handoff"
        )))
        .and(body_json(
            json!({ "handoff_blurb": "Connect your accounts." }),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "handoff_blurb": "Connect your accounts." })),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        success(
            run(
                root.path(),
                &[
                    "developer-app",
                    "handoff",
                    "set",
                    APP,
                    "--text",
                    "Connect your accounts."
                ],
                "json"
            )
            .await
        )["handoff_blurb"],
        "Connect your accounts."
    );
    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/developer/oauth-clients/{APP}")))
        .and(body_json(json!({ "homepage_url": "https://app.example" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "branding_revision": 2 })))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        success(
            run(
                root.path(),
                &[
                    "developer-app",
                    "branding",
                    "homepage",
                    APP,
                    "--url",
                    "https://app.example"
                ],
                "json"
            )
            .await
        )["branding_revision"],
        2
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==").unwrap();
    let file = root.path().join("logo.png");
    std::fs::write(&file, &bytes).unwrap();
    Mock::given(method("POST"))
        .and(path(format!(
            "/api/v1/developer/oauth-clients/{APP}/branding/logo"
        )))
        .respond_with(move |request: &wiremock::Request| {
            assert!(
                request.headers["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with("multipart/form-data; boundary=")
            );
            assert!(
                request
                    .body
                    .windows(bytes.len())
                    .any(|window| window == bytes)
            );
            assert!(
                String::from_utf8_lossy(&request.body)
                    .contains("name=\"logo\"; filename=\"logo.png\"")
            );
            ResponseTemplate::new(200)
                .set_body_json(json!({ "logo_asset_id": "new-asset", "branding_revision": 3 }))
        })
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        success(
            run(
                root.path(),
                &[
                    "developer-app",
                    "branding",
                    "logo",
                    APP,
                    "--file",
                    file.to_str().unwrap()
                ],
                "json"
            )
            .await
        )["logo_asset_id"],
        "new-asset"
    );
}

#[tokio::test]
async fn app_connect_cli_admin_rollout_get_set_and_reset_json_roundtrip() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    seed_profile(root.path(), &server);
    let response = json!({ "effective": "disabled", "env_default": "disabled", "override_value": null, "allowed_org_ids": [ORG] });
    Mock::given(method("GET"))
        .and(path("/api/v1/admin/settings/app-connect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        success(
            run(
                root.path(),
                &["admin", "app-connect", "rollout", "get"],
                "json"
            )
            .await
        ),
        response
    );
    for mode in ["disabled", "allowlist", "reset"] {
        let rollout = if mode == "reset" { None } else { Some(mode) };
        let response = json!({ "effective": rollout.unwrap_or("disabled"), "env_default": "disabled", "override_value": rollout, "allowed_org_ids": [ORG] });
        Mock::given(method("PATCH"))
            .and(path("/api/v1/admin/settings/app-connect"))
            .and(body_json(json!({ "rollout": rollout })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            success(
                run(
                    root.path(),
                    &["admin", "app-connect", "rollout", "set", mode],
                    "json"
                )
                .await
            ),
            response
        );
    }
}

#[tokio::test]
async fn app_connect_cli_admin_capability_and_exact_branding_revision() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    seed_profile(root.path(), &server);
    for enabled in [true, false] {
        let response = json!({ "app_connect_capability_enabled": enabled });
        Mock::given(method("PATCH"))
            .and(path(format!(
                "/api/v1/admin/oauth-clients/{APP}/app-connect-capability"
            )))
            .and(body_json(json!({ "enabled": enabled })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            success(
                run(
                    root.path(),
                    &[
                        "admin",
                        "app-connect",
                        "capability",
                        APP,
                        if enabled { "--enable" } else { "--disable" }
                    ],
                    "json"
                )
                .await
            ),
            response
        );
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/admin/oauth-clients/{APP}/branding/verify"
            )))
            .and(body_json(
                json!({ "branding_revision": 7, "verified": enabled }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "verified": enabled })))
            .expect(1)
            .mount(&server)
            .await;
        let mut args = vec![
            "admin",
            "app-connect",
            "verify-branding",
            APP,
            "--revision",
            "7",
        ];
        if !enabled {
            args.push("--unverify");
        }
        assert_eq!(
            success(run(root.path(), &args, "json").await)["verified"],
            enabled
        );
    }
}

#[tokio::test]
async fn app_connect_cli_rejects_public_rollout_conflicting_flags_and_invalid_files() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec!["admin", "app-connect", "rollout", "set", "public"],
        vec![
            "admin",
            "app-connect",
            "capability",
            APP,
            "--enable",
            "--disable",
        ],
        vec!["admin", "app-connect", "capability", APP],
        vec!["admin", "app-connect", "verify-branding", APP],
    ] {
        assert!(!run(root.path(), &args, "json").await.status.success());
    }
    let logo = root.path().join("large.png");
    std::fs::write(&logo, vec![0; 256 * 1024 + 1]).unwrap();
    let output = run(
        root.path(),
        &[
            "developer-app",
            "branding",
            "logo",
            APP,
            "--file",
            logo.to_str().unwrap(),
        ],
        "json",
    )
    .await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds 262144 bytes"));
    let manifest = root.path().join("invalid.json");
    std::fs::write(&manifest, "[]").unwrap();
    let output = run(
        root.path(),
        &[
            "developer-app",
            "requirements",
            "publish",
            APP,
            "--file",
            manifest.to_str().unwrap(),
        ],
        "json",
    )
    .await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Manifest must be a JSON object"));
}
