//! Actual CLI runtime over a loopback WebSocket, with NyxID's gateway/proxy
//! pipeline and a local TLS upstream. No provider credentials or external calls.
use super::{
    assistant_authority_tests::{Fixture, fixture, orchestrator_fixture},
    machine_integration_tests::node,
    node_service,
    node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage},
};
use crate::{
    handlers::{machine_gateway::Session, machine_tools::call},
    test_utils::*,
};
use axum::{
    Router,
    body::{Body, Bytes},
    http::{Request, Response},
};
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use nyxid_node_proxy_test::machine::Runtime;
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use uuid::Uuid;

struct Peer {
    runtime: Arc<Runtime>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _root: tempfile::TempDir,
}
impl Drop for Peer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Peer {
    async fn start(f: &Fixture) -> (Self, crate::models::node::Node) {
        let root = tempfile::tempdir().unwrap();
        let mut node = node(f, &f.owner).await;
        let config = nyxid_machine::config::Config {
            shell: true,
            files: true,
            roots: vec![root.path().to_owned()],
            allow_root: true,
            ..Default::default()
        };
        let runtime = Runtime::new(&config, &node.id, &root.path().join("private-node")).unwrap();
        let profile = runtime.profile().await;
        node.machine = Some(profile.clone());
        f.state
            .db
            .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&node.id},
                doc! {"$set":{"machine":mongodb::bson::to_bson(&profile).unwrap()}},
            )
            .await
            .unwrap();
        let (server_tx, mut server_rx) = mpsc::channel(256);
        register_test_node_connection(&f.state, &node.id, server_tx.clone()).await;
        let caps: NodeCapabilitiesMsg = serde_json::from_value(json!({"machine":profile})).unwrap();
        f.state.node_ws_manager.record_capabilities(&node.id, &caps);
        let secret =
            node_service::get_node_signing_secret(&f.state.db, &f.state.encryption_keys, &node.id)
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
        let state = f.state.clone();
        let id = node.id.clone();
        let session = Session::new(server_tx);
        let target_client = super::proxy_service::TARGET_HTTP_CLIENT_BUILDER
            .try_with(Clone::clone)
            .ok();
        tasks.push(tokio::spawn(async move {
            while let Some(Ok(msg)) = server_read.next().await {
                match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        match value["type"].as_str() {
                            Some("machine_result") => {
                                state.node_ws_manager.deliver_machine_result(
                                    &id,
                                    serde_json::from_value(value).unwrap(),
                                );
                            }
                            Some("machine_service_call") => {
                                let start = session.start(
                                    state.clone(),
                                    &id,
                                    serde_json::from_value(value).unwrap(),
                                );
                                if let Some(builder) = &target_client {
                                    super::proxy_service::TARGET_HTTP_CLIENT_BUILDER
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
        tasks.push(tokio::spawn(async move {
            let mut requests = tokio::task::JoinSet::new();
            while let Some(Ok(msg)) = client_read.next().await {
                match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        match value["type"].as_str() {
                            Some("machine_request") => {
                                let request: nyxid_machine::Request =
                                    serde_json::from_value(value).unwrap();
                                let active = active.clone();
                                let node_tx = node_tx.clone();
                                let secret = secret.clone();
                                requests.spawn(async move {
                                    let request_id = request.request_id.clone();
                                    let result = active.handle(request, &secret).await;
                                    node_tx.send(nyxid_node_proxy_test::ws_client::NodeWsMessage::Text(
                                        json!({"type":"machine_result","request_id":request_id,"result":result}).to_string(),
                                    )).await.unwrap();
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
        (
            Self {
                runtime,
                tasks,
                _root: root,
            },
            node,
        )
    }
}

async fn connect_service(f: &Fixture, slug: &str, url: &str, secret: &str) -> String {
    use super::user_api_key_service::{CreateApiKeyParams, create_api_key};
    let key = create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        CreateApiKeyParams {
            label: "Machine test",
            credential_type: "bearer",
            credential: secret,
            access_token: None,
            refresh_token: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        },
    )
    .await
    .unwrap();
    let endpoint = Uuid::new_v4().to_string();
    let id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(test_user_endpoint(
            &endpoint, &f.owner, slug, url, None, None,
        ))
        .await
        .unwrap();
    let mut service = test_user_service(&id, &f.owner, slug, &endpoint, None, None);
    service.api_key_id = Some(key.id);
    if slug == "api-github" {
        let mut catalog = test_auto_connected_catalog_service();
        catalog.slug = slug.into();
        catalog.base_url = url.into();
        catalog.auth_method = "bearer".into();
        catalog.requires_user_credential = true;
        catalog.git_http = Some(crate::models::downstream_service::GitHttp {
            origin: "https://github.com".into(),
            username: "x-access-token".into(),
        });
        service.catalog_service_id = Some(catalog.id.clone());
        f.state
            .db
            .collection(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(catalog)
            .await
            .unwrap();
    }
    service.auth_method = "bearer".into();
    f.state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn machine_gateway_uses_live_specialist_scope_and_server_credentials() {
    use super::assistant_team_service::{self as team, GrantChange, MachineGrantMode};
    let f = fixture("machine_gateway_real_runtime").await;
    let provider_secret = format!("provider-{}", Uuid::new_v4());
    let expected = provider_secret.clone();
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", upstream.local_addr().unwrap());
    let server = tokio::spawn(
        axum::serve(
            upstream,
            Router::new().fallback(move |request: Request<Body>| {
                let expected = expected.clone();
                async move {
                    assert_eq!(
                        request.headers().get("authorization").unwrap(),
                        &format!("Bearer {expected}")
                    );
                    "upstream accepted server credential"
                }
            }),
        )
        .into_future(),
    );
    let service = connect_service(&f, "test-machine-api", &url, &provider_secret).await;
    let (peer, node) = Peer::start(&f).await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        GrantChange::Machine {
            base: Box::new(GrantChange::Add(Default::default())),
            machines: Some(vec![node.id.clone()]),
            logins: None,
            mode: MachineGrantMode::Add,
        },
    )
    .await
    .unwrap();
    let chat = super::assistant_acknowledgement_service::for_key(
        &f.state.db,
        &f.owner,
        Some(&f.chat.api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let command = "curl -sS -H \"Authorization: Bearer $NYXID_GATEWAY_TOKEN\" \"$NYXID_GATEWAY_URL/s/test-machine-api/hello?_nyxid_via=untrusted-process-override\"";
    let denied = call(
        &f.state,
        &chat,
        "nyx__machine_exec",
        json!({"machine":node.id,"command":command}),
    )
    .await
    .unwrap();
    assert!(
        !denied["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("upstream accepted")
    );
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        GrantChange::Add(crate::models::assistant_agent::AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let undeclared = call(
        &f.state,
        &chat,
        "nyx__machine_exec",
        json!({
            "machine":node.id,"command":command,
        }),
    )
    .await
    .unwrap();
    assert!(
        undeclared["stdout"]
            .as_str()
            .unwrap()
            .contains("Declare test-machine-api in services on nyx__machine_exec")
    );
    let allowed = call(
        &f.state,
        &chat,
        "nyx__machine_exec",
        json!({"machine":node.id,"command":command,"services":["test-machine-api"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        allowed["stdout"], "upstream accepted server credential",
        "declared service call should reach the upstream"
    );
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
    super::agent_operation_scope_service::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &service,
        &crate::models::agent_operation_scope::OperationSelection {
            expected_revision: 0,
            all_operations: false,
            endpoint_ids: vec![],
            rules: vec![crate::models::downstream_service::ProxyOperationRule {
                method: "GET".into(),
                path_template: "/allowed".into(),
                ..Default::default()
            }],
        },
        true,
    )
    .await
    .unwrap();
    let narrowed = call(
        &f.state,
        &chat,
        "nyx__machine_exec",
        json!({"machine":node.id,"command":command,"services":["test-machine-api"]}),
    )
    .await
    .unwrap();
    assert!(
        narrowed["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("Allowed operations")
    );
    assert!(
        !narrowed["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("upstream accepted")
    );
    let env = call(
        &f.state,
        &chat,
        "nyx__machine_exec",
        json!({"machine":node.id,"command":"env"}),
    )
    .await
    .unwrap();
    assert!(!format!("{denied}{allowed}{env}").contains(&provider_secret));
    assert!(
        env["stdout"]
            .as_str()
            .unwrap()
            .contains("NYXID_GATEWAY_URL=http://127.0.0.1:")
    );
    peer.runtime.shutdown().await;
    server.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "repeatable real node benchmark; run alone with --ignored --nocapture"]
async fn machine_loopback_exec_performance() {
    let f = orchestrator_fixture("machine_loopback_exec_perf").await;
    let (peer, node) = Peer::start(&f).await;
    let mut samples = Vec::new();
    for iteration in 0..110 {
        let start = Instant::now();
        let result = call(
            &f.state,
            &f.chat,
            "nyx__machine_exec",
            json!({"machine":node.id,"command":"true"}),
        )
        .await
        .unwrap();
        assert_eq!(
            result["exit_code"], 0,
            "result command should exit successfully"
        );
        let overhead = (start.elapsed().as_secs_f64() * 1000.0
            - result["duration_ms"].as_f64().unwrap())
        .max(0.0);
        if iteration >= 10 {
            samples.push(overhead);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "machine_exec loopback WS, actual CLI, 100 samples, command runtime excluded: p50={:.3}ms p95={:.3}ms",
        samples[49], samples[94]
    );
    assert!(
        samples[94] <= 50.0,
        "machine loopback overhead exceeds budget"
    );
    peer.runtime.shutdown().await;
    f.state.db.drop().await.unwrap();
}

struct TlsProxyListener {
    listener: tokio::net::TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
}
impl axum::serve::Listener for TlsProxyListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = std::net::SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok((mut stream, address)) = self.listener.accept().await else {
                continue;
            };
            // CONNECT preserves provider host validation, SNI and verified TLS while
            // keeping every byte in this test process.
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") && header.len() < 8192 {
                let Ok(byte) = stream.read_u8().await else {
                    break;
                };
                header.push(byte);
            }
            if !header.starts_with(b"CONNECT ") {
                continue;
            }
            if stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .is_err()
            {
                continue;
            }
            if let Ok(tls) = self.acceptor.accept(stream).await {
                return (tls, address);
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}
struct GitUpstream {
    client: reqwest::Client,
    client_builder: Arc<dyn Fn() -> reqwest::ClientBuilder + Send + Sync>,
    proxy: String,
    ca: std::path::PathBuf,
    root: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for GitUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn git(args: &[&str], cwd: &std::path::Path) {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
impl GitUpstream {
    async fn start(secret: &str, medium: bool, compressed: bool) -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let certificate = rcgen::generate_simple_self_signed(vec![
            "github.com".into(),
            "api.github.com".into(),
            "direct.example".into(),
        ])
        .unwrap();
        let trust = reqwest::Certificate::from_pem(certificate.cert.pem().as_bytes()).unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.cert.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    certificate.signing_key.serialize_der().into(),
                ),
            )
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let ca = root.path().join("ca.pem");
        std::fs::write(&ca, certificate.cert.pem()).unwrap();
        std::fs::create_dir(root.path().join("owner")).unwrap();
        git(
            &["init", "--bare", "--initial-branch=main", "owner/repo.git"],
            root.path(),
        )
        .await;
        git(
            &[
                "--git-dir=owner/repo.git",
                "config",
                "http.receivepack",
                "true",
            ],
            root.path(),
        )
        .await;
        git(&["init", "--initial-branch=main", "seed"], root.path()).await;
        let seed = root.path().join("seed");
        git(&["config", "user.name", "NyxID test"], &seed).await;
        git(&["config", "user.email", "machine@example.test"], &seed).await;
        for n in 0..if medium { 96 } else { 2 } {
            let bytes: Vec<u8> = (0..128 * 1024).map(|_| rand::random::<u8>()).collect();
            std::fs::write(seed.join(format!("file-{n}.bin")), bytes).unwrap();
        }
        git(&["add", "."], &seed).await;
        git(&["commit", "-qm", "medium repository"], &seed).await;
        git(&["push", "../owner/repo.git", "main"], &seed).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let proxy_url = proxy.clone();
        let client_builder: Arc<dyn Fn() -> reqwest::ClientBuilder + Send + Sync> =
            Arc::new(move || {
                reqwest::Client::builder()
                    .proxy(reqwest::Proxy::all(&proxy_url).unwrap())
                    .add_root_certificate(trust.clone())
            });
        let client = client_builder().build().unwrap();
        let repo = root.path().to_owned();
        let expected = secret.to_owned();
        let router = Router::new().fallback(move |request: Request<Body>| {
            let repo = repo.clone();
            let expected = expected.clone();
            async move {
                let host = request.headers().get("host").unwrap().to_str().unwrap();
                if host != "direct.example" {
                    use base64::Engine;
                    let auth = if host == "github.com" {
                        format!(
                            "Basic {}",
                            base64::engine::general_purpose::STANDARD
                                .encode(format!("x-access-token:{expected}"))
                        )
                    } else {
                        format!("Bearer {expected}")
                    };
                    assert_eq!(request.headers().get("authorization").unwrap(), &auth);
                }
                if request.uri().path() == "/download" {
                    let chunk = Bytes::from(vec![42; 65536]);
                    return Response::builder()
                        .header("content-type", "application/octet-stream")
                        .header("content-length", 100 * 1024 * 1024)
                        .body(Body::from_stream(futures::stream::iter(
                            (0..1600).map(move |_| Ok::<_, std::io::Error>(chunk.clone())),
                        )))
                        .unwrap();
                }
                let accepts_gzip = request
                    .headers()
                    .get("accept-encoding")
                    .is_some_and(|v| v.to_str().unwrap_or_default().contains("gzip"));
                if request.uri().path() == "/sdk" {
                    assert!(accepts_gzip, "SDK advertises gzip");
                    let bytes = gzip(b"{\"sdk\":\"compressed response decoded\"}");
                    return Response::builder()
                        .header("content-type", "application/json")
                        .header("content-encoding", "gzip")
                        .header("content-length", bytes.len())
                        .body(Body::from(bytes))
                        .unwrap();
                }
                let path = request.uri().path().to_owned();
                let query = request.uri().query().unwrap_or_default().to_owned();
                let method = request.method().as_str().to_owned();
                let content_type = request
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                let body = axum::body::to_bytes(request.into_body(), 32 * 1024 * 1024)
                    .await
                    .unwrap();
                let mut child = tokio::process::Command::new("git")
                    .arg("http-backend")
                    .env("GIT_PROJECT_ROOT", &repo)
                    .env("GIT_HTTP_EXPORT_ALL", "1")
                    .env("PATH_INFO", path)
                    .env("QUERY_STRING", query)
                    .env("REQUEST_METHOD", method)
                    .env("CONTENT_TYPE", content_type)
                    .env("CONTENT_LENGTH", body.len().to_string())
                    .env("REMOTE_USER", "test")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap();
                let mut input = child.stdin.take().unwrap();
                tokio::spawn(async move {
                    input.write_all(&body).await.unwrap();
                });
                use tokio::io::AsyncBufReadExt;
                let mut reader = tokio::io::BufReader::new(child.stdout.take().unwrap());
                let mut response = Response::builder();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    let (name, value) = line.trim_end().split_once(':').unwrap();
                    if name.eq_ignore_ascii_case("status") {
                        response = response.status(
                            value
                                .trim()
                                .split(' ')
                                .next()
                                .unwrap()
                                .parse::<u16>()
                                .unwrap(),
                        );
                    } else {
                        response = response.header(name, value.trim());
                    }
                }
                let stream = futures::stream::unfold(
                    (reader, child),
                    |(mut reader, mut child)| async move {
                        let mut bytes = vec![0; 65536];
                        match reader.read(&mut bytes).await {
                            Ok(0) => {
                                assert!(child.wait().await.unwrap().success());
                                None
                            }
                            Ok(n) => {
                                bytes.truncate(n);
                                Some((Ok::<_, std::io::Error>(bytes), (reader, child)))
                            }
                            Err(error) => Some((Err(error), (reader, child))),
                        }
                    },
                );
                if compressed {
                    assert!(accepts_gzip, "git advertises gzip");
                    let bytes = axum::body::to_bytes(Body::from_stream(stream), 4 * 1024 * 1024)
                        .await
                        .unwrap();
                    let bytes = gzip(&bytes);
                    response
                        .header("content-encoding", "gzip")
                        .header("content-length", bytes.len())
                        .body(Body::from(bytes))
                        .unwrap()
                } else {
                    response.body(Body::from_stream(stream)).unwrap()
                }
            }
        });
        let task = tokio::spawn(async move {
            axum::serve(
                TlsProxyListener {
                    listener,
                    acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(config)),
                },
                router,
            )
            .await
            .unwrap();
        });
        Self {
            client,
            client_builder,
            proxy,
            ca,
            root,
            task,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "100 MiB streaming and real git clone/push benchmark; run alone with --ignored --nocapture"]
async fn machine_gateway_streaming_and_git_performance() {
    let mut f = orchestrator_fixture("machine_gateway_stream_git").await;
    let secret = format!("synthetic-provider-{}", Uuid::new_v4());
    let upstream = GitUpstream::start(&secret, true, false).await;
    f.state.http_client = upstream.client.clone();
    connect_service(&f, "api-github", "https://api.github.com", &secret).await;
    let (peer, node) = super::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .scope(upstream.client_builder.clone(), Peer::start(&f))
        .await;
    let direct = Instant::now();
    let mut count = 0usize;
    let mut stream = upstream
        .client
        .get("https://direct.example/download")
        .send()
        .await
        .unwrap()
        .bytes_stream();
    while let Some(chunk) = stream.next().await {
        count += chunk.unwrap().len();
    }
    let direct_download = direct.elapsed();
    assert_eq!(count, 100 * 1024 * 1024);
    let download=call(&f.state,&f.chat,"nyx__machine_exec",json!({"machine":node.id,"services":["api-github"],"command":"curl --fail -sS -H \"Authorization: Bearer $NYXID_GATEWAY_TOKEN\" \"$NYXID_GATEWAY_URL/s/api-github/download\" -o large.bin && wc -c < large.bin","timeout_secs":120})).await.unwrap();
    assert_eq!(
        download["exit_code"], 0,
        "download command should exit successfully"
    );
    assert_eq!(download["stdout"].as_str().unwrap().trim(), "104857600");
    let direct = Instant::now();
    let output = tokio::process::Command::new("git")
        .args([
            "-c",
            &format!("http.proxy={}", upstream.proxy),
            "-c",
            &format!("http.sslCAInfo={}", upstream.ca.display()),
            "clone",
            "--quiet",
            "https://direct.example/owner/repo.git",
            "baseline",
        ])
        .current_dir(upstream.root.path())
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "direct git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let direct_clone = direct.elapsed();
    let cloned=call(&f.state,&f.chat,"nyx__machine_exec",json!({"machine":node.id,"services":["api-github"],"command":"git clone --quiet https://github.com/owner/repo.git clone && git -C clone config --get remote.origin.url","timeout_secs":120})).await.unwrap();
    assert_eq!(
        cloned["exit_code"], 0,
        "cloned command should exit successfully"
    );
    assert_eq!(cloned["stdout"], "https://github.com/owner/repo.git\n");
    let pushed=call(&f.state,&f.chat,"nyx__machine_exec",json!({"machine":node.id,"services":["api-github"],"command":"cd clone && git config user.name 'Machine test' && git config user.email machine@example.test && printf verified > pushed.txt && git add pushed.txt && git commit -qm push && git push --quiet origin main && git fetch --quiet && git pull --quiet","timeout_secs":120})).await.unwrap();
    assert_eq!(
        pushed["exit_code"], 0,
        "pushed command should exit successfully"
    );
    let verify = tokio::process::Command::new("git")
        .args(["--git-dir=owner/repo.git", "show", "main:pushed.txt"])
        .current_dir(upstream.root.path())
        .output()
        .await
        .unwrap();
    assert_eq!(verify.stdout, b"verified");
    let config = std::fs::read_to_string(peer._root.path().join("clone/.git/config")).unwrap();
    assert!(!config.contains("127.0.0.1"));
    assert!(!config.contains("extraHeader"));
    assert!(!config.contains(&secret));
    for result in [&download, &cloned, &pushed] {
        assert!(!result.to_string().contains(&secret));
    }
    println!(
        "100 MiB: direct={:.3}s ({:.2}MiB/s), gateway={:.3}s ({:.2}MiB/s); 12 MiB git clone: direct={:.3}s gateway={:.3}s; smart-HTTP clone/fetch/pull/push verified",
        direct_download.as_secs_f64(),
        100.0 / direct_download.as_secs_f64(),
        download["duration_ms"].as_f64().unwrap() / 1000.0,
        100000.0 / download["duration_ms"].as_f64().unwrap(),
        direct_clone.as_secs_f64(),
        cloned["duration_ms"].as_f64().unwrap() / 1000.0
    );
    peer.runtime.shutdown().await;
    f.state.db.drop().await.unwrap();
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn machine_gateway_git_clone_fetch_pull_push_and_sdk_preserve_gzip() {
    let mut f = orchestrator_fixture("machine_git_gzip_correctness").await;
    let secret = format!("synthetic-provider-{}", Uuid::new_v4());
    let upstream = GitUpstream::start(&secret, false, true).await;
    f.state.http_client = upstream.client.clone();
    connect_service(&f, "api-github", "https://api.github.com", &secret).await;
    let (peer, node) = super::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .scope(upstream.client_builder.clone(), Peer::start(&f))
        .await;
    let sdk = call(&f.state, &f.chat, "nyx__machine_exec", json!({
        "machine":node.id,"services":["api-github"],
        "command":"curl --compressed --fail -sS -H \"Authorization: Bearer $NYXID_GATEWAY_TOKEN\" \"$NYXID_GATEWAY_URL/s/api-github/sdk\"",
    })).await.unwrap();
    assert_eq!(sdk["exit_code"], 0, "sdk command should exit successfully");
    assert_eq!(
        serde_json::from_str::<Value>(sdk["stdout"].as_str().unwrap()).unwrap()["sdk"],
        "compressed response decoded"
    );
    let cloned=call(&f.state,&f.chat,"nyx__machine_exec",json!({"machine":node.id,"services":["api-github"],"command":"git clone --quiet https://github.com/owner/repo.git clone && git -C clone config --get remote.origin.url","timeout_secs":120})).await.unwrap();
    assert_eq!(
        cloned["exit_code"], 0,
        "cloned command should exit successfully"
    );
    assert_eq!(cloned["stdout"], "https://github.com/owner/repo.git\n");
    let pushed=call(&f.state,&f.chat,"nyx__machine_exec",json!({"machine":node.id,"services":["api-github"],"command":"cd clone && git config user.name 'Machine test' && git config user.email machine@example.test && printf verified > pushed.txt && git add pushed.txt && git commit -qm push && git push --quiet origin main && git fetch --quiet && git pull --quiet","timeout_secs":120})).await.unwrap();
    assert_eq!(
        pushed["exit_code"], 0,
        "pushed command should exit successfully"
    );
    let verify = tokio::process::Command::new("git")
        .args(["--git-dir=owner/repo.git", "show", "main:pushed.txt"])
        .current_dir(upstream.root.path())
        .output()
        .await
        .unwrap();
    assert_eq!(verify.stdout, b"verified");
    let config = std::fs::read_to_string(peer._root.path().join("clone/.git/config")).unwrap();
    assert!(!config.contains("127.0.0.1"));
    assert!(!config.contains("extraHeader"));
    assert!(!config.contains(&secret));
    for result in [&cloned, &pushed] {
        assert!(!result.to_string().contains(&secret));
    }

    peer.runtime.shutdown().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_declared_services_are_bound_to_job_card_and_audit() {
    use super::assistant_acknowledgement_service as acks;
    use crate::models::{
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        audit_log::AuditLog,
    };
    let f = orchestrator_fixture("machine_declarations_card_audit").await;
    let service = connect_service(
        &f,
        "declared-service",
        "https://example.test",
        "synthetic-key",
    )
    .await;
    let (peer, node) = Peer::start(&f).await;
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$set":{"machine_confirm":"all"}},
        )
        .await
        .unwrap();
    let mut args = json!({"machine":node.id,"command":"true","services":[service]});
    let card = call(&f.state, &f.chat, "nyx__machine_exec", args.clone())
        .await
        .unwrap();
    let id = card["acknowledgement_id"].as_str().unwrap();
    let row = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert!(row.summary.contains("declared services: declared-service"));
    acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
        .await
        .unwrap();
    args["acknowledgement_id"] = json!(id);
    let result = call(&f.state, &f.chat, "nyx__machine_exec", args)
        .await
        .unwrap();
    assert_eq!(
        result["exit_code"], 0,
        "result command should exit successfully"
    );
    let job = f
        .state
        .db
        .collection::<crate::models::machine_job::MachineJob>(
            crate::models::machine_job::COLLECTION_NAME,
        )
        .find_one(doc! {"conversation_id":&f.row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        job.services,
        vec![crate::models::machine_job::DeclaredService {
            id: service,
            slug: "declared-service".into()
        }]
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(audit) = f
                .state
                .db
                .collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME)
                .find_one(doc! {"event_type":"machine_operation","event_data.node_id":&node.id})
                .await
                .unwrap()
            {
                assert_eq!(
                    audit.event_data.unwrap()["services"],
                    json!(["declared-service"])
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    peer.runtime.shutdown().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_gateway_and_direct_api_key_auth_have_identical_effective_authority() {
    use crate::mw::auth::AuthUser;
    use axum::extract::FromRequestParts;
    let f = orchestrator_fixture("machine_auth_parity").await;
    let node = node(&f, &f.owner).await;
    let ordinary = connect_service(&f, "ordinary", "https://example.test", "synthetic").await;
    let auto = connect_service(&f, "auto", "https://example.test", "synthetic").await;
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&auto},
            doc! {"$set":{"source":"auto_provision"}},
        )
        .await
        .unwrap();
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "platform-test".into();
    catalog.auth_method = "bearer".into();
    catalog.credential_encrypted = vec![1];
    catalog.platform_key = Some(crate::models::downstream_service::PlatformKeyConfig {
        enabled: true,
        audience: crate::models::downstream_service::PlatformKeyAudience::Public,
        ..Default::default()
    });
    let platform = catalog.id.clone();
    f.state
        .db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(catalog)
        .await
        .unwrap();
    f.state.db.collection::<mongodb::bson::Document>(crate::models::api_key::COLLECTION_NAME).update_one(doc!{"_id":&f.chat.api_key_id},doc!{"$set":{
        "allow_all_services":false,"allowed_service_ids":[&ordinary],"allowed_platform_service_ids":[&platform],"allow_auto_connected_services":true
    }}).await.unwrap();
    let credential = super::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (mut parts, _) = Request::builder()
        .uri("/api/v1/proxy/s/ordinary/status")
        .header(
            "authorization",
            format!("Bearer {}", credential.raw_key.as_str()),
        )
        .body(())
        .unwrap()
        .into_parts();
    let direct = AuthUser::from_request_parts(&mut parts, &f.state)
        .await
        .unwrap();
    let job = super::machine_service::issue_job(&f.state.db, &f.chat, &node, 120, Vec::new())
        .await
        .unwrap();
    let gateway = crate::handlers::machine_gateway::job_auth(&f.state, &job)
        .await
        .unwrap();
    assert_eq!(gateway.allowed_service_ids, direct.allowed_service_ids);
    assert!(gateway.allowed_service_ids.contains(&ordinary));
    assert!(gateway.allowed_service_ids.contains(&auto));
    assert!(gateway.allowed_service_ids.contains(&platform));
    assert_eq!(gateway.api_key_purpose, direct.api_key_purpose);
    assert_eq!(gateway.allow_all_services, direct.allow_all_services);
    assert_eq!(gateway.allowed_node_ids, direct.allowed_node_ids);
    assert_eq!(gateway.scope, direct.scope);
    let listed =
        super::machine_gateway_service::services(&f.state.db, &f.owner, &f.chat.api_key_id)
            .await
            .unwrap();
    for id in [&ordinary, &auto, &platform] {
        assert!(listed.iter().any(|row| &row.id == id));
    }
    assert!(
        listed
            .iter()
            .find(|row| row.id == platform)
            .unwrap()
            .git
            .is_none()
    );
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allowed_platform_service_ids":[],"allow_auto_connected_services":false}},
        )
        .await
        .unwrap();
    let listed =
        super::machine_gateway_service::services(&f.state.db, &f.owner, &f.chat.api_key_id)
            .await
            .unwrap();
    assert!(listed.iter().all(|row| row.id == ordinary));
    f.state.db.drop().await.unwrap();
}
