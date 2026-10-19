use serde_json::json;
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn run(server: &MockServer, args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(args)
        .args(["--base-url", &server.uri(), "--access-token", "test-token"])
        .env("HOME", home.path())
        .env("CI", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_TELEMETRY_DSN")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .unwrap()
}

#[tokio::test]
async fn show_list_and_status_print_the_effective_platform_grant_once_and_preserve_json() {
    let id = "00000000-0000-4000-8000-000000000001";
    for allow_all in [false, true] {
        let server = MockServer::start().await;
        let key = json!({
            "id": id, "name": "platform-agent", "allow_all_services": allow_all,
            "allow_auto_connected_services": true, "allowed_service_ids": ["platform-id"],
            "allowed_services": [{ "id": "platform-id", "slug": "autoplatform", "auto_connected": true }]
        });
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/api-keys/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(&key))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/api-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [&key] })))
            .mount(&server)
            .await;
        for (endpoint, body) in [
            ("/users/me", json!({ "email": "person@example.test" })),
            ("/keys", json!({ "keys": [] })),
            ("/nodes", json!({ "nodes": [] })),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/api/v1{endpoint}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
        }
        for args in [
            vec!["api-key", "show", id],
            vec!["api-key", "list"],
            vec!["status"],
        ] {
            let output = run(&server, &args).await;
            let text = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{text}");
            assert_eq!(
                text.matches("Platform services: all auto-connected")
                    .count(),
                usize::from(!allow_all),
                "{text}"
            );
            assert_eq!(
                text.contains("autoplatform (Platform)"),
                !allow_all,
                "{text}"
            );
            if allow_all {
                assert!(text.contains("all"), "{text}");
            }
        }
        let output = run(&server, &["api-key", "show", id, "--output", "json"]).await;
        assert!(output.status.success());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            key
        );
    }
}

#[tokio::test]
async fn service_list_guidance_lists_platform_service_slugs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [{
            "id": "platform-id", "slug": "autoplatform", "label": "Platform Search",
            "is_active": true, "auto_connected": true
        }] })))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(&server, &["service", "list"]).await;
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("autoplatform"), "{text}");
}
