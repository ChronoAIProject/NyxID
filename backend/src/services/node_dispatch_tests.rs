use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use mongodb::bson::doc;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
use crate::services::billing::route_inventory::internal_node_dispatch_permit;
use crate::services::internal_auth::InternalAuth;
use crate::services::node_dispatch::{NodeDispatch, internal_router};
use crate::services::node_owner_service::{self, ReplicaIdentity};
use crate::services::node_ws_manager::{
    CredentialAckOutcome, CredentialUpdateParams, NodeOutboundMessage, NodeProxyRequest,
    NodeProxyResponse, NodeSshExecRequest, NodeSshExecResult, NodeSshTunnelRequest,
    NodeWebTerminalAuthMode, NodeWebTerminalRequest, NodeWsManager, NodeWsProxyRequest,
    ProxyResponseType, SshTunnelChunk, StreamChunk, WebTerminalChunk, WsProxyFrame,
};

struct ReplicaFixture {
    db: mongodb::Database,
    node_id: String,
    owner_manager: Arc<NodeWsManager>,
    caller_dispatch: Arc<NodeDispatch>,
    outbound: mpsc::Receiver<NodeOutboundMessage>,
    pending_proxy: Arc<dashmap::DashMap<String, crate::services::node_ws_manager::PendingRequest>>,
    server_task: tokio::task::JoinHandle<()>,
}

impl Drop for ReplicaFixture {
    fn drop(&mut self) {
        self.server_task.abort();
    }
}

async fn two_replica_fixture(prefix: &str) -> Option<ReplicaFixture> {
    two_replica_fixture_with_limit(prefix, 2 * 1024 * 1024).await
}

async fn two_replica_fixture_with_limit(
    prefix: &str,
    internal_message_limit: usize,
) -> Option<ReplicaFixture> {
    let db = crate::test_utils::connect_test_database(prefix).await?;
    crate::services::coordination_service::ensure_indexes(&db)
        .await
        .unwrap();

    let node_id = uuid::Uuid::new_v4().to_string();
    db.collection::<Node>(NODES)
        .insert_one(test_node(&node_id))
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let owner_identity = Arc::new(ReplicaIdentity {
        instance_name: "backend-owner".to_string(),
        generation_id: uuid::Uuid::new_v4().to_string(),
        internal_base_url: format!("http://{address}"),
    });
    let caller_identity = Arc::new(ReplicaIdentity {
        instance_name: "backend-caller".to_string(),
        generation_id: uuid::Uuid::new_v4().to_string(),
        internal_base_url: "http://127.0.0.1:1".to_string(),
    });
    let connection_id = uuid::Uuid::new_v4().to_string();
    let owner_manager = Arc::new(NodeWsManager::new(5, 100));
    let (outbound_tx, outbound) = mpsc::channel(256);
    let pending_proxy =
        owner_manager.register_connection_with_id(&node_id, connection_id.clone(), outbound_tx);
    node_owner_service::claim(
        &db,
        &node_id,
        &owner_identity,
        &connection_id,
        Duration::from_secs(30),
    )
    .await
    .unwrap()
    .expect("owner claim");

    let auth = InternalAuth::new(
        db.clone(),
        Zeroizing::new([0x55; 32]),
        Duration::from_secs(30),
        Duration::from_secs(60),
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let owner_dispatch = Arc::new(NodeDispatch::new(
        db.clone(),
        owner_manager.clone(),
        owner_identity,
        client.clone(),
        auth.clone(),
        internal_message_limit,
        Duration::from_secs(3),
    ));
    let caller_dispatch = Arc::new(NodeDispatch::new(
        db.clone(),
        Arc::new(NodeWsManager::new(5, 100)),
        caller_identity,
        client,
        auth,
        internal_message_limit,
        Duration::from_secs(3),
    ));
    let app = internal_router(
        owner_dispatch,
        internal_message_limit,
        Duration::from_secs(3),
    );
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Some(ReplicaFixture {
        db,
        node_id,
        owner_manager,
        caller_dispatch,
        outbound,
        pending_proxy,
        server_task,
    })
}

fn test_node(id: &str) -> Node {
    let now = Utc::now();
    Node {
        id: id.to_string(),
        user_id: uuid::Uuid::new_v4().to_string(),
        name: "two-replica-node".to_string(),
        status: NodeStatus::Offline,
        auth_token_hash: "auth-hash".to_string(),
        signing_secret_encrypted: None,
        signing_secret_hash: "signing-hash".to_string(),
        last_heartbeat_at: None,
        connected_at: None,
        metadata: None,
        metrics: NodeMetrics::default(),
        connection_owner: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

fn proxy_request(request_id: &str) -> NodeProxyRequest {
    NodeProxyRequest {
        request_id: request_id.to_string(),
        service_id: "service-id".to_string(),
        service_slug: "service-slug".to_string(),
        base_url: "https://service.invalid".to_string(),
        method: "GET".to_string(),
        path: "/v1/data".to_string(),
        query: None,
        headers: vec![],
        body: None,
    }
}

async fn next_outbound(outbound: &mut mpsc::Receiver<NodeOutboundMessage>) -> String {
    let message = tokio::time::timeout(Duration::from_secs(3), outbound.recv())
        .await
        .expect("node outbound timeout")
        .expect("node outbound channel closed");
    match message {
        NodeOutboundMessage::Text(text) => text,
        NodeOutboundMessage::Close { code, reason } => {
            panic!("unexpected close frame {code}: {reason}")
        }
    }
}

fn message_json(message: &str) -> serde_json::Value {
    serde_json::from_str(message).expect("node outbound JSON")
}

#[tokio::test]
async fn remote_complete_proxy_preserves_response_and_request_identity() {
    let Some(mut fixture) = two_replica_fixture("node_dispatch_complete").await else {
        return;
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_request_id = request_id.clone();
    let request_task = tokio::spawn(async move {
        dispatch
            .send_proxy_request_classified(
                &node_id,
                proxy_request(&sent_request_id),
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });

    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["type"], "proxy_request");
    assert_eq!(outbound["request_id"], request_id);
    fixture.owner_manager.deliver_proxy_response(
        &fixture.node_id,
        NodeProxyResponse {
            request_id: request_id.clone(),
            status: 201,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{"ok":true}"#.to_vec(),
        },
    );

    let response = request_task
        .await
        .unwrap()
        .unwrap_or_else(|failure| panic!("remote proxy failed: {}", failure.error));
    let ProxyResponseType::Complete(response) = response else {
        panic!("expected complete response");
    };
    assert_eq!(response.request_id, request_id);
    assert_eq!(response.status, 201);
    assert_eq!(response.body, br#"{"ok":true}"#);
}

#[tokio::test]
async fn remote_proxy_preserves_high_byte_body_within_raw_limit() {
    let Some(mut fixture) =
        two_replica_fixture_with_limit("node_dispatch_large_body", 2 * 1024 * 1024).await
    else {
        return;
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let expected_body = vec![0xff; 1024 * 1024];
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_request_id = request_id.clone();
    let sent_body = expected_body.clone();
    let request_task = tokio::spawn(async move {
        let mut request = proxy_request(&sent_request_id);
        request.body = Some(sent_body);
        dispatch
            .send_proxy_request_classified(&node_id, request, None, internal_node_dispatch_permit())
            .await
    });

    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    let forwarded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        outbound["body"].as_str().expect("forwarded base64 body"),
    )
    .unwrap();
    assert_eq!(forwarded, expected_body);
    fixture.owner_manager.deliver_proxy_response(
        &fixture.node_id,
        NodeProxyResponse {
            request_id,
            status: 204,
            headers: vec![],
            body: vec![],
        },
    );
    assert!(matches!(
        request_task
            .await
            .unwrap()
            .unwrap_or_else(|failure| { panic!("remote proxy failed: {}", failure.error) }),
        ProxyResponseType::Complete(_)
    ));
}

#[tokio::test]
async fn remote_proxy_streams_before_completion_and_cancels_owner_work() {
    let Some(mut fixture) = two_replica_fixture("node_dispatch_stream").await else {
        return;
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_request_id = request_id.clone();
    let request_task = tokio::spawn(async move {
        dispatch
            .send_proxy_request_classified(
                &node_id,
                proxy_request(&sent_request_id),
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });

    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["request_id"], request_id);
    assert!(fixture.owner_manager.deliver_stream_start(
        &fixture.node_id,
        &request_id,
        200,
        vec![("content-type".to_string(), "text/event-stream".to_string())],
    ));

    let response = request_task
        .await
        .unwrap()
        .unwrap_or_else(|failure| panic!("remote stream failed: {}", failure.error));
    let ProxyResponseType::Streaming(mut stream) = response else {
        panic!("expected streaming response");
    };
    assert!(matches!(
        stream.recv().await,
        Some(StreamChunk::Start { status: 200, .. })
    ));
    fixture.owner_manager.deliver_stream_chunk(
        &fixture.node_id,
        &request_id,
        b"first\n\n".to_vec(),
    );
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), stream.recv()).await,
        Ok(Some(StreamChunk::Data(data))) if data == b"first\n\n"
    ));
    assert!(fixture.pending_proxy.contains_key(&request_id));

    drop(stream);
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.pending_proxy.contains_key(&request_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner-side proxy request was not cancelled");
}

#[tokio::test]
async fn remote_proxy_cancellation_before_headers_clears_owner_work() {
    let Some(mut fixture) = two_replica_fixture("node_dispatch_preheader_cancel").await else {
        return;
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_request_id = request_id.clone();
    let request_task = tokio::spawn(async move {
        dispatch
            .send_proxy_request_classified(
                &node_id,
                proxy_request(&sent_request_id),
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });

    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["request_id"], request_id);
    assert!(
        fixture
            .owner_manager
            .has_pending_proxy_request(&fixture.node_id, &request_id)
    );

    request_task.abort();
    let cleared = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !fixture
                .owner_manager
                .has_pending_proxy_request(&fixture.node_id, &request_id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(cleared.is_ok(), "owner pending work was not cancelled");
}

#[tokio::test]
async fn remote_ssh_exec_and_duplex_sessions_forward_both_directions() {
    let Some(mut fixture) = two_replica_fixture("node_dispatch_duplex").await else {
        return;
    };

    let exec_id = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_exec_id = exec_id.clone();
    let exec_task = tokio::spawn(async move {
        dispatch
            .exec_ssh_command(
                &node_id,
                NodeSshExecRequest {
                    request_id: sent_exec_id,
                    host: "host.internal".to_string(),
                    port: 22,
                    principal: "user".to_string(),
                    private_key_pem: "private".to_string(),
                    certificate_openssh: "certificate".to_string(),
                    command: "id".to_string(),
                    timeout_secs: 5,
                },
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });
    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["type"], "ssh_exec");
    fixture.owner_manager.deliver_ssh_exec_result(
        &fixture.node_id,
        NodeSshExecResult {
            request_id: exec_id,
            exit_code: 0,
            stdout: "uid=1000".to_string(),
            stderr: String::new(),
            duration_ms: 10,
            timed_out: false,
            error: None,
            error_code: None,
        },
    );
    assert_eq!(exec_task.await.unwrap().unwrap().stdout, "uid=1000");

    let ssh_session = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_session = ssh_session.clone();
    let open_task = tokio::spawn(async move {
        dispatch
            .open_ssh_tunnel(
                &node_id,
                NodeSshTunnelRequest {
                    session_id: sent_session,
                    service_id: "ssh-service".to_string(),
                    host: "host.internal".to_string(),
                    port: 22,
                },
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });
    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["type"], "ssh_tunnel_open");
    assert!(
        fixture
            .owner_manager
            .deliver_ssh_tunnel_opened(&fixture.node_id, &ssh_session)
    );
    let mut ssh_rx = open_task.await.unwrap().unwrap();

    fixture
        .caller_dispatch
        .send_ssh_tunnel_data(&fixture.node_id, &ssh_session, b"client-data")
        .unwrap();
    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["type"], "ssh_tunnel_data");
    fixture.owner_manager.deliver_ssh_tunnel_data(
        &fixture.node_id,
        &ssh_session,
        b"server-data".to_vec(),
    );
    assert!(matches!(
        ssh_rx.recv().await,
        Some(SshTunnelChunk::Data(data)) if data == b"server-data"
    ));
    fixture
        .caller_dispatch
        .close_ssh_tunnel(&fixture.node_id, &ssh_session)
        .unwrap();
    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["type"], "ssh_tunnel_close");

    let terminal_session = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_session = terminal_session.clone();
    let terminal_task = tokio::spawn(async move {
        dispatch
            .open_web_terminal(
                &node_id,
                NodeWebTerminalRequest {
                    session_id: sent_session,
                    service_id: "ssh-service".to_string(),
                    service_slug: "ssh".to_string(),
                    auth_mode: NodeWebTerminalAuthMode::NodeKey,
                    host: "host.internal".to_string(),
                    port: 22,
                    principal: "user".to_string(),
                    cols: 80,
                    rows: 24,
                },
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });
    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(outbound["type"], "web_terminal_open");
    assert!(
        fixture
            .owner_manager
            .deliver_web_terminal_started(&fixture.node_id, &terminal_session)
    );
    let mut terminal_rx = terminal_task.await.unwrap().unwrap();
    fixture
        .caller_dispatch
        .send_web_terminal_data(&fixture.node_id, &terminal_session, b"ls\n")
        .unwrap();
    assert_eq!(
        message_json(&next_outbound(&mut fixture.outbound).await)["type"],
        "web_terminal_data"
    );
    fixture
        .caller_dispatch
        .send_web_terminal_resize(&fixture.node_id, &terminal_session, 120, 40)
        .unwrap();
    assert_eq!(
        message_json(&next_outbound(&mut fixture.outbound).await)["type"],
        "web_terminal_resize"
    );
    fixture.owner_manager.deliver_web_terminal_data(
        &fixture.node_id,
        &terminal_session,
        b"output".to_vec(),
    );
    assert!(matches!(
        terminal_rx.recv().await,
        Some(WebTerminalChunk::Data(data)) if data == b"output"
    ));
    fixture
        .caller_dispatch
        .close_web_terminal(&fixture.node_id, &terminal_session)
        .unwrap();
    assert_eq!(
        message_json(&next_outbound(&mut fixture.outbound).await)["type"],
        "web_terminal_close"
    );
}

#[tokio::test]
async fn remote_ws_passthrough_forwards_text_binary_and_close() {
    let Some(mut fixture) = two_replica_fixture("node_dispatch_ws_proxy").await else {
        return;
    };
    let session_id = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_session = session_id.clone();
    let open_task = tokio::spawn(async move {
        dispatch
            .open_ws_proxy(
                &node_id,
                NodeWsProxyRequest {
                    session_id: sent_session,
                    service_slug: "socket".to_string(),
                    base_url: "wss://socket.invalid".to_string(),
                    path: "/events".to_string(),
                    query: None,
                    headers: vec![],
                    ws_frame_injections: vec![],
                },
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });
    assert_eq!(
        message_json(&next_outbound(&mut fixture.outbound).await)["type"],
        "ws_proxy_open"
    );
    assert!(fixture.owner_manager.deliver_ws_proxy_opened(
        &fixture.node_id,
        &session_id,
        Some("chat".to_string())
    ));
    let mut session = open_task.await.unwrap().unwrap();
    assert_eq!(session.selected_protocol.as_deref(), Some("chat"));

    fixture
        .caller_dispatch
        .send_ws_proxy_text(&fixture.node_id, &session_id, "hello")
        .unwrap();
    assert_eq!(
        message_json(&next_outbound(&mut fixture.outbound).await)["type"],
        "ws_proxy_text"
    );
    fixture
        .caller_dispatch
        .send_ws_proxy_binary(&fixture.node_id, &session_id, b"binary")
        .unwrap();
    assert_eq!(
        message_json(&next_outbound(&mut fixture.outbound).await)["type"],
        "ws_proxy_binary"
    );
    fixture
        .owner_manager
        .deliver_ws_proxy_text(&fixture.node_id, &session_id, "reply".to_string());
    fixture.owner_manager.deliver_ws_proxy_binary(
        &fixture.node_id,
        &session_id,
        b"reply-bin".to_vec(),
    );
    assert!(
        matches!(session.frames.recv().await, Some(WsProxyFrame::Text(data)) if data == "reply")
    );
    assert!(
        matches!(session.frames.recv().await, Some(WsProxyFrame::Binary(data)) if data == b"reply-bin")
    );

    fixture
        .caller_dispatch
        .send_ws_proxy_close(
            &fixture.node_id,
            &session_id,
            Some(1000),
            Some("done".to_string()),
        )
        .unwrap();
    let close = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(close["type"], "ws_proxy_close");
    assert_eq!(close["code"], 1000);
}

#[tokio::test]
async fn remote_ws_passthrough_preserves_large_binary_frames() {
    let Some(mut fixture) =
        two_replica_fixture_with_limit("node_dispatch_ws_large", 8 * 1024 * 1024).await
    else {
        return;
    };
    let session_id = uuid::Uuid::new_v4().to_string();
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let sent_session = session_id.clone();
    let open_task = tokio::spawn(async move {
        dispatch
            .open_ws_proxy(
                &node_id,
                NodeWsProxyRequest {
                    session_id: sent_session,
                    service_slug: "socket".to_string(),
                    base_url: "wss://socket.invalid".to_string(),
                    path: "/events".to_string(),
                    query: None,
                    headers: vec![],
                    ws_frame_injections: vec![],
                },
                None,
                internal_node_dispatch_permit(),
            )
            .await
    });
    let _ = next_outbound(&mut fixture.outbound).await;
    assert!(
        fixture
            .owner_manager
            .deliver_ws_proxy_opened(&fixture.node_id, &session_id, None)
    );
    let mut session = open_task.await.unwrap().unwrap();
    let payload = vec![0xff; 5 * 1024 * 1024];

    fixture
        .caller_dispatch
        .send_ws_proxy_binary(&fixture.node_id, &session_id, &payload)
        .unwrap();
    let outbound = message_json(&next_outbound(&mut fixture.outbound).await);
    let forwarded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        outbound["data"].as_str().expect("forwarded base64 frame"),
    )
    .unwrap();
    assert_eq!(forwarded, payload);

    fixture
        .owner_manager
        .deliver_ws_proxy_binary(&fixture.node_id, &session_id, payload.clone());
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), session.frames.recv()).await,
        Ok(Some(WsProxyFrame::Binary(data))) if data == payload
    ));
}

#[tokio::test]
async fn unsigned_internal_duplex_handshake_is_rejected_before_upgrade() {
    let Some(fixture) = two_replica_fixture("node_dispatch_unsigned_duplex").await else {
        return;
    };
    let stored = fixture
        .db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": &fixture.node_id })
        .await
        .unwrap()
        .unwrap();
    let base_url = stored.connection_owner.unwrap().internal_base_url;
    let ws_url = format!(
        "ws://{}/internal/v1/nodes/{}/duplex",
        base_url.trim_start_matches("http://"),
        fixture.node_id
    );

    let error = tokio_tungstenite::connect_async(ws_url).await.unwrap_err();
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected HTTP handshake rejection");
    };
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn remote_credential_ack_and_admin_disconnect_reach_owner() {
    let Some(mut fixture) = two_replica_fixture("node_dispatch_commands").await else {
        return;
    };
    let dispatch = fixture.caller_dispatch.clone();
    let node_id = fixture.node_id.clone();
    let update_task = tokio::spawn(async move {
        dispatch
            .send_credential_update_and_wait(
                &node_id,
                &CredentialUpdateParams {
                    service_slug: "service".to_string(),
                    injection_method: "header".to_string(),
                    header_name: Some("Authorization".to_string()),
                    header_value: Some("secret".to_string()),
                    param_name: None,
                    param_value: None,
                    target_url: None,
                },
                Duration::from_secs(2),
            )
            .await
    });
    let update = message_json(&next_outbound(&mut fixture.outbound).await);
    assert_eq!(update["type"], "credential_update");
    let request_id = update["request_id"].as_str().unwrap();
    fixture.owner_manager.deliver_credential_ack(
        &fixture.node_id,
        request_id,
        CredentialAckOutcome::Ok,
    );
    update_task.await.unwrap().unwrap();

    assert!(
        fixture
            .caller_dispatch
            .disconnect(&fixture.node_id, 4001, "admin revoked node")
            .await
            .unwrap()
    );
    let close = tokio::time::timeout(Duration::from_secs(2), fixture.outbound.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        close,
        NodeOutboundMessage::Close { code: 4001, reason } if reason == "admin revoked node"
    ));
    let stored = fixture
        .db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": &fixture.node_id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, NodeStatus::Offline);
    assert!(stored.connection_owner.is_none());
}
