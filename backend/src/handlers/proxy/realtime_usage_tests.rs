use super::*;
use crate::billing_integration_tests::usage_checkpoints::{Fixture, realtime_ws as fixtures};
use crate::models::service_billing::BillingMetric;
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Downstream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
#[derive(Clone)]
struct Harness {
    state: crate::AppState,
    metered: crate::services::billing::MeteredProxyContext,
    downstream: Arc<Mutex<Option<Downstream>>>,
    node_frames: Arc<Mutex<Option<mpsc::Receiver<crate::services::node_ws_manager::WsProxyFrame>>>>,
    enabled: bool,
    duration: Option<std::time::Duration>,
    abort: tokio_util::sync::CancellationToken,
    completion: Arc<Mutex<Option<BridgeCompletion>>>,
}
// A single upgraded connection owns both senders. A panic drops the sender
// and is visible immediately to the test awaiting that phase.
struct BridgeCompletion {
    closed: oneshot::Sender<()>,
    finished: oneshot::Sender<()>,
}
async fn serve_bridge(
    State(h): State<Harness>,
    ws: WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    let mut session = h
        .state
        .billing
        .mark_realtime_forwarded(&h.metered, h.enabled, true)
        .await
        .unwrap();
    if let Some(duration) = h.duration {
        session.as_mut().unwrap().deadline =
            crate::services::billing::realtime_usage::BridgeDeadline::new(duration);
    }
    let completion = h
        .completion
        .lock()
        .await
        .take()
        .expect("one upgrade per fixture");
    ws.on_upgrade(move |client| async move {
        let work = async {
            let stats = if let Some(downstream) = h.downstream.lock().await.take() {
                bridge_websockets(
                    client,
                    downstream,
                    "realtime-test".into(),
                    true,
                    Vec::new(),
                    String::new().into(),
                    h.state.db.clone(),
                    "actor".into(),
                    None,
                    None,
                    None,
                    None,
                    tokio_util::sync::CancellationToken::new(),
                    None,
                    &mut session,
                )
                .await
            } else {
                let receiver = h.node_frames.lock().await.take().unwrap();
                bridge_websockets_via_node(
                    client,
                    receiver,
                    &h.state.node_dispatch,
                    "node-test",
                    "session-test",
                    "realtime-test".into(),
                    true,
                    h.state.db.clone(),
                    "actor".into(),
                    None,
                    None,
                    "owner".into(),
                    "actor".into(),
                    None,
                    None,
                    tokio_util::sync::CancellationToken::new(),
                    &mut session,
                )
                .await
            };
            let _ = completion.closed.send(());
            // Same intent and money functions as production; await settlement
            // here so command accounting needs no polling queries.
            let usage = websocket_platform_usage(&stats);
            if let Some(session) = session {
                let rows = session.persist_close(usage).await.unwrap();
                fixtures::settle_rows(&h.state.db, rows).await;
            } else {
                h.state
                    .billing
                    .settle(&h.metered, usage, None, None)
                    .await
                    .unwrap();
            }
        };
        tokio::select! {_=work=>{},()=h.abort.cancelled()=>{}}
        let _ = completion.finished.send(());
    })
}
struct Running {
    client: Downstream,
    closed: oneshot::Receiver<()>,
    finished: oneshot::Receiver<()>,
    direct: Option<Downstream>,
    node: Option<mpsc::Sender<crate::services::node_ws_manager::WsProxyFrame>>,
    harness: Harness,
    _tasks: tokio::task::JoinSet<()>,
}
impl Running {
    async fn new(f: &Fixture, node: bool, enabled: bool) -> Self {
        Self::with_duration(f, node, enabled, None).await
    }
    async fn with_duration(
        f: &Fixture,
        node: bool,
        enabled: bool,
        duration: Option<std::time::Duration>,
    ) -> Self {
        let mut state = crate::test_utils::test_app_state(f.db.clone());
        state.billing = Arc::new(f.service.clone());
        let metered = fixtures::reserve(f, BillingMetric::Tokens, true).await;
        let (tx, rx) = mpsc::channel(32);
        let mut tasks = tokio::task::JoinSet::new();
        let (downstream, direct) = if node {
            (None, None)
        } else {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let accept = async {
                let (tcp, _) = listener.accept().await.unwrap();
                tokio_tungstenite::accept_async(tokio_tungstenite::MaybeTlsStream::Plain(tcp))
                    .await
                    .unwrap()
            };
            let connect = tokio_tungstenite::connect_async(format!("ws://{address}"));
            let (server, client) = tokio::join!(accept, connect);
            (Some(client.unwrap().0), Some(server))
        };
        let (closed_tx, closed) = oneshot::channel();
        let (finished_tx, finished) = oneshot::channel();
        let harness = Harness {
            state,
            metered,
            downstream: Arc::new(Mutex::new(downstream)),
            node_frames: Arc::new(Mutex::new(Some(rx))),
            enabled,
            duration,
            abort: Default::default(),
            completion: Arc::new(Mutex::new(Some(BridgeCompletion {
                closed: closed_tx,
                finished: finished_tx,
            }))),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route("/realtime", axum::routing::get(serve_bridge))
            .with_state(harness.clone());
        tasks.spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = tokio_tungstenite::connect_async(format!("ws://{address}/realtime"))
            .await
            .unwrap()
            .0;
        Self {
            closed,
            finished,
            client,
            direct,
            node: node.then_some(tx),
            harness,
            _tasks: tasks,
        }
    }
    async fn text(&mut self, text: &str) {
        if let Some(ws) = &mut self.direct {
            ws.send(Message::Text(text.into())).await.unwrap();
        } else {
            self.node
                .as_ref()
                .unwrap()
                .send(crate::services::node_ws_manager::WsProxyFrame::Text(
                    text.into(),
                ))
                .await
                .unwrap();
        }
        let received = tokio::time::timeout(std::time::Duration::from_secs(3), self.client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(received.into_text().unwrap().as_str(), text);
    }
    async fn binary(&mut self, bytes: &[u8]) {
        if let Some(ws) = &mut self.direct {
            ws.send(Message::Binary(bytes.to_vec().into()))
                .await
                .unwrap();
        } else {
            self.node
                .as_ref()
                .unwrap()
                .send(crate::services::node_ws_manager::WsProxyFrame::Binary(
                    bytes.to_vec(),
                ))
                .await
                .unwrap();
        }
        assert_eq!(
            self.client
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .as_ref(),
            bytes
        );
    }
    async fn finish(&mut self) {
        if let Some(ws) = &mut self.direct {
            ws.send(Message::Close(None)).await.unwrap();
        } else {
            self.node
                .as_ref()
                .unwrap()
                .send(crate::services::node_ws_manager::WsProxyFrame::Closed {
                    code: None,
                    reason: None,
                })
                .await
                .unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), &mut self.closed)
            .await
            .expect("WebSocket bridge did not finish after closing the upstream")
            .expect("bridge task dropped before closing");
        (&mut self.finished)
            .await
            .expect("bridge task dropped before settlement completed");
    }
    async fn crash(&mut self) {
        self.harness.abort.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), &mut self.finished)
            .await
            .unwrap()
            .expect("bridge task dropped during crash");
    }
}

#[tokio::test]
async fn realtime_ws_bridges_crash_after_delivered_report_recovers_hold() {
    for node in [false, true] {
        let f = Fixture::new().await;
        let mut running = Running::new(&f, node, true).await;
        running.text(&fixtures::done("response", 3)).await;
        assert!(f.row().await.forwarded);
        assert!(f.row().await.quantity.is_none());
        running.crash().await;
        fixtures::expire(&f).await;
        fixtures::recover(&f).await;
        fixtures::recover(&f).await;
        f.charged(3).await;
        drop(running);
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn realtime_ws_bridges_command_counts() {
    for node in [false, true] {
        for enabled in [false, true] {
            let f = Fixture::new().await;
            f.count();
            let mut running = Running::new(&f, node, enabled).await;
            let initialization = f.count();
            for _ in 0..200 {
                running
                    .text(r#"{"type":"response.audio.delta","delta":"audio"}"#)
                    .await;
            }
            running.binary(&[0, 1, 2, 3]).await;
            let chunks = f.count();
            assert_eq!(chunks, 0);
            running.text(&fixtures::done("one", 3)).await;
            let first = f.count();
            assert_eq!(first, usize::from(enabled));
            running.text(&fixtures::done("one", 3)).await;
            let duplicate = f.count();
            assert_eq!(duplicate, 0);
            running.text(&fixtures::done("two", 5)).await;
            let second = f.count();
            assert_eq!(second, usize::from(enabled));
            running.finish().await;
            let terminal = f.count();
            println!(
                "realtime command counts node={node} checkpoint={enabled}: admission+forward={initialization}, content/audio={chunks}, first={first}, duplicate={duplicate}, second={second}, terminal={terminal}, total={}",
                initialization + first + second + terminal
            );
            f.charged(8).await;
            drop(running);
            f.db.drop().await.unwrap();
        }
    }
}
#[tokio::test]
async fn realtime_ws_bridges_faults_preserve_bytes_and_normal_settlement() {
    for node in [false, true] {
        for fault in ["database_error", "no_match", "timeout"] {
            let capture = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::WARN)
                .with_writer(capture.reopen().unwrap())
                .finish();
            let _subscriber = tracing::subscriber::set_default(subscriber);
            tracing::callsite::rebuild_interest_cache();
            let f = Fixture::new().await;
            let mut running = Running::new(&f, node, true).await;
            match fault {
                "database_error" => {
                    f.db.run_command(doc! {"collMod":"usage_meter","validator":{"realtime_session.sequence":{"$lte":0}},"validationLevel":"strict"}).await.unwrap();
                }
                "no_match" => {
                    f.service
                        .settle(
                            &running.harness.metered,
                            PlatformUsage::llm_completion(0, 3),
                            None,
                            None,
                        )
                        .await
                        .unwrap();
                }
                "timeout" => {
                    fixtures::stall_once(&f.db);
                }
                _ => unreachable!(),
            }
            let started = std::time::Instant::now();
            running.text(&fixtures::done("one", 3)).await;
            running.text(&fixtures::done("two", 5)).await;
            assert!(started.elapsed() < std::time::Duration::from_secs(2));
            running.finish().await;
            f.charged(if fault == "no_match" { 3 } else { 8 }).await;
            let logs = std::fs::read_to_string(capture.path()).unwrap();
            let failures: Vec<_> = logs
                .lines()
                .filter(|l| l.contains("Realtime usage checkpoint disabled"))
                .collect();
            assert_eq!(failures.len(), 1, "{logs}");
            assert!(failures[0].contains(fault));
            assert!(!failures[0].contains("total_tokens"));
            drop(running);
            f.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn realtime_ws_bridges_limit_closes_quiet_sessions_and_cancels_blocked_observation() {
    for node in [false, true] {
        for blocked in [false, true] {
            let f = Fixture::new().await;
            if blocked {
                fixtures::stall_once(&f.db);
            }
            let mut running =
                Running::with_duration(&f, node, true, Some(std::time::Duration::from_millis(200)))
                    .await;
            if blocked {
                let text = fixtures::done("one", 3);
                if let Some(ws) = &mut running.direct {
                    ws.send(Message::Text(text.into())).await.unwrap();
                } else {
                    running
                        .node
                        .as_ref()
                        .unwrap()
                        .send(crate::services::node_ws_manager::WsProxyFrame::Text(text))
                        .await
                        .unwrap();
                }
            }
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                let received = running.client.next().await;
                assert!(
                    matches!(received, Some(Ok(Message::Close(None))) | None),
                    "{received:?}"
                );
                (&mut running.closed)
                    .await
                    .expect("bridge task dropped before closing");
            })
            .await
            .expect("WebSocket bridge exceeded its close deadline");
            (&mut running.finished)
                .await
                .expect("bridge task dropped before settlement completed");
            fixtures::settled(&f, if blocked { 3 } else { 0 }).await;
            drop(running);
            f.db.drop().await.unwrap();
        }
    }
}

#[cfg(unix)]
fn frame_cpu_ns() -> u128 {
    let mut stamp = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid writable storage is supplied for the current thread clock.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut stamp) },
        0
    );
    stamp.tv_sec as u128 * 1_000_000_000 + stamp.tv_nsec as u128
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "CPU benchmark: run serially with the local MongoDB fixture"]
async fn realtime_ws_frame_cpu_per_megabyte() {
    let f = Fixture::new().await;
    let metered = fixtures::reserve(&f, BillingMetric::Tokens, true).await;
    let text =
        serde_json::json!({"type":"response.audio.delta","delta":"x".repeat(256)}).to_string();
    let frames = (16_usize * 1024 * 1024).div_ceil(text.len());
    let mb = frames as f64 * text.len() as f64 / 1_000_000.0;
    let mut samples = [Vec::new(), Vec::new()];
    for trial in 0..5 {
        for offset in 0..2 {
            let mode = (trial + offset) % 2;
            // Start outside the CPU/count interval; neither mode sees a done
            // boundary. Reset to an independent row for the next trial.
            let mut route = metered.route.clone().unwrap();
            route.billing_request_id = uuid::Uuid::new_v4().to_string();
            let current = f.service.open(&route).await.unwrap();
            let mut session = f
                .service
                .mark_realtime_forwarded(&current, true, true)
                .await
                .unwrap();
            let deadline = session.as_ref().unwrap().deadline.clone();
            let mut collector = llm_usage_service::RealtimeLlmUsageCollector::new(true);
            f.count();
            let started = frame_cpu_ns();
            if mode == 0 {
                for _ in 0..frames {
                    collector.observe_downstream_text(&text);
                    tokio::task::yield_now().await;
                }
            } else {
                deadline
                    .run(async {
                        for _ in 0..frames {
                            if deadline.expired() {
                                break;
                            }
                            observe_realtime_boundary(&mut collector, &mut session, &text, 0).await;
                            if deadline.expired() {
                                break;
                            }
                            // Both modes yield once/frame to model a socket wake.
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
            }
            std::hint::black_box(collector.finalize());
            samples[mode].push((frame_cpu_ns() - started) as f64 / 1_000_000.0 / mb);
            assert_eq!(f.count(), 0, "content frames issue no DB commands");
        }
    }
    for (name, mut samples) in ["before", "after"].into_iter().zip(samples) {
        samples.sort_by(f64::total_cmp);
        println!(
            "REALTIME_WS_FRAME_CPU {name} median_ms_per_MB={:.3} samples={samples:?}",
            samples[2]
        );
    }
    f.db.drop().await.unwrap();
}
