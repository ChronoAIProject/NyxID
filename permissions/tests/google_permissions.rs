use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::Request as HttpRequest,
};
use http::Method;
use nyxid_permissions::{
    Engine, Error, Policy, Request, ResourceBoundary, Response, Transport, adapter_for_policy,
    google::{ApiOperation, is_google_api_origin},
    server,
};
use serde_json::{Value, json};
use tower::ServiceExt;

fn workspace() -> Policy {
    serde_json::from_str(include_str!("../examples/google-workspace-policy.json")).unwrap()
}

#[derive(Default)]
struct Google {
    calls: Mutex<Vec<(Request, String)>>,
    response: Mutex<Option<Response>>,
}

#[async_trait]
impl Transport for Google {
    async fn send(&self, _: &Request) -> Result<Response, Error> {
        panic!("generic calls must require an origin");
    }
    async fn send_to(&self, request: &Request, origin: Option<&str>) -> Result<Response, Error> {
        let origin = origin.expect("policy origin");
        assert!(is_google_api_origin(origin));
        self.calls
            .lock()
            .unwrap()
            .push((request.clone(), origin.into()));
        Ok(self.response.lock().unwrap().clone().unwrap_or(Response {
            status: 200,
            content_type: "application/json".into(),
            body: br#"{"result":"allowed"}"#.as_slice().into(),
        }))
    }
}

fn engine(policy: Policy, google: Arc<Google>) -> Engine {
    Engine::new(policy.clone(), adapter_for_policy(&policy).unwrap(), google).unwrap()
}

fn operations(policy: &mut Policy) -> &mut Vec<ApiOperation> {
    let ResourceBoundary::GoogleApi { operations } = &mut policy.resource else {
        panic!("Google fixture")
    };
    operations
}

fn request_for(operation: &ApiOperation) -> Request {
    let mut path = operation.path.clone();
    for (name, rule) in &operation.path_parameters {
        let nyxid_permissions::ValueRule::Exact { value } = rule else {
            panic!("exact fixture")
        };
        path = path.replace(&format!("{{{name}}}"), value.as_str().unwrap());
    }
    let mut request = Request::get(path);
    request.method = Method::from_bytes(operation.method.as_bytes()).unwrap();
    for (name, parameter) in &operation.query_parameters {
        let value = match &parameter.rule {
            nyxid_permissions::ValueRule::Exact { value } => value.as_str().unwrap().to_string(),
            nyxid_permissions::ValueRule::MaxInteger { value } => value.to_string(),
            _ => panic!("fixture rule"),
        };
        request.query.insert(name.clone(), value);
    }
    request
}

fn write_request() -> Request {
    let mut policy = workspace();
    let mut request = request_for(&operations(&mut policy)[6]);
    request.body = json!({"values":[["First", "Second"]]}).to_string().into();
    request.content_type = Some("application/json".into());
    request
}

#[tokio::test]
async fn all_workspace_products_share_the_engine_and_pin_distinct_origins() {
    let mut policy = workspace();
    let google = Arc::new(Google::default());
    let evaluator = engine(policy.clone(), google.clone());
    for op in &operations(&mut policy)[..6] {
        evaluator.execute(request_for(op)).await.unwrap();
        assert_eq!(google.calls.lock().unwrap().last().unwrap().1, op.origin);
    }
    evaluator.execute(write_request()).await.unwrap();
    assert_eq!(google.calls.lock().unwrap().len(), 7);
}

#[tokio::test]
async fn cloud_youtube_and_gemini_use_data_without_new_adapters() {
    for source in [
        include_str!("../examples/google-cloud-policy.json"),
        include_str!("../examples/google-youtube-policy.json"),
        include_str!("../examples/google-gemini-policy.json"),
    ] {
        let mut policy: Policy = serde_json::from_str(source).unwrap();
        let google = Arc::new(Google::default());
        let evaluator = engine(policy.clone(), google.clone());
        let op = &operations(&mut policy)[0];
        let mut request = request_for(op);
        if op.method == "POST" {
            request.body = json!({"contents":[{"role":"user","parts":[{"text":"Hello"}]}],"generationConfig":{"maxOutputTokens":128}}).to_string().into();
            request.content_type = Some("application/json".into());
        }
        evaluator.execute(request).await.unwrap();
        assert_eq!(google.calls.lock().unwrap()[0].1, op.origin);
    }
}

#[tokio::test]
async fn wrong_resource_method_or_unknown_api_never_reaches_transport() {
    let google = Arc::new(Google::default());
    let evaluator = engine(workspace(), google.clone());
    for path in [
        "/v1/documents/other",
        "/v1/documents/project-document:batchUpdate",
        "/gmail/v1/users/another/messages",
        "/drive/v3/files/report-file/permissions",
        "/calendar/v3/calendars/payroll@example.com/events",
        "/batch",
        "/upload/drive/v3/files",
    ] {
        assert!(
            matches!(
                evaluator.execute(Request::get(path)).await,
                Err(Error::Denied(_))
            ),
            "{path}"
        );
    }
    let mut write = write_request();
    write.method = Method::DELETE;
    assert!(evaluator.execute(write).await.is_err());
    assert!(google.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn canonical_paths_cannot_escape_exact_resource_or_operation() {
    let google = Arc::new(Google::default());
    let evaluator = engine(workspace(), google.clone());
    for path in [
        "https://docs.googleapis.com/v1/documents/project-document",
        "//v1/documents/project-document",
        "/v1//documents/project-document",
        "/v1/documents/../project-document",
        "/v1/documents/%70roject-document",
        "/v1/documents/project-document%2Fpermissions",
        "/v1/documents/project-document/",
        "/v1/documents/project-document?alt=media",
        "/v1/documents/project-document#extra",
        "/v1/documents/project-document\\extra",
    ] {
        assert!(
            evaluator.execute(Request::get(path)).await.is_err(),
            "{path}"
        );
    }
    assert!(google.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn query_limits_required_fields_and_system_overrides_are_enforced() {
    let mut policy = workspace();
    let request = request_for(&operations(&mut policy)[1]);
    let google = Arc::new(Google::default());
    let evaluator = engine(policy, google.clone());
    for (name, value) in [
        ("maxResults", "21"),
        ("labelIds", "Other"),
        ("q", "in:anywhere"),
        ("access_token", "caller-token"),
        ("key", "caller-key"),
        ("$httpMethod", "DELETE"),
        ("_nyxid_via", "other-connection"),
        ("alt", "media"),
        ("callback", "code"),
    ] {
        let mut changed = request.clone();
        changed.query.insert(name.into(), value.into());
        assert!(evaluator.execute(changed).await.is_err(), "{name}");
    }
    let mut missing = request;
    missing.query.remove("labelIds");
    assert!(evaluator.execute(missing).await.is_err());
    assert!(google.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn body_schema_limits_arrays_types_unknown_fields_and_ambiguous_json() {
    let google = Arc::new(Google::default());
    let evaluator = engine(workspace(), google.clone());
    for body in [
        json!({"values":[["a","b","c"]]}),
        json!({"values":[[true]]}),
        json!({"values":[["a"]],"range":"Elsewhere!A1"}),
        json!({"values":[["a".repeat(1001)]]}),
        json!({"values":vec![vec!["x"];11]}),
        json!({}),
        json!({"values":null}),
    ] {
        let mut request = write_request();
        request.body = body.to_string().into();
        assert!(evaluator.execute(request).await.is_err());
    }
    for (body, content_type) in [
        (r#"{"values":[],"values":[["second"]]}"#, "application/json"),
        (r#"{"values":[]}"#, "multipart/related"),
        ("", "application/json"),
    ] {
        let mut request = write_request();
        request.body = body.to_string().into();
        request.content_type = Some(content_type.into());
        assert!(evaluator.execute(request).await.is_err());
    }
    assert!(google.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn nested_gemini_schema_cannot_enable_tools_or_increase_generation_budget() {
    let policy: Policy =
        serde_json::from_str(include_str!("../examples/google-gemini-policy.json")).unwrap();
    let google = Arc::new(Google::default());
    let evaluator = engine(policy, google.clone());
    for body in [
        json!({"contents":[{"role":"user","parts":[{"text":"Hi","fileData":{"fileUri":"gs://other"}}]}],"generationConfig":{"maxOutputTokens":128}}),
        json!({"contents":[{"role":"user","parts":[{"text":"Hi"}]}],"generationConfig":{"maxOutputTokens":513}}),
        json!({"contents":[{"role":"user","parts":[{"text":"Hi"}]}],"generationConfig":{"maxOutputTokens":128},"tools":[{"googleSearch":{}}]}),
    ] {
        let mut request = Request::get("/models/gemini-2.5-flash:generateContent");
        request.method = Method::POST;
        request.content_type = Some("application/json".into());
        request.body = body.to_string().into();
        assert!(evaluator.execute(request).await.is_err());
    }
    assert!(google.calls.lock().unwrap().is_empty());
}

#[test]
fn google_origin_eligibility_rejects_spoofing_ports_and_non_origins() {
    for origin in [
        "https://www.googleapis.com",
        "https://sheets.googleapis.com",
        "https://us-central1-aiplatform.googleapis.com",
    ] {
        assert!(is_google_api_origin(origin));
    }
    for origin in [
        "http://www.googleapis.com",
        "https://www.googleapis.com:443",
        "https://www.googleapis.com:8443",
        "https://www.googleapis.com/",
        "https://www.googleapis.com.evil.test",
        "https://fakegoogleapis.com",
        "https://evil@www.googleapis.com",
        "https://www.googleapis.com?url=evil",
        "https://www.googleapis.com/v1",
        "https://*.googleapis.com",
        "https://127.0.0.1",
        "https://googleapis.com",
    ] {
        assert!(!is_google_api_origin(origin), "{origin}");
    }
}

#[test]
fn malformed_or_unbounded_policies_are_rejected_at_issuance() {
    for change in 0..10 {
        let mut policy = workspace();
        let ops = operations(&mut policy);
        match change {
            0 => ops[0].origin = "https://www.googleapis.com.evil.test".into(),
            1 => ops[0].path = "/drive/v3/files/*".into(),
            2 => {
                ops[0].path_parameters.clear();
            }
            3 => {
                ops[0].path_parameters.insert(
                    "file_id".into(),
                    nyxid_permissions::ValueRule::MaxLength { value: 256 },
                );
            }
            4 => {
                ops[0].query_parameters.insert(
                    "access_token".into(),
                    serde_json::from_value(json!({"rule":{"type":"max_length","value":100}}))
                        .unwrap(),
                );
            }
            5 => ops[0].id = ops[1].id.clone(),
            6 => ops[0].path = "/{file_id}/{file_id}".into(),
            7 => ops[0].body = Some(serde_json::from_value(json!({"type":"boolean"})).unwrap()),
            8 => {
                ops[6].body = Some(
                    serde_json::from_value(
                        json!({"type":"array","max_items":1001,"items":{"type":"boolean"}}),
                    )
                    .unwrap(),
                )
            }
            _ => ops[0].method = "CONNECT".into(),
        }
        assert!(
            Engine::new(
                policy.clone(),
                adapter_for_policy(&policy).unwrap(),
                Arc::new(Google::default())
            )
            .is_err(),
            "change {change}"
        );
    }
}

#[tokio::test]
async fn ambiguous_routes_do_not_choose_a_more_permissive_operation() {
    let mut policy = workspace();
    let mut duplicate = operations(&mut policy)[3].clone();
    duplicate.id = "docs.other".into();
    duplicate.origin = "https://sheets.googleapis.com".into();
    policy.allowed_operations.push(duplicate.id.clone());
    operations(&mut policy).push(duplicate);
    let google = Arc::new(Google::default());
    assert!(
        engine(policy, google.clone())
            .execute(Request::get("/v1/documents/project-document"))
            .await
            .is_err()
    );
    assert!(google.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn transports_without_origin_attestation_fail_closed() {
    struct OldTransport;
    #[async_trait]
    impl Transport for OldTransport {
        async fn send(&self, _: &Request) -> Result<Response, Error> {
            panic!("must not send")
        }
    }
    let policy = workspace();
    let engine = Engine::new(
        policy.clone(),
        adapter_for_policy(&policy).unwrap(),
        Arc::new(OldTransport),
    )
    .unwrap();
    assert!(
        engine
            .execute(Request::get("/v1/documents/project-document"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn hooks_apply_to_google_reads_and_writes_without_replaying_effects() {
    let mut policy = workspace();
    policy.hooks.push(serde_json::from_value(json!({"name":"sensitive-output","handler":"response_markers","stage":"after_response","config":{"markers":["allowed"]}})).unwrap());
    let google = Arc::new(Google::default());
    let evaluator = engine(policy, google.clone());
    assert!(matches!(
        evaluator
            .execute(Request::get("/v1/documents/project-document"))
            .await,
        Err(Error::HookDenied { .. })
    ));
    assert!(matches!(
        evaluator.execute(write_request()).await,
        Err(Error::WriteAcceptedResponseWithheld)
    ));
    assert_eq!(google.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn response_validation_rejects_html_duplicates_and_redirects() {
    for (status, content_type, body) in [
        (200, "text/html", "<html>bad</html>"),
        (200, "application/json", r#"{"a":1,"a":2}"#),
        (302, "application/json", "{}"),
    ] {
        let google = Arc::new(Google::default());
        *google.response.lock().unwrap() = Some(Response {
            status,
            content_type: content_type.into(),
            body: body.to_string().into(),
        });
        assert!(
            engine(workspace(), google)
                .execute(Request::get("/v1/documents/project-document"))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn rest_and_mcp_expose_and_enforce_the_same_google_policy() {
    let google = Arc::new(Google::default());
    let evaluator = Arc::new(engine(workspace(), google.clone()));
    let key = "google-test-client-key-with-32-bytes";
    let router = server::router(evaluator.clone(), key.to_string().into()).unwrap();
    for (path, allowed) in [
        ("/v1/documents/project-document", true),
        ("/v1/documents/elsewhere", false),
    ] {
        let rest = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let rpc = server::handle_mcp(&evaluator, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"google_request","arguments":{"method":"GET","path":path}}}).to_string().into()).await;
        let body: Value =
            serde_json::from_slice(&to_bytes(rpc.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(rest.status().is_success(), allowed);
        assert_eq!(body["result"]["isError"], !allowed);
    }
    let list = server::handle_mcp(
        &evaluator,
        br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.as_slice().into(),
    )
    .await;
    let body: Value =
        serde_json::from_slice(&to_bytes(list.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["result"]["tools"][0]["name"], "google_request");
    assert!(
        body["result"]["tools"][0]["description"]
            .as_str()
            .unwrap()
            .contains("docs.googleapis.com")
    );
    assert_eq!(google.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn mcp_distinguishes_an_explicit_json_null_from_an_absent_body() {
    let mut policy = workspace();
    let op = &mut operations(&mut policy)[6];
    op.body = Some(serde_json::from_value(json!({"type":"exact","value":null})).unwrap());
    let google = Arc::new(Google::default());
    let evaluator = engine(policy, google.clone());
    let mut request = write_request();
    request.body = "null".into();
    evaluator.execute(request.clone()).await.unwrap();
    let mut message = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"google_request","arguments":{"method":"PUT","path":request.path,"query":request.query,"body":null}
    }});
    for expected_error in [false, true] {
        let response = server::handle_mcp(&evaluator, message.to_string().into()).await;
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["result"]["isError"], expected_error);
        message["params"]["arguments"]
            .as_object_mut()
            .unwrap()
            .remove("body");
    }
    let calls = google.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|(request, _)| request.body == "null"));
}
