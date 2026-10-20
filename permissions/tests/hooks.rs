use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request as HttpRequest, StatusCode},
};
use nyxid_permissions::{
    Engine, Error, Policy, Request, Response, Transport,
    drive::{DriveAdapter, example_policy},
    hooks::{
        HookContext, HookDecision, HookDefinition, HookFailure, HookRegistry, HookStage,
        PermissionHook,
    },
    mock::MockDrive,
    parse_json, server,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use zeroize::Zeroizing;

const KEY: &str = "test-only-hook-client-credential-32-bytes";

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("nyxid-hook-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn definition(name: &str, handler: &str, stage: HookStage, config: Value) -> HookDefinition {
    HookDefinition {
        name: name.into(),
        handler: handler.into(),
        stage,
        operations: vec![],
        timeout_ms: 100,
        config,
    }
}

fn setup(hooks: Vec<HookDefinition>) -> (Arc<Engine>, Arc<MockDrive>) {
    let mut policy = example_policy();
    policy.hooks = hooks;
    let drive = Arc::new(MockDrive::default());
    (
        Arc::new(Engine::new(policy, Arc::new(DriveAdapter), drive.clone()).unwrap()),
        drive,
    )
}

fn rename(name: &str) -> Request {
    Request {
        method: Method::PATCH,
        path: "/drive/v3/files/report".into(),
        query: Default::default(),
        body: json!({"name":name}).to_string().into(),
        content_type: Some("application/json".into()),
    }
}

fn marker_hook() -> HookDefinition {
    definition(
        "confidential-output",
        "response_markers",
        HookStage::AfterResponse,
        json!({"markers":["CONFIDENTIAL"]}),
    )
}

#[tokio::test]
async fn pause_switch_changes_live_and_denies_before_any_upstream_call() {
    let directory = TempDir::new();
    let flag = directory.0.join("paused");
    let (engine, drive) = setup(vec![definition(
        "owner-pause",
        "pause_switch",
        HookStage::BeforeExecute,
        json!({"path":flag}),
    )]);
    engine
        .execute(Request::get("/drive/v3/files/report"))
        .await
        .unwrap();
    std::fs::write(&flag, "").unwrap();
    let calls = drive.requests().len();
    assert!(matches!(
        engine.execute(rename("paused")).await,
        Err(Error::HookDenied {
            stage: HookStage::BeforeExecute,
            ..
        })
    ));
    assert_eq!(drive.requests().len(), calls);
    assert_eq!(drive.file("report").unwrap().name, "October report.txt");
    std::fs::remove_file(flag).unwrap();
    engine.execute(rename("resumed")).await.unwrap();
    assert_eq!(drive.file("report").unwrap().name, "resumed");
}

#[tokio::test]
async fn pause_can_select_writes_while_reads_continue() {
    let directory = TempDir::new();
    let flag = directory.0.join("paused");
    std::fs::write(&flag, "").unwrap();
    let mut hook = definition(
        "pause-writes",
        "pause_switch",
        HookStage::BeforeExecute,
        json!({"path":flag}),
    );
    hook.operations = vec!["drive.files.rename".into(), "drive.folders.create".into()];
    let (engine, _) = setup(vec![hook]);
    engine
        .execute(Request::get("/drive/v3/files/report"))
        .await
        .unwrap();
    assert!(matches!(
        engine.execute(rename("blocked")).await,
        Err(Error::HookDenied { .. })
    ));
}

#[tokio::test]
async fn unavailable_pause_directory_is_not_treated_as_an_unset_flag() {
    let directory = TempDir::new();
    let (engine, drive) = setup(vec![definition(
        "pause",
        "pause_switch",
        HookStage::BeforeExecute,
        json!({"path":directory.0.join("missing-directory/flag")}),
    )]);
    assert!(matches!(
        engine.execute(rename("blocked")).await,
        Err(Error::HookUnavailable { .. })
    ));
    assert!(drive.requests().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn dangling_pause_symlink_still_pauses_execution() {
    let directory = TempDir::new();
    let flag = directory.0.join("paused");
    std::os::unix::fs::symlink(directory.0.join("absent"), &flag).unwrap();
    let (engine, drive) = setup(vec![definition(
        "pause",
        "pause_switch",
        HookStage::BeforeExecute,
        json!({"path":flag}),
    )]);
    assert!(matches!(
        engine.execute(rename("blocked")).await,
        Err(Error::HookDenied { .. })
    ));
    assert!(drive.requests().is_empty());
}

enum Behavior {
    Allow,
    Deny,
    Fail,
    Panic,
    Wait,
}
struct TestHook {
    label: &'static str,
    behavior: Behavior,
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PermissionHook for TestHook {
    fn validate(&self, _stage: HookStage, _config: &Value) -> Result<(), Error> {
        Ok(())
    }
    async fn check(
        &self,
        context: HookContext<'_>,
        _config: &Value,
    ) -> Result<HookDecision, HookFailure> {
        self.seen.lock().unwrap().push(self.label.into());
        assert_eq!(
            context
                .plan
                .request
                .query
                .get("supportsAllDrives")
                .map(String::as_str),
            Some("true")
        );
        if let Some(response) = context.response {
            let value = parse_json(&response.body).unwrap();
            assert!(
                value.get("parents").is_none(),
                "hooks only see sanitized provider output"
            );
        }
        match self.behavior {
            Behavior::Allow => Ok(HookDecision::Allow),
            Behavior::Deny => Ok(HookDecision::Deny),
            Behavior::Fail => Err(HookFailure),
            Behavior::Panic => panic!("synthetic hook failure"),
            Behavior::Wait => std::future::pending().await,
        }
    }
}

fn custom(policy: Policy, hooks: Vec<(&str, TestHook)>, drive: Arc<MockDrive>) -> Engine {
    let mut registry = HookRegistry::builtins();
    for (name, hook) in hooks {
        registry.register(name, Arc::new(hook)).unwrap();
    }
    Engine::new_with_hooks(policy, Arc::new(DriveAdapter), drive, registry).unwrap()
}

#[tokio::test]
async fn hooks_run_in_order_on_prepared_requests_and_sanitized_responses() {
    let seen = Arc::new(Mutex::new(vec![]));
    let mut policy = example_policy();
    policy.hooks = vec![
        definition("first", "first", HookStage::BeforeExecute, json!({})),
        definition("last", "last", HookStage::AfterResponse, json!({})),
    ];
    let engine = custom(
        policy,
        vec![
            (
                "first",
                TestHook {
                    label: "first",
                    behavior: Behavior::Allow,
                    seen: seen.clone(),
                },
            ),
            (
                "last",
                TestHook {
                    label: "last",
                    behavior: Behavior::Allow,
                    seen: seen.clone(),
                },
            ),
        ],
        Arc::new(MockDrive::default()),
    );
    engine
        .execute(Request::get("/drive/v3/files/report"))
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["first", "last"]);
}

#[tokio::test]
async fn hook_allow_cannot_override_operation_parameters_or_folder_scope() {
    let seen = Arc::new(Mutex::new(vec![]));
    let mut policy = example_policy();
    policy.hooks = vec![definition(
        "allow",
        "allow",
        HookStage::BeforeExecute,
        json!({}),
    )];
    let drive = Arc::new(MockDrive::default());
    let engine = custom(
        policy,
        vec![(
            "allow",
            TestHook {
                label: "allow",
                behavior: Behavior::Allow,
                seen: seen.clone(),
            },
        )],
        drive.clone(),
    );
    let mut delete = Request::get("/drive/v3/files/report");
    delete.method = Method::DELETE;
    assert!(engine.execute(delete).await.is_err());
    assert!(engine.execute(rename(&"x".repeat(81))).await.is_err());
    assert!(seen.lock().unwrap().is_empty());
    assert!(drive.requests().is_empty());
    assert!(matches!(
        engine.execute(Request::get("/drive/v3/files/salary")).await,
        Err(Error::Denied(_))
    ));
    assert_eq!(*seen.lock().unwrap(), ["allow"]);
}

#[tokio::test(start_paused = true)]
async fn denial_failure_panic_and_timeout_stop_before_dispatch() {
    for behavior in [
        Behavior::Deny,
        Behavior::Fail,
        Behavior::Panic,
        Behavior::Wait,
    ] {
        let mut policy = example_policy();
        policy.hooks = vec![definition(
            "check",
            "check",
            HookStage::BeforeExecute,
            json!({}),
        )];
        let drive = Arc::new(MockDrive::default());
        let seen = Arc::new(Mutex::new(vec![]));
        let engine = custom(
            policy,
            vec![(
                "check",
                TestHook {
                    label: "check",
                    behavior,
                    seen,
                },
            )],
            drive.clone(),
        );
        let start = tokio::time::Instant::now();
        assert!(matches!(
            engine.execute(rename("blocked")).await,
            Err(Error::HookDenied { .. } | Error::HookUnavailable { .. })
        ));
        assert!(start.elapsed() <= std::time::Duration::from_millis(100));
        assert!(drive.requests().is_empty());
    }
}

#[tokio::test]
async fn denied_hook_prevents_later_hooks_from_running() {
    let seen = Arc::new(Mutex::new(vec![]));
    let mut policy = example_policy();
    policy.hooks = vec![
        definition("deny", "deny", HookStage::BeforeExecute, json!({})),
        definition("later", "later", HookStage::BeforeExecute, json!({})),
    ];
    let drive = Arc::new(MockDrive::default());
    let engine = custom(
        policy,
        vec![
            (
                "deny",
                TestHook {
                    label: "deny",
                    behavior: Behavior::Deny,
                    seen: seen.clone(),
                },
            ),
            (
                "later",
                TestHook {
                    label: "later",
                    behavior: Behavior::Allow,
                    seen: seen.clone(),
                },
            ),
        ],
        drive.clone(),
    );
    assert!(engine.execute(rename("blocked")).await.is_err());
    assert_eq!(*seen.lock().unwrap(), ["deny"]);
    assert!(drive.requests().is_empty());
}

#[tokio::test]
async fn response_marker_blocks_delivery_without_returning_content() {
    let (engine, drive) = setup(vec![marker_hook()]);
    let mut file = drive.file("report").unwrap();
    file.name = "CONFIDENTIAL document".into();
    drive.put(file);
    let error = engine
        .execute(Request::get("/drive/v3/files/report"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::HookDenied {
            stage: HookStage::AfterResponse,
            ..
        }
    ));
    assert!(!error.to_string().contains("CONFIDENTIAL document"));
}

#[tokio::test]
async fn after_response_denial_does_not_claim_to_roll_back_an_accepted_write() {
    let (engine, drive) = setup(vec![marker_hook()]);
    let error = engine
        .execute(rename("CONFIDENTIAL document"))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::WriteAcceptedResponseWithheld));
    assert_eq!(drive.file("report").unwrap().name, "CONFIDENTIAL document");
    assert_eq!(
        drive
            .requests()
            .iter()
            .filter(|r| r.method == Method::PATCH)
            .count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn post_response_timeout_preserves_accepted_write_outcome() {
    let mut policy = example_policy();
    policy.hooks = vec![definition(
        "wait",
        "wait",
        HookStage::AfterResponse,
        json!({}),
    )];
    let drive = Arc::new(MockDrive::default());
    let engine = custom(
        policy,
        vec![(
            "wait",
            TestHook {
                label: "wait",
                behavior: Behavior::Wait,
                seen: Arc::new(Mutex::new(vec![])),
            },
        )],
        drive.clone(),
    );
    assert!(matches!(
        engine.execute(rename("accepted")).await,
        Err(Error::WriteAcceptedResponseWithheld)
    ));
    assert_eq!(drive.file("report").unwrap().name, "accepted");
}

struct LargeDownload(MockDrive);
#[async_trait]
impl Transport for LargeDownload {
    async fn send(&self, request: &Request) -> Result<Response, Error> {
        if request.query.get("alt").is_some_and(|s| s == "media") {
            let mut body = vec![b'x'; 64 * 1024 - 4];
            body.extend_from_slice(b"CONFIDENTIAL");
            body.extend_from_slice(&[b'x'; 10]);
            return Ok(Response {
                status: 200,
                content_type: "text/plain".into(),
                body: body.into(),
            });
        }
        self.0.send(request).await
    }
}

#[tokio::test]
async fn marker_detection_covers_chunk_boundaries_in_downloads() {
    let mut policy = example_policy();
    policy.hooks = vec![marker_hook()];
    let engine = Engine::new(
        policy,
        Arc::new(DriveAdapter),
        Arc::new(LargeDownload(MockDrive::default())),
    )
    .unwrap();
    let mut request = Request::get("/drive/v3/files/report");
    request.query.insert("alt".into(), "media".into());
    assert!(matches!(
        engine.execute(request).await,
        Err(Error::HookDenied { .. })
    ));
}

#[tokio::test]
async fn response_hook_has_rest_and_mcp_parity_and_no_content_leak() {
    let (engine, drive) = setup(vec![marker_hook()]);
    let mut file = drive.file("report").unwrap();
    file.name = "CONFIDENTIAL file contents".into();
    drive.put(file);
    let router = server::router(engine, Zeroizing::new(KEY.into())).unwrap();
    for mcp in [false, true] {
        let (method, path, body) = if mcp {
            (Method::POST, "/mcp", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"drive_request","arguments":{"method":"GET","path":"/drive/v3/files/report"}}}).to_string())
        } else {
            (Method::GET, "/drive/v3/files/report", String::new())
        };
        let response = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method(method)
                    .uri(path)
                    .header("Authorization", format!("Bearer {KEY}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if mcp {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("CONFIDENTIAL file contents"));
        if mcp {
            assert_eq!(parse_json(&bytes).unwrap()["result"]["isError"], true);
        }
    }
}

#[test]
fn bad_hook_configuration_is_rejected_at_startup() {
    for case in [
        "handler",
        "name",
        "timeout",
        "selector",
        "stage",
        "config",
        "duplicate",
        "limit",
    ] {
        let mut policy = example_policy();
        let mut hook = marker_hook();
        match case {
            "handler" => hook.handler = "unregistered".into(),
            "name" => hook.name = "unsafe\nname".into(),
            "timeout" => hook.timeout_ms = 2001,
            "selector" => hook.operations = vec!["drive.files.delete".into()],
            "stage" => hook.stage = HookStage::BeforeExecute,
            "config" => hook.config = json!({"markers":[],"unknown":true}),
            _ => {}
        }
        policy.hooks = vec![hook.clone()];
        if case == "duplicate" {
            policy.hooks.push(hook);
        }
        if case == "limit" {
            policy.hooks = vec![marker_hook(); 9];
        }
        assert!(
            Engine::new(
                policy,
                Arc::new(DriveAdapter),
                Arc::new(MockDrive::default())
            )
            .is_err(),
            "{case}"
        );
    }
}
