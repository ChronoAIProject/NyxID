use serde_json::json;
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

const CATALOG: &str = "aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const GROUP: &str = "catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const ACTIVE: &str = "11111111-1111-4111-8111-111111111111";
const OLD: &str = "22222222-2222-4222-8222-222222222222";
const DISABLED: &str = "33333333-3333-4333-8333-333333333333";
const SSH: &str = "44444444-4444-4444-8444-444444444444";

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
    let preference =
        json!({"groups":[{"group":GROUP,"ordered":[ACTIVE]}],"version":7,"updated_at":null});
    Mock::given(method("GET"))
        .and(path("/api/v1/service-preferences"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&preference))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys":[
            {"catalog_service_id":CATALOG,"id":ACTIVE,"slug":"ranked","label":"Ranked service","service_type":"http","is_active":true,"preference_rank":1},
            {"catalog_service_id":CATALOG,"id":OLD,"slug":"unranked","label":"Unranked service","service_type":"http","is_active":true,"preference_rank":null}
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
            .find(|line| line.split(['│', '┆']).any(|cell| cell.trim() == slug))
            .unwrap_or_else(|| panic!("Missing {slug} row: {table}"))
            .split(['│', '┆'])
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .collect();
        assert_eq!(cells.last().copied(), Some(rank), "{table}");
    }
    let output = run(&server, &["service", "preference", "show"]).await;
    let table = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{table}");
    assert!(
        table.contains("Position") && table.contains(ACTIVE),
        "{table}"
    );
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
                    .set_body_json(json!({"groups":[],"version":7,"updated_at":null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys":[
                {"catalog_service_id":CATALOG,"id":OLD,"slug":"same","service_type":"http","is_active":false},
                {"catalog_service_id":CATALOG,"id":ACTIVE,"slug":"same","service_type":"http","is_active":true}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/service-preferences/groups/{GROUP}")))
            .and(body_json(json!({"ordered":[ACTIVE],"expected_version":7})))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(if status == 200 {
                    json!({"groups":[{"group":GROUP,"ordered":[ACTIVE]}],"version":8,"updated_at":null})
                } else {
                    json!({"error":"conflict","error_code":1004,"message":"changed"})
                }),
            )
            .expect(1)
            .mount(&server)
            .await;
        let output = run(
            &server,
            &[
                "service",
                "preference",
                "set",
                "--group",
                GROUP,
                "same",
                "--output",
                "json",
            ],
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
                serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["groups"][0]["ordered"],
                json!([ACTIVE])
            );
        }
    }
}

#[tokio::test]
async fn service_preference_cli_set_prints_returned_ranks_and_disabled_positions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/service-preferences"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "groups":[{"group":GROUP,"ordered":[ACTIVE,SSH,OLD,DISABLED]}],
            "version":7,"updated_at":null
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys":[
            {"catalog_service_id":CATALOG,"id":ACTIVE,"slug":"alpha","label":"Alpha","service_type":"http","is_active":true,"preference_rank":1,"preference_position":1},
            {"catalog_service_id":CATALOG,"id":OLD,"slug":"beta","label":"Beta","service_type":"http","is_active":true,"preference_rank":2,"preference_position":2},
            {"catalog_service_id":CATALOG,"id":DISABLED,"slug":"disabled","label":"Disabled","service_type":"http","is_active":false,"preference_rank":null,"preference_position":3},
            {"catalog_service_id":CATALOG,"id":SSH,"slug":"ssh","label":"SSH","service_type":"ssh","is_active":true,"preference_rank":null}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/service-preferences/groups/{GROUP}")))
        .and(body_json(
            json!({"ordered":[OLD,SSH,DISABLED,ACTIVE],"expected_version":7}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "groups":[{"group":GROUP,"ordered":[OLD,SSH,DISABLED,ACTIVE]}],
            "version":8,"updated_at":null
        })))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "service",
            "preference",
            "set",
            "--group",
            GROUP,
            "beta",
            "ssh",
            "disabled",
            "alpha",
        ],
    )
    .await;
    let table = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{table}");
    assert!(table.contains("Discovery"), "{table}");
    for (id, position, discovery) in [
        (OLD, "1", "1"),
        (SSH, "2", "-"),
        (DISABLED, "3", "saved:3"),
        (ACTIVE, "4", "2"),
    ] {
        let cells: Vec<_> = table
            .lines()
            .find(|line| line.split(['│', '┆']).any(|cell| cell.trim() == id))
            .unwrap_or_else(|| panic!("Missing {id} row: {table}"))
            .split(['│', '┆'])
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .collect();
        assert_eq!(cells.get(1).copied(), Some(position), "{table}");
        assert_eq!(cells.get(2).copied(), Some(discovery), "{table}");
    }
}

#[tokio::test]
async fn service_preference_cli_reset_release_confirmation_and_capacity() {
    let server = MockServer::start().await;
    let preference =
        json!({"groups":[{"group":GROUP,"ordered":[ACTIVE,OLD]}],"version":7,"updated_at":null});
    Mock::given(method("GET"))
        .and(path("/api/v1/service-preferences"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&preference))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/keys")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys":[{"id":ACTIVE,"slug":"ranked","catalog_service_id":CATALOG,"catalog_service_slug":"anthropic","service_type":"http","is_active":true,"preference_rank":1},{"id":OLD,"slug":"disabled","catalog_service_id":CATALOG,"catalog_service_slug":"anthropic","service_type":"http","is_active":false,"preference_rank":null,"preference_position":2}]}))).mount(&server).await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/service-preferences/groups/{GROUP}")))
        .and(body_json(json!({"ordered":[],"expected_version":7})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"groups":[],"version":8,"updated_at":null})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/service-preferences/hidden"))
        .and(body_json(json!({"expected_version":7})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&preference))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        &server,
        &[
            "service",
            "preference",
            "reset",
            "--group",
            "anthropic",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run(&server, &["service", "preference", "release-hidden"]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Use --yes"));
    let output = run(
        &server,
        &[
            "service",
            "preference",
            "release-hidden",
            "--yes",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run(
        &server,
        &["service", "preference", "show", "--group", "anthropic"],
    )
    .await;
    let table = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{table}");
    assert!(
        table.contains("saved:2") && table.contains("false"),
        "{table}"
    );
    Mock::given(method("PUT")).and(path(format!("/api/v1/service-preferences/groups/{GROUP}"))).and(body_json(json!({"ordered":[ACTIVE],"expected_version":7}))).respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"validation_error","error_code":1001,"message":"Agent order storage is full (200 connections across all services). Release unavailable preferences."}))).expect(1).mount(&server).await;
    let output = run(
        &server,
        &["service", "preference", "set", "--group", GROUP, "ranked"],
    )
    .await;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("200 connections across all services")
    );
    let output = run(&server, &["service", "preference", "set", "ranked"]).await;
    assert!(!output.status.success(), "flat set requires --group");
}
