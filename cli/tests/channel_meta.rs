use serde_json::json;
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

#[tokio::test]
async fn manual_inbox_registration_ignores_lark_environment_credentials() {
    let api = MockServer::start().await;
    for platform in ["messenger", "instagram"] {
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots"))
            .and(body_json(json!({
                "platform": platform, "label": "Customer support", "app_id": "1001",
                "bot_token": "inbox-test-token", "app_secret": "inbox-test-signing-secret",
                "verification_token": "exclusive"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "inbox", "status": "pending_webhook"
            })))
            .expect(1)
            .mount(&api)
            .await;
        let result = Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args([
                "channel-bot",
                "register",
                "--platform",
                platform,
                "--label",
                "Customer support",
                "--app-id",
                "1001",
                "--token-env",
                "TEST_INBOX_TOKEN",
                "--app-secret-env",
                "TEST_INBOX_SECRET",
                "--verification-token",
                "exclusive",
                "--base-url",
                &api.uri(),
                "--access-token",
                "test-account-token",
                "--output",
                "json",
            ])
            .env("TEST_INBOX_TOKEN", "inbox-test-token")
            .env("TEST_INBOX_SECRET", "inbox-test-signing-secret")
            .env("NYXID_LARK_VERIFICATION_TOKEN", "unrelated-test-token")
            .env("NYXID_LARK_ENCRYPT_KEY", "unrelated-test-key")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env("CI", "1")
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
