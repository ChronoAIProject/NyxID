use super::*;
use crate::test_support::mock_auth;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

async fn rpc_server(sse: bool, expired: Option<&'static str>, result: Value) -> MockServer {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST")).and(path("/mcp")).respond_with(move |request:&Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        let id=body["id"].clone();
        match body["method"].as_str().unwrap() {
            "initialize"=>ResponseTemplate::new(200).insert_header("Mcp-Session-Id","test-session").set_body_json(json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}})),
            "notifications/initialized"=>{
                assert_eq!(request.headers["mcp-session-id"],"test-session");ResponseTemplate::new(202)
            },
            "tools/list"|"tools/call"=>{
                assert_eq!(request.headers["mcp-session-id"],"test-session");
                assert_eq!(request.headers["mcp-protocol-version"],"2025-06-18");
                let n=calls.fetch_add(1,Ordering::SeqCst);
                if n==0 && let Some(expired)=expired {return match expired {
                    "404"=>ResponseTemplate::new(404),
                    "-32002"=>ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32002,"message":"Invalid or expired session"}})),
                    _=>panic!("expiry"),
                };}
                let message=json!({"jsonrpc":"2.0","id":id,"result":result});
                if sse {ResponseTemplate::new(200).set_body_raw(format!(": keepalive\r\n\r\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}}\r\n\r\ndata: {message}\r\n\r\n"), "text/event-stream")}else{ResponseTemplate::new(200).set_body_json(message)}
            },
            _=>panic!("unexpected method"),
        }
    }).mount(&server).await;
    server
}

#[tokio::test]
async fn mcp_tools_and_call_handshake_json_and_sse_keep_schemas_and_session_headers() {
    for sse in [false, true] {
        let server=rpc_server(sse,None,json!({"tools":[{"name":"telnyx-platform__chat_completions","inputSchema":{"type":"object"}}]})).await;
        run(McpCommands::Tools {
            service: Some("telnyx-platform".into()),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
        let methods = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| {
                serde_json::from_slice::<Value>(&request.body).unwrap()["method"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            ["initialize", "notifications/initialized", "tools/list"]
        );
        let server = rpc_server(
            sse,
            None,
            json!({"content":[{"type":"text","text":"OK"}],"isError":false}),
        )
        .await;
        run(McpCommands::Call {
            tool: "telnyx-platform__chat_completions".into(),
            args: "{}".into(),
            envelope: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn mcp_reinitializes_once_for_http_404_and_rpc_session_expiry() {
    for expired in ["404", "-32002"] {
        let server = rpc_server(false, Some(expired), json!({"content":[],"isError":false})).await;
        run(McpCommands::Call {
            tool: "tool".into(),
            args: "{}".into(),
            envelope: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
        let methods = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| {
                serde_json::from_slice::<Value>(&request.body).unwrap()["method"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            [
                "initialize",
                "notifications/initialized",
                "tools/call",
                "initialize",
                "notifications/initialized",
                "tools/call"
            ]
        );
    }
}

#[tokio::test]
async fn mcp_is_error_and_pending_have_distinct_exit_codes_and_no_replay() {
    let id = "00000000-0000-4000-8000-000000000001";
    for (text, exit) in [
        ("Tool rejected".to_string(), 1),
        (
            format!(
                "Managed execution {id} is pending usage verification; inspect its status and do not retry automatically"
            ),
            2,
        ),
    ] {
        let server = rpc_server(
            true,
            None,
            json!({"isError":true,"content":[{"type":"text","text":text}]}),
        )
        .await;
        let error = run(McpCommands::Call {
            tool: "tool".into(),
            args: "{}".into(),
            envelope: false,
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<super::super::admin_tools::PendingReadiness>()
                .unwrap()
                .exit_code,
            exit
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn mcp_invalid_args_and_agent_keys_never_send_a_request() {
    let server = MockServer::start().await;
    for raw in ["{", "[]", "@/nonexistent/nyxid-args"] {
        assert!(
            run(McpCommands::Call {
                tool: "tool".into(),
                args: raw.into(),
                envelope: false,
                auth: mock_auth(server.uri())
            })
            .await
            .is_err()
        );
    }
    let mut auth = mock_auth(server.uri());
    auth.access_token = Some("nyxid_ag_SECRET".into());
    let error = run(McpCommands::Tools {
        service: None,
        auth,
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("nyxid login"));
    assert!(!error.to_string().contains("SECRET"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn mcp_args_file_is_forwarded_through_universal_dispatcher() {
    let server = rpc_server(false, None, json!({"content":[],"isError":false})).await;
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "{\"model\":\"org/model:1\"}").unwrap();
    run(McpCommands::Call {
        tool: "tool".into(),
        args: format!("@{}", file.path().display()),
        envelope: false,
        auth: mock_auth(server.uri()),
    })
    .await
    .unwrap();
    let requests = server.received_requests().await.unwrap();
    let call: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert_eq!(call["params"]["name"], "nyx__call_tool");
    let forwarded: Value = serde_json::from_str(
        call["params"]["arguments"]["arguments_json"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(forwarded["model"], "org/model:1");
}

#[tokio::test]
async fn mcp_fresh_human_session_discovers_lazy_service_tools_without_execution() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/mcp")).respond_with(|request: &Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let result = match body["method"].as_str().unwrap() {
            "initialize" => json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}),
            "notifications/initialized" => return ResponseTemplate::new(202),
            "tools/list" => json!({"tools":[{"name":"nyx__search_tools","inputSchema":{"type":"object"}}]}),
            "tools/call" => {
                assert_eq!(body["params"],json!({"name":"nyx__search_tools","arguments":{"query":"telnyx-platform"}}));
                json!({"isError":false,"content":[{"type":"text","text":json!({"matches":[{"name":"telnyx-platform__chat_completions","inputSchema":{"type":"object","properties":{"model":{"enum":["org/model:1"]}}}},{"name":"other__tool"}]}).to_string()}]})
            }
            _ => panic!("unexpected method"),
        };
        ResponseTemplate::new(200).insert_header("Mcp-Session-Id","session").set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
    }).expect(4).mount(&server).await;
    run(McpCommands::Tools {
        service: Some("telnyx-platform".into()),
        auth: mock_auth(server.uri()),
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mcp_repeated_expiry_and_http_rpc_errors_stop_without_call_replay() {
    for (status, code, expected_requests) in [
        (404, None, 6),
        (200, Some(-32002), 6),
        (503, None, 3),
        (200, Some(-32603), 3),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/mcp")).respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            match body["method"].as_str().unwrap() {
                "initialize" => ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}})),
                "notifications/initialized" => ResponseTemplate::new(202),
                "tools/call" => ResponseTemplate::new(status).set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":code,"message":"rejected"}})),
                _ => panic!("unexpected method"),
            }
        }).expect(expected_requests).mount(&server).await;
        assert!(
            run(McpCommands::Call {
                tool: "telnyx-platform__chat_completions".into(),
                args: "{}".into(),
                envelope: false,
                auth: mock_auth(server.uri())
            })
            .await
            .is_err()
        );
    }
}
