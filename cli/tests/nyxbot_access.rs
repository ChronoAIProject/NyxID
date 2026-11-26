use serde_json::{Value, json};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

async fn command(
    server: &MockServer,
    verb: &str,
    format: &str,
    file: Option<&std::path::Path>,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nyxid"));
    cmd.args([
        "nyxbot",
        "access",
        verb,
        "nyxa-specialist",
        "--output",
        format,
        "--base-url",
        &server.uri(),
        "--access-token",
        "test-account-token",
    ]);
    if let Some(file) = file {
        cmd.arg("--file").arg(file);
    }
    cmd.env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env("CI", "1")
        .output()
        .await
        .unwrap()
}

#[tokio::test]
async fn nyxbot_access_round_trip_and_stale_revision() {
    let server = MockServer::start().await;
    let resource = "/api/v1/assistant/nyxagent/agents/nyxa-specialist/org-access";
    let before = json!({"maintainers":null,"users":[],"revision":4,"authoring_available":true});
    Mock::given(method("GET"))
        .and(path(resource))
        .respond_with(ResponseTemplate::new(200).set_body_json(&before))
        .expect(2)
        .mount(&server)
        .await;
    let show = command(&server, "show", "json", None).await;
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let policy: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(policy, before);
    let text = command(&server, "show", "table", None).await;
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("Maintainers: Inherit") && text.contains("Users: Nobody"));
    let draft = json!({"expected_revision":policy["revision"],"maintainers":[],"users":null});
    let temp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(temp.path(), draft.to_string()).unwrap();
    let after = json!({"maintainers":[],"users":null,"revision":5,"authoring_available":true});
    Mock::given(method("PUT"))
        .and(path(resource))
        .and(body_json(&draft))
        .respond_with(ResponseTemplate::new(200).set_body_json(&after))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    let saved = command(&server, "set", "json", Some(temp.path())).await;
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&saved.stdout).unwrap(),
        after
    );
    Mock::given(method("PUT"))
        .and(path(resource))
        .and(body_json(&draft))
        .respond_with(ResponseTemplate::new(409).set_body_json(
            json!({"error":"conflict","error_code":1004,"message":"stale revision"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let stale = command(&server, "set", "json", Some(temp.path())).await;
    assert!(!stale.status.success());
    let failure: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(failure["http_status"], 409);
    assert!(
        failure["guidance"]
            .as_str()
            .unwrap()
            .contains("revision is stale")
    );
    server.verify().await;
}
