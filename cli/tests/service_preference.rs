use serde_json::json;
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

const ACTIVE: &str = "11111111-1111-4111-8111-111111111111";
const OLD: &str = "22222222-2222-4222-8222-222222222222";

async fn run(server: &MockServer, args: &[&str]) -> std::process::Output {
    let config_dir = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(args)
        .args(["--base-url", &server.uri(), "--access-token", "test-token"])
        .env("HOME", config_dir.path())
        .env("CI", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_TELEMETRY_DSN")
        .env_remove("NYXID_SHARE_ANALYTICS")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .unwrap()
}

#[tokio::test]
async fn service_preference_cli_table_and_json_show_saved_ranks() {
    let server = MockServer::start().await;
    let preference = json!({"ordered":[ACTIVE],"version":7,"updated_at":null});
    Mock::given(method("GET"))
        .and(path("/api/v1/service-preferences"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&preference))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys":[
            {"id":ACTIVE,"slug":"ranked","label":"Ranked service","is_active":true,"preference_rank":1},
            {"id":OLD,"slug":"unranked","label":"Unranked service","is_active":true,"preference_rank":null}
        ]})))
        .expect(3)
        .mount(&server)
        .await;
    let output = run(&server, &["service", "list"]).await;
    let table = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{table}");
    assert!(table.contains("Pref"), "{table}");
    for (slug, rank) in [("ranked", "1"), ("unranked", "-")] {
        let cells: Vec<_> = table
            .lines()
            .find(|line| line.split('│').any(|cell| cell.trim() == slug))
            .unwrap()
            .split('│')
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .collect();
        assert_eq!(cells.last().copied(), Some(rank), "{table}");
    }
    let output = run(&server, &["service", "preference", "show"]).await;
    let table = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{table}");
    assert!(table.contains("Rank") && table.contains(ACTIVE), "{table}");
    assert!(!table.contains(OLD), "{table}");
    let output = run(
        &server,
        &["service", "preference", "show", "--output", "json"],
    )
    .await;
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        preference
    );
}

#[tokio::test]
async fn service_preference_cli_set_uses_active_slug_and_conflict_exits_nonzero() {
    for status in [200, 409] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/service-preferences"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ordered":[],"version":7,"updated_at":null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys":[
                {"id":OLD,"slug":"same","is_active":false},
                {"id":ACTIVE,"slug":"same","is_active":true}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/service-preferences"))
            .and(body_json(json!({"ordered":[ACTIVE],"expected_version":7})))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(if status == 200 {
                    json!({"ordered":[ACTIVE],"version":8,"updated_at":null})
                } else {
                    json!({"error":"conflict","error_code":1004,"message":"changed"})
                }),
            )
            .expect(1)
            .mount(&server)
            .await;
        let output = run(
            &server,
            &["service", "preference", "set", "same", "--output", "json"],
        )
        .await;
        if status == 409 {
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("preference order changed elsewhere; re-run")
            );
        } else {
            assert!(output.status.success());
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["ordered"],
                json!([ACTIVE])
            );
        }
    }
}
