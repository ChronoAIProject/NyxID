use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request as HttpRequest, StatusCode},
};
use nyxid_permissions::{
    Engine, Error, ParameterConstraint, ParameterLocation, Policy, Request, ResourceBoundary,
    Response, Transport, ValueRule,
    drive::{DriveAdapter, FOLDER, FileMetadata, example_policy},
    mock::MockDrive,
    parse_json, parse_query, server,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use zeroize::Zeroizing;

const TEST_KEY: &str = "test-only-client-key-with-at-least-32-bytes";

struct WaitingTransport {
    entered: tokio::sync::Semaphore,
}

#[async_trait]
impl Transport for WaitingTransport {
    async fn send(&self, _request: &Request) -> Result<Response, Error> {
        self.entered.add_permits(1);
        std::future::pending().await
    }
}

#[tokio::test]
async fn concurrency_is_bounded_without_an_unbounded_wait_queue() {
    let transport = Arc::new(WaitingTransport {
        entered: tokio::sync::Semaphore::new(0),
    });
    let engine = engine(example_policy(), transport.clone());
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let engine = engine.clone();
        tasks.push(tokio::spawn(async move {
            engine.execute(Request::get("/drive/v3/files/report")).await
        }));
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        for _ in 0..32 {
            transport.entered.acquire().await.unwrap().forget();
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        engine.execute(Request::get("/drive/v3/files/report")).await,
        Err(Error::Busy)
    ));
    for task in tasks {
        task.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn total_request_deadline_bounds_resource_verification() {
    let transport = Arc::new(WaitingTransport {
        entered: tokio::sync::Semaphore::new(0),
    });
    let engine = engine(example_policy(), transport);
    let start = tokio::time::Instant::now();
    assert!(matches!(
        engine.execute(Request::get("/drive/v3/files/report")).await,
        Err(Error::Timeout)
    ));
    assert_eq!(start.elapsed(), std::time::Duration::from_secs(20));
}

fn engine(policy: Policy, drive: Arc<dyn Transport>) -> Arc<Engine> {
    Arc::new(Engine::new(policy, Arc::new(DriveAdapter), drive).unwrap())
}

fn setup() -> (Arc<Engine>, Arc<MockDrive>) {
    let drive = Arc::new(MockDrive::default());
    (engine(example_policy(), drive.clone()), drive)
}

fn write(method: Method, path: &str, body: Value) -> Request {
    Request {
        method,
        path: path.into(),
        query: BTreeMap::new(),
        body: serde_json::to_vec(&body).unwrap().into(),
        content_type: Some("application/json".into()),
    }
}

fn folder(parent: &str) -> Request {
    write(
        Method::POST,
        "/drive/v3/files",
        json!({"name":"New reports","mimeType":FOLDER,"parents":[parent]}),
    )
}

fn mutations(drive: &MockDrive) -> Vec<Request> {
    drive
        .requests()
        .into_iter()
        .filter(|r| r.method != Method::GET)
        .collect()
}

#[tokio::test]
async fn descendants_can_be_read_and_responses_do_not_expose_parent_or_secret_fields() {
    let (engine, drive) = setup();
    let result = engine
        .execute(Request::get("/drive/v3/files/report"))
        .await
        .unwrap();
    assert_eq!(
        parse_json(&result.body).unwrap(),
        json!({"id":"report","name":"October report.txt","mimeType":"text/plain"})
    );
    assert_eq!(
        drive.requests().len(),
        4,
        "three ancestry reads and one execution"
    );
    assert!(
        engine
            .execute(Request::get("/drive/v3/files/salary"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn live_ancestry_is_rechecked_after_a_file_is_moved() {
    let (engine, drive) = setup();
    engine
        .execute(Request::get("/drive/v3/files/report"))
        .await
        .unwrap();
    let mut file = drive.file("report").unwrap();
    file.parents = vec!["payroll".into()];
    drive.put(file);
    assert!(matches!(
        engine.execute(Request::get("/drive/v3/files/report")).await,
        Err(Error::Denied(_))
    ));
}

#[tokio::test]
async fn binary_downloads_and_exports_have_separate_operation_permissions() {
    let (engine, _) = setup();
    let mut download = Request::get("/drive/v3/files/report");
    download.query.insert("alt".into(), "media".into());
    let response = engine.execute(download.clone()).await.unwrap();
    assert_eq!(response.content_type, "application/octet-stream");
    assert!(response.body.starts_with(b"Demo content"));
    let mut export = Request::get("/drive/v3/files/report/export");
    export.query.insert("mimeType".into(), "text/plain".into());
    assert!(engine.execute(export.clone()).await.is_ok());
    let policy: Policy =
        serde_json::from_str(include_str!("../examples/drive-readonly-policy.json")).unwrap();
    let drive = Arc::new(MockDrive::default());
    let restricted = Engine::new(policy, Arc::new(DriveAdapter), drive.clone()).unwrap();
    assert!(restricted.execute(download).await.is_err());
    assert!(restricted.execute(export).await.is_err());
    assert!(drive.requests().is_empty());
}

#[tokio::test]
async fn rename_and_folder_creation_are_real_bounded_mutations() {
    let (engine, drive) = setup();
    engine
        .execute(write(
            Method::PATCH,
            "/drive/v3/files/report",
            json!({"name":"Renamed.txt"}),
        ))
        .await
        .unwrap();
    assert_eq!(drive.file("report").unwrap().name, "Renamed.txt");
    let result = engine.execute(folder("reports")).await.unwrap();
    let created = parse_json(&result.body).unwrap();
    assert_eq!(
        drive.file(created["id"].as_str().unwrap()).unwrap().parents,
        ["reports"]
    );
    assert_eq!(mutations(&drive).len(), 2);
}

#[tokio::test]
async fn outside_writes_and_boundary_folder_mutations_never_dispatch() {
    for request in [
        write(
            Method::PATCH,
            "/drive/v3/files/salary",
            json!({"name":"stolen"}),
        ),
        write(
            Method::PATCH,
            "/drive/v3/files/client-a",
            json!({"name":"renamed boundary"}),
        ),
        folder("payroll"),
        write(
            Method::POST,
            "/drive/v3/files",
            json!({"name":"unparented","mimeType":FOLDER}),
        ),
    ] {
        let (engine, drive) = setup();
        assert!(engine.execute(request).await.is_err());
        assert!(mutations(&drive).is_empty());
    }
}

#[tokio::test]
async fn unsupported_operations_and_fields_are_rejected_before_metadata_lookups() {
    let mut requests = vec![
        write(Method::DELETE, "/drive/v3/files/report", json!({})),
        write(
            Method::POST,
            "/drive/v3/files/report/copy",
            json!({"parents":["payroll"]}),
        ),
        write(
            Method::POST,
            "/drive/v3/files/report/permissions",
            json!({"type":"anyone"}),
        ),
        write(Method::POST, "/batch/drive/v3", json!({})),
        write(
            Method::POST,
            "/v4/spreadsheets/report:batchUpdate",
            json!({"requests":[]}),
        ),
        write(Method::PATCH, "/upload/drive/v3/files/report", json!({})),
        write(
            Method::PATCH,
            "/drive/v3/files/report",
            json!({"name":"ok","trashed":true}),
        ),
        write(
            Method::PATCH,
            "/drive/v3/files/report",
            json!({"name":"ok","parents":["payroll"]}),
        ),
        write(
            Method::POST,
            "/drive/v3/files",
            json!({"name":"shortcut","mimeType":"application/vnd.google-apps.shortcut","parents":["client-a"],"shortcutDetails":{"targetId":"salary"}}),
        ),
    ];
    for (key, value) in [
        ("addParents", "payroll"),
        ("removeParents", "reports"),
        ("_nyxid_via", "other"),
        ("access_token", "attacker"),
        ("key", "attacker"),
        ("callback", "jsonp"),
    ] {
        let mut request = write(
            Method::PATCH,
            "/drive/v3/files/report",
            json!({"name":"ok"}),
        );
        request.query.insert(key.into(), value.into());
        requests.push(request);
    }
    for request in requests {
        let (engine, drive) = setup();
        assert!(
            engine.execute(request.clone()).await.is_err(),
            "{request:?}"
        );
        assert!(drive.requests().is_empty(), "{request:?}");
    }
}

#[tokio::test]
async fn operation_and_parameter_constraints_are_checked_before_upstream_access() {
    let mut policy = example_policy();
    policy
        .allowed_operations
        .retain(|op| op != "drive.folders.create");
    let drive = Arc::new(MockDrive::default());
    let engine = engine(policy, drive.clone());
    assert!(engine.execute(folder("client-a")).await.is_err());
    assert!(
        engine
            .execute(write(
                Method::PATCH,
                "/drive/v3/files/report",
                json!({"name":"x".repeat(81)})
            ))
            .await
            .is_err()
    );
    let mut export = Request::get("/drive/v3/files/report/export");
    export.query.insert("mimeType".into(), "text/csv".into());
    assert!(
        engine.execute(export).await.is_err(),
        "provider supports CSV; this policy does not"
    );
    assert!(drive.requests().is_empty());
}

#[tokio::test]
async fn exact_and_required_body_constraints_apply_to_the_effective_request() {
    let mut policy = example_policy();
    policy.constraints.insert(
        "drive.files.rename".into(),
        vec![ParameterConstraint {
            location: ParameterLocation::Body,
            name: "/name".into(),
            required: true,
            rule: ValueRule::Exact {
                value: json!("Approved name"),
            },
        }],
    );
    let drive = Arc::new(MockDrive::default());
    let engine = engine(policy, drive.clone());
    assert!(
        engine
            .execute(write(
                Method::PATCH,
                "/drive/v3/files/report",
                json!({"name":"Other name"})
            ))
            .await
            .is_err()
    );
    assert!(drive.requests().is_empty());
    engine
        .execute(write(
            Method::PATCH,
            "/drive/v3/files/report",
            json!({"name":"Approved name"}),
        ))
        .await
        .unwrap();
    assert_eq!(drive.file("report").unwrap().name, "Approved name");
}

#[tokio::test]
async fn listing_is_rewritten_scoped_paginated_and_does_not_traverse_shortcuts() {
    let (engine, drive) = setup();
    let mut list = Request::get("/drive/v3/files");
    list.query.insert("pageSize".into(), "1".into());
    let first = parse_json(&engine.execute(list.clone()).await.unwrap().body).unwrap();
    assert_eq!(first["files"].as_array().unwrap().len(), 1);
    list.query.insert(
        "pageToken".into(),
        first["nextPageToken"].as_str().unwrap().into(),
    );
    let second = parse_json(&engine.execute(list).await.unwrap().body).unwrap();
    assert_ne!(first["files"][0]["id"], second["files"][0]["id"]);
    let all = parse_json(
        &engine
            .execute(Request::get("/drive/v3/files"))
            .await
            .unwrap()
            .body,
    )
    .unwrap();
    assert!(!all.to_string().contains("shortcut"));
    assert!(!all.to_string().contains("salary"));
    let dispatched = drive
        .requests()
        .into_iter()
        .find(|r| r.path == "/drive/v3/files")
        .unwrap();
    assert_eq!(
        dispatched.query["q"],
        "'client-a' in parents and trashed = false"
    );
    for q in [
        "'client-a' in parents or 'payroll' in parents",
        "name contains 'salary'",
        "'client-a' in parents and trashed = false",
        "'payroll' in parents",
    ] {
        let mut list = Request::get("/drive/v3/files");
        list.query.insert("q".into(), q.into());
        assert!(engine.execute(list).await.is_err(), "{q}");
    }
}

#[tokio::test]
async fn direct_children_policy_cannot_list_or_create_inside_subfolders() {
    let mut policy = example_policy();
    let ResourceBoundary::GoogleDriveFolder {
        include_descendants,
        ..
    } = &mut policy.resource
    else {
        panic!("Drive fixture")
    };
    *include_descendants = false;
    let drive = Arc::new(MockDrive::default());
    let engine = engine(policy, drive.clone());
    engine
        .execute(Request::get("/drive/v3/files/brief"))
        .await
        .unwrap();
    assert!(
        engine
            .execute(Request::get("/drive/v3/files/report"))
            .await
            .is_err()
    );
    let mut list = Request::get("/drive/v3/files");
    list.query.insert("q".into(), "'reports' in parents".into());
    assert!(engine.execute(list).await.is_err());
    assert!(engine.execute(folder("reports")).await.is_err());
    assert!(mutations(&drive).is_empty());
}

#[tokio::test]
async fn shortcuts_trashed_files_nonfolders_cycles_and_deep_ancestry_fail_closed() {
    for condition in [
        "shortcut",
        "trashed",
        "nonfolder",
        "cycle",
        "deep",
        "missing",
    ] {
        let (engine, drive) = setup();
        let id = match condition {
            "shortcut" => "shortcut-out",
            "trashed" => {
                let mut file = drive.file("report").unwrap();
                file.trashed = true;
                drive.put(file);
                "report"
            }
            "nonfolder" => {
                let mut file = drive.file("reports").unwrap();
                file.mime_type = "text/plain".into();
                drive.put(file);
                "report"
            }
            "cycle" => {
                let mut file = drive.file("reports").unwrap();
                file.parents = vec!["reports".into()];
                drive.put(file);
                "report"
            }
            "missing" => "missing",
            _ => {
                for index in 0..40 {
                    drive.put(FileMetadata {
                        id: format!("deep-{index}"),
                        name: "deep".into(),
                        mime_type: FOLDER.into(),
                        parents: vec![if index == 39 {
                            "client-a".into()
                        } else {
                            format!("deep-{}", index + 1)
                        }],
                        trashed: false,
                    });
                }
                "deep-0"
            }
        };
        assert!(
            engine
                .execute(Request::get(format!("/drive/v3/files/{id}")))
                .await
                .is_err(),
            "{condition}"
        );
        assert!(drive.requests().len() <= 32);
    }
}

#[tokio::test]
async fn noncanonical_paths_and_duplicate_json_never_reach_upstream() {
    let (engine, drive) = setup();
    for path in [
        "/drive/v3/files/%72eport",
        "/drive/v3/files/%2572eport",
        "/drive/v3/files/report/../salary",
        "/drive/v3/files//report",
        "/drive/v3/files/report:delete",
        "/drive/v3/files/report?alt=media",
        "https://evil.test/drive/v3/files/report",
    ] {
        assert!(engine.execute(Request::get(path)).await.is_err(), "{path}");
    }
    let mut request = write(
        Method::PATCH,
        "/drive/v3/files/report",
        json!({"name":"ok"}),
    );
    request.body = br#"{"name":"ok","name":"different"}"#.as_slice().into();
    assert!(engine.execute(request).await.is_err());
    assert!(drive.requests().is_empty());
}

#[test]
fn query_and_json_parsing_reject_ambiguity() {
    for query in [
        "alt=json&alt=media",
        "alt=json&%61lt=media",
        "q=%FF",
        "q=%",
        "q=%0X",
    ] {
        assert!(parse_query(query).is_err(), "{query}");
    }
    assert_eq!(
        parse_query("q=%27reports%27+in+parents").unwrap()["q"],
        "'reports' in parents"
    );
    assert!(parse_json(br#"{"a":{"b":1,"b":2}}"#).is_err());
    assert!(parse_json(br#"{"a":1} trailing"#).is_err());
}

struct BrokenTransport {
    mock: Arc<MockDrive>,
    mode: &'static str,
}

#[async_trait]
impl Transport for BrokenTransport {
    async fn send(&self, request: &Request) -> Result<Response, Error> {
        if self.mode == "write_failure" && request.method != Method::GET {
            self.mock.send(request).await?;
            return Err(Error::Upstream);
        }
        if request.path == "/drive/v3/files" && request.method == Method::GET {
            let mut response = self.mock.send(request).await?;
            response.body = match self.mode {
                "outside_row" => {
                    serde_json::to_vec(&json!({"files":[self.mock.file("salary").unwrap()]}))
                        .unwrap()
                        .into()
                }
                "incomplete" => br#"{"files":[],"incompleteSearch":true}"#.as_slice().into(),
                "oversize" => vec![0; nyxid_permissions::MAX_RESPONSE_BYTES + 1].into(),
                _ => return Err(Error::Upstream),
            };
            return Ok(response);
        }
        if self.mode == "metadata_failure" {
            return Err(Error::Upstream);
        }
        self.mock.send(request).await
    }
}

#[tokio::test]
async fn bad_upstream_pages_and_metadata_fail_closed() {
    for mode in ["outside_row", "incomplete", "oversize", "metadata_failure"] {
        let mock = Arc::new(MockDrive::default());
        let engine = engine(example_policy(), Arc::new(BrokenTransport { mock, mode }));
        assert!(
            engine
                .execute(Request::get("/drive/v3/files"))
                .await
                .is_err(),
            "{mode}"
        );
    }
}

#[tokio::test]
async fn ambiguous_write_is_not_retried_and_is_reported_as_uncertain() {
    let mock = Arc::new(MockDrive::default());
    let engine = engine(
        example_policy(),
        Arc::new(BrokenTransport {
            mock: mock.clone(),
            mode: "write_failure",
        }),
    );
    assert!(matches!(
        engine.execute(folder("client-a")).await,
        Err(Error::OutcomeUncertain)
    ));
    assert_eq!(mutations(&mock).len(), 1);
}

#[test]
fn invalid_policies_fail_at_startup() {
    for change in [
        "version",
        "provider",
        "root",
        "operation",
        "constraint",
        "pointer",
    ] {
        let mut policy = example_policy();
        match change {
            "version" => policy.version = 2,
            "provider" => policy.provider = "unknown".into(),
            "root" => {
                let ResourceBoundary::GoogleDriveFolder { root_folder_id, .. } =
                    &mut policy.resource
                else {
                    panic!("Drive fixture")
                };
                *root_folder_id = "root".into();
            }
            "operation" => policy.allowed_operations.push("drive.anything".into()),
            "constraint" => {
                policy.constraints.get_mut("drive.files.list").unwrap()[0].name = "unknown".into()
            }
            _ => {
                policy.constraints.get_mut("drive.files.rename").unwrap()[0].name = "/~2name".into()
            }
        }
        assert!(
            Engine::new(
                policy,
                Arc::new(DriveAdapter),
                Arc::new(MockDrive::default())
            )
            .is_err(),
            "{change}"
        );
    }
}

async fn call(
    router: &axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = HttpRequest::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {TEST_KEY}"))
        .header("Content-Type", "application/json")
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn rest_and_mcp_enforce_the_same_live_policy_and_mutations() {
    let (engine, drive) = setup();
    let router = server::router(engine, Zeroizing::new(TEST_KEY.into())).unwrap();
    for (id, allowed) in [("report", true), ("salary", false), ("shortcut-out", false)] {
        let (status, _) = call(
            &router,
            Method::GET,
            &format!("/drive/v3/files/{id}"),
            Value::Null,
        )
        .await;
        assert_eq!(status.is_success(), allowed);
        let (_, result) = call(&router, Method::POST, "/mcp", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"drive_request","arguments":{"method":"GET","path":format!("/drive/v3/files/{id}")}}})).await;
        assert_eq!(result["result"]["isError"], !allowed);
    }
    let (_, result) = call(&router, Method::POST, "/mcp", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"drive_request","arguments":{"method":"PATCH","path":"/drive/v3/files/report","body":{"name":"MCP renamed"}}}})).await;
    assert_eq!(result["result"]["isError"], false);
    assert_eq!(drive.file("report").unwrap().name, "MCP renamed");
    let (status, _) = call(
        &router,
        Method::GET,
        "/drive/v3/files/report?alt=json&alt=media",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &router,
        Method::GET,
        "/drive/v3/files/%72eport",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn authentication_browser_and_body_limits_apply_before_execution() {
    let (engine, drive) = setup();
    let router = server::router(engine, Zeroizing::new(TEST_KEY.into())).unwrap();
    for (key, origin, body, expected) in [
        ("wrong", false, String::new(), StatusCode::UNAUTHORIZED),
        (TEST_KEY, true, String::new(), StatusCode::FORBIDDEN),
        (
            TEST_KEY,
            false,
            "x".repeat(nyxid_permissions::MAX_BODY_BYTES + 1),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let mut request = HttpRequest::builder()
            .method(Method::PATCH)
            .uri("/drive/v3/files/report")
            .header("Authorization", format!("Bearer {key}"));
        if origin {
            request = request.header("Origin", "https://example.test");
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    assert!(drive.requests().is_empty());
}
