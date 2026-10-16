//! Test-only loopback bridge into the actual CLI Runtime. No fake executor or identity.
use crate::{
    handlers::machine_gateway::Session,
    services::{
        node_service,
        node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage},
    },
    test_utils::register_test_node_connection,
};
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use nyxid_node_proxy_test::machine::Runtime;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{RwLock, mpsc};
pub(super) struct Peer {
    pub(super) runtime: Arc<Runtime>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    pub(super) root: tempfile::TempDir,
    pub(super) dispatches: Arc<AtomicUsize>,
    request_context: Arc<RwLock<crate::AppState>>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Peer {
    /// Replace only the backend request context. The real Node Runtime and its
    /// signed socket remain alive, as they do when another replica serves HTTP.
    pub(super) async fn replace_request_context(&self, state: crate::AppState) {
        *self.request_context.write().await = state;
    }

    pub(super) async fn start(
        state: &crate::AppState,
        owner: &str,
    ) -> (Self, crate::models::node::Node) {
        crate::services::machine_access_service::ensure_indexes(&state.db)
            .await
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let (_, token, _) = node_service::create_registration_token(
            &state.db,
            owner,
            "public-runtime-fixture",
            100,
            300,
        )
        .await
        .unwrap();
        let (mut node, _, _) =
            node_service::register_node(&state.db, &state.encryption_keys, &token, None)
                .await
                .unwrap();
        node.status = crate::models::node::NodeStatus::Online;
        state
            .db
            .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
            .update_one(doc! {"_id":&node.id}, doc! {"$set":{"status":"online"}})
            .await
            .unwrap();
        let config = nyxid_machine::config::Config {
            shell: true,
            files: true,
            roots: vec![root.path().to_owned()],
            allow_root: true,
            ..Default::default()
        };
        let private = root.path().join("private-node");
        std::fs::create_dir(&private).unwrap();
        let runtime = Runtime::new(&config, &node.id, &private).unwrap();
        let profile = runtime.profile().await;
        node.machine = Some(profile.clone());
        state
            .db
            .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&node.id},
                doc! {"$set":{"machine":mongodb::bson::to_bson(&profile).unwrap()}},
            )
            .await
            .unwrap();
        let (server_tx, mut server_rx) = mpsc::channel(256);
        register_test_node_connection(state, &node.id, server_tx.clone()).await;
        let caps: NodeCapabilitiesMsg = serde_json::from_value(json!({"machine":profile})).unwrap();
        state.node_ws_manager.record_capabilities(&node.id, &caps);
        let secret =
            node_service::get_node_signing_secret(&state.db, &state.encryption_keys, &node.id)
                .await
                .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move {
            tokio_tungstenite::accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap()
        });
        let (client, _) =
            tokio_tungstenite::connect_async_with_config(format!("ws://{address}"), None, true)
                .await
                .unwrap();
        let server = accept.await.unwrap();
        let (mut server_write, mut server_read) = server.split();
        let (mut client_write, mut client_read) = client.split();
        let mut tasks = Vec::new();
        tasks.push(tokio::spawn(async move {
            while let Some(msg) = server_rx.recv().await {
                let msg = match msg {
                    NodeOutboundMessage::Close { .. } => break,
                    NodeOutboundMessage::Text(s) => {
                        tokio_tungstenite::tungstenite::Message::Text(s.into())
                    }
                    NodeOutboundMessage::Binary(b) => {
                        tokio_tungstenite::tungstenite::Message::Binary(b.into())
                    }
                };
                if server_write.send(msg).await.is_err() {
                    break;
                }
            }
        }));
        let (node_tx, mut node_rx) = mpsc::channel(256);
        runtime.connect(node_tx.clone(), &secret).await.unwrap();
        tasks.push(tokio::spawn(async move {
            while let Some(msg) = node_rx.recv().await {
                let msg = match msg {
                    nyxid_node_proxy_test::ws_client::NodeWsMessage::Text(s) => {
                        tokio_tungstenite::tungstenite::Message::Text(s.into())
                    }
                    nyxid_node_proxy_test::ws_client::NodeWsMessage::Binary(b) => {
                        tokio_tungstenite::tungstenite::Message::Binary(b.into())
                    }
                };
                if client_write.send(msg).await.is_err() {
                    break;
                }
            }
        }));
        let gateway_state = state.clone();
        let request_context = Arc::new(RwLock::new(state.clone()));
        let gateway_context = request_context.clone();
        let id = node.id.clone();
        let session = Session::new(server_tx);
        let target_client = crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
            .try_with(Clone::clone)
            .ok();
        tasks.push(tokio::spawn(async move {
            while let Some(Ok(msg)) = server_read.next().await {
                match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        match value["type"].as_str() {
                            Some("machine_result") => {
                                gateway_state.node_ws_manager.deliver_machine_result(
                                    &id,
                                    serde_json::from_value(value).unwrap(),
                                );
                            }
                            Some("machine_service_call") => {
                                let active_state = gateway_context.read().await.clone();
                                let start = session.start(
                                    active_state,
                                    &id,
                                    serde_json::from_value(value).unwrap(),
                                );
                                if let Some(builder) = &target_client {
                                    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
                                        .scope(builder.clone(), start)
                                        .await;
                                } else {
                                    start.await;
                                }
                            }
                            _ => {}
                        }
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        if let Ok(frame) = nyxid_machine::binary::Frame::decode(&bytes) {
                            session.receive(frame).await;
                        }
                    }
                    _ => {}
                }
            }
            session.close().await;
        }));
        let active = runtime.clone();
        let dispatches = Arc::new(AtomicUsize::new(0));
        let dispatch_count = dispatches.clone();
        tasks.push(tokio::spawn(async move {
            let mut requests = tokio::task::JoinSet::new();
            while let Some(Ok(msg)) = client_read.next().await {
                match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        match value["type"].as_str() {
                            Some(
                                "machine_request" | "machine_request_v2" | "machine_request_v3",
                            ) => {
                                dispatch_count.fetch_add(1, Ordering::SeqCst);
                                let request: nyxid_machine::Request =
                                    serde_json::from_value(value).unwrap();
                                let active = active.clone();
                                let node_tx = node_tx.clone();
                                let secret = secret.clone();
                                requests.spawn(async move {
                                    let request_id = request.request_id.clone();
                                    let operation = request.operation;
                                    let revision = active
                                        .control_revision(operation, &request.parameters)
                                        .await;
                                    let mut parameters = request.parameters.clone();
                                    parameters["_signed_authority"] =
                                        serde_json::to_value(&request.authority).unwrap();
                                    parameters["_signed_external_authority"] =
                                        serde_json::to_value(&request.external_authority).unwrap();
                                    let result = active.handle(request, &secret).await;
                                    active
                                        .send_result(
                                            &node_tx,
                                            &request_id,
                                            operation,
                                            revision,
                                            &parameters,
                                            result,
                                        )
                                        .await;
                                });
                            }
                            Some("machine_service_response") => {
                                let id = value["request_id"].as_str().unwrap().to_owned();
                                active.gateway_response(&id, value).await;
                            }
                            Some("machine_job_finished_ack") => {
                                active
                                    .job_finished_ack(value["request_id"].as_str().unwrap())
                                    .await
                            }
                            _ => {}
                        }
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        active.binary(&bytes).await
                    }
                    _ => {}
                }
                while requests.try_join_next().is_some() {}
            }
        }));
        // The real server renews leases independently of the HTTP call. This
        // loopback adapter uses that same live resolver while its peer exists.
        let maintenance_context = request_context.clone();
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let state = maintenance_context.read().await.clone();
                let _ = Box::pin(crate::services::machine_external_service::maintenance(
                    &state,
                ))
                .await;
            }
        }));
        (
            Self {
                runtime,
                tasks,
                root,
                dispatches,
                request_context,
            },
            node,
        )
    }
}
