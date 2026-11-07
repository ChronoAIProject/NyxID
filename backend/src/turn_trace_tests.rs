use super::*;
use axum::{body::to_bytes, http::StatusCode};

#[test]
fn callback_requires_exact_authenticated_turn_and_valid_parent() {
    let turn = Uuid::new_v4().to_string();
    let root = TurnContext::for_turn(&turn).unwrap();
    let mut headers = HeaderMap::new();
    assert!(TurnContext::callback(&headers, Some(&turn)).is_none());
    headers.insert("traceparent", root.traceparent().parse().unwrap());
    assert_eq!(
        TurnContext::callback(&headers, Some(&turn))
            .unwrap()
            .turn_id(),
        turn
    );
    assert!(TurnContext::callback(&headers, None).is_none());
    assert!(TurnContext::callback(&headers, Some(&Uuid::new_v4().to_string())).is_none());
    let oversized = format!(
        "{}-{}",
        root.traceparent().replacen("00-", "01-", 1),
        "a".repeat(256)
    );
    headers.insert("traceparent", oversized.parse().unwrap());
    assert!(TurnContext::callback(&headers, Some(&turn)).is_none());
    headers.insert("traceparent", root.traceparent().parse().unwrap());
    headers.append("traceparent", root.traceparent().parse().unwrap());
    assert!(TurnContext::callback(&headers, Some(&turn)).is_none());
    for invalid in [
        "private-canary",
        "00-00000000000000000000000000000000-0000000000000001-01",
    ] {
        headers.insert("traceparent", invalid.parse().unwrap());
        assert!(TurnContext::callback(&headers, Some(&turn)).is_none());
    }
}

#[test]
fn headers_keep_idempotency_and_never_copy_untrusted_context() {
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    let mut headers = vec![
        ("Idempotency-Key".into(), "unchanged:continuation:2".into()),
        ("traceparent".into(), "private-canary".into()),
        ("baggage".into(), "private-canary".into()),
        ("tracestate".into(), "private-canary".into()),
        ("X-Request-ID".into(), "private-canary".into()),
    ];
    root.inject(&mut headers);
    assert_eq!(headers.len(), 3);
    assert_eq!(headers[0].1, "unchanged:continuation:2");
    assert!(!format!("{headers:?}").contains("private-canary"));
    let next = root.child();
    assert_eq!(root.turn, next.turn);
    assert_ne!(root.span, next.span);
    assert_ne!(root.request, next.request);
    assert_eq!(
        root.span,
        TurnContext::for_turn(&root.turn_id()).unwrap().span
    );
}

#[tokio::test]
async fn admission_link_is_scoped_and_audit_keeps_persisted_source_turn() {
    let ingress = Ingress::new();
    let id = Uuid::new_v4().to_string();
    CAPTURE.scope(std::sync::Mutex::new(vec![]), async {
        ingress.scope(async {
            admitted(&id, chrono::Utc::now(), None);
            let root = ingress.admitted().unwrap();
            scope(Some(root), async {
                assert_eq!(audit_data(None).unwrap()["turn_id"], id);
                let old = Uuid::new_v4().to_string();
                assert_eq!(audit_data(Some(serde_json::json!({"turn_id":old}))).unwrap()["turn_id"], old);
                assert_eq!(audit_data(Some(serde_json::json!({"trace_id":"untrusted"}))).unwrap()["trace_id"], root.turn.simple().to_string());
            }).await;
        }).await;
        assert!(TurnContext::current().is_none());
        CAPTURE.with(|rows| {
            let rows = rows.lock().unwrap();
            assert_eq!(rows.len(), 3);
            assert_eq!(rows[2].ingress_request_id, Some(ingress.request_id));
        });
    }).await;
}

#[tokio::test]
async fn stream_is_not_drained_and_cancellation_is_incomplete() {
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    CAPTURE
        .scope(std::sync::Mutex::new(vec![]), async {
            let pending = response(Some(root), Stage::Proxy, async {
                Ok(Response::builder()
                    .body(Body::from_stream(futures::stream::pending::<
                        Result<Bytes, std::io::Error>,
                    >()))
                    .unwrap())
            })
            .await
            .unwrap();
            assert_eq!(pending.status(), StatusCode::OK);
            drop(pending);
            CAPTURE.with(|rows| {
                let rows = rows.lock().unwrap();
                assert_eq!(rows.len(), 2);
                assert!(matches!(rows[1].outcome, Some(Outcome::Unknown)));
            });
            let response = response(Some(root), Stage::Proxy, async {
                Ok(Response::new(Body::from("content-canary")))
            })
            .await
            .unwrap();
            assert_eq!(
                to_bytes(response.into_body(), 100).await.unwrap(),
                "content-canary"
            );
            CAPTURE.with(|rows| {
                let rows = rows.lock().unwrap();
                assert!(matches!(
                    rows.last().unwrap().outcome,
                    Some(Outcome::Completed)
                ));
                for row in rows.iter() {
                    let json = serde_json::to_string(row).unwrap();
                    assert!(!json.contains("canary"));
                    assert!(json.len() < MAX_RECORD_BYTES);
                    assert_eq!(row.target, TARGET);
                }
            });
        })
        .await;
}

#[test]
fn queue_has_a_fixed_nonblocking_memory_bound() {
    let (sender, _receiver) = mpsc::sync_channel(CAPACITY);
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    for _ in 0..CAPACITY {
        sender
            .try_send(Record::new(root, Stage::Execution, Phase::Start))
            .unwrap();
    }
    assert!(matches!(
        sender.try_send(Record::new(root, Stage::Execution, Phase::Start)),
        Err(mpsc::TrySendError::Full(_))
    ));
    assert!(std::mem::size_of::<Record>() * CAPACITY < 16 * 1024 * 1024);
    assert!(!std::mem::needs_drop::<Record>());
}

#[test]
fn reusable_encoder_preserves_hex_ids_and_clears_optional_fields() {
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    let mut record = Record::new(root.child(), Stage::Gateway, Phase::End);
    record.parent_span_id = Some(1);
    record.duration_ms = Some(42);
    record.outcome = Some(Outcome::Completed);
    record.http_status = Some(200);
    let mut bytes = Vec::with_capacity(MAX_RECORD_BYTES);
    let allocation = bytes.as_ptr();
    assert!(encode(&record, &mut bytes));
    assert_eq!(bytes.last(), Some(&b'\n'));
    let child: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(child["parent_span_id"], "0000000000000001");
    assert_eq!(child["span_id"], format!("{:016x}", record.span_id));
    assert_eq!(child["duration_ms"], 42);

    let ingress = Ingress::new().record(Phase::Start);
    assert!(encode(&ingress, &mut bytes));
    assert_eq!(bytes.as_ptr(), allocation);
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    let decoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for key in [
        "turn_id",
        "trace_id",
        "parent_span_id",
        "duration_ms",
        "outcome",
        "http_status",
    ] {
        assert!(decoded.get(key).is_none(), "stale optional field: {key}");
    }
}

#[tokio::test]
async fn empty_response_and_stream_error_have_terminal_outcomes() {
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    CAPTURE
        .scope(std::sync::Mutex::new(vec![]), async {
            drop(
                response(Some(root), Stage::Proxy, async {
                    Ok(Response::new(Body::empty()))
                })
                .await
                .unwrap(),
            );
            let failing = response(Some(root), Stage::Proxy, async {
                Ok(Response::new(Body::from_stream(futures::stream::once(
                    async { Err::<Bytes, _>(std::io::Error::other("private-error-canary")) },
                ))))
            })
            .await
            .unwrap();
            assert!(to_bytes(failing.into_body(), 100).await.is_err());
            CAPTURE.with(|rows| {
                let rows = rows.lock().unwrap();
                assert!(matches!(rows[1].outcome, Some(Outcome::Completed)));
                assert!(matches!(rows[3].outcome, Some(Outcome::Failed)));
                assert!(!serde_json::to_string(&*rows).unwrap().contains("canary"));
            });
        })
        .await;
}

#[tokio::test]
async fn rejected_ingress_has_no_turn_and_detached_work_keeps_association() {
    let ingress = Ingress::new();
    let turn = Uuid::new_v4().to_string();
    CAPTURE
        .scope(std::sync::Mutex::new(vec![]), async {
            ingress.scope(async {}).await;
            ingress.finish(403);
            CAPTURE
                .with(|rows| assert!(rows.lock().unwrap().iter().all(|row| row.turn_id.is_none())));
            ingress
                .scope(async {
                    admitted(&turn, chrono::Utc::now(), None);
                    let seen = spawn(async { audit_data(None).unwrap() }).await.unwrap();
                    assert_eq!(seen["turn_id"], turn);
                })
                .await;
        })
        .await;
}

#[path = "turn_trace_db_tests.rs"]
mod database;

#[tokio::test]
async fn bounded_writer_healthy_and_full_boundary_cost() {
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    for saturated in [false, true] {
        let (sender, receiver) = mpsc::sync_channel(CAPACITY);
        if saturated {
            for _ in 0..CAPACITY {
                sender
                    .try_send(Record::new(root, Stage::Engine, Phase::Start))
                    .unwrap();
            }
        }
        let dropped_before = DROPPED.load(Ordering::Relaxed);
        let mut samples = Vec::with_capacity(10_000);
        TEST_SINK
            .scope(sender, async {
                for _ in 0..10_000 {
                    let start = Instant::now();
                    let mut operation = Operation::start(Some(root), Stage::Engine);
                    let mut headers = vec![];
                    operation.context().unwrap().inject(&mut headers);
                    operation.finish(Outcome::Completed, Some(200), None);
                    samples.push(start.elapsed().as_nanos());
                    if !saturated {
                        while receiver.try_recv().is_ok() {}
                    }
                }
            })
            .await;
        samples.sort_unstable();
        let p99 = samples[9900];
        eprintln!("turn trace saturated={saturated}: p99 boundary {p99} ns");
        assert!(
            p99 < 100_000,
            "boundary p99 exceeds 100 microseconds: {p99} ns"
        );
        if saturated {
            assert!(DROPPED.load(Ordering::Relaxed) >= dropped_before + 20_000);
        }
        assert!((std::mem::size_of::<Record>() + 16) * CAPACITY < 16 * 1024 * 1024);
    }
}

#[tokio::test]
async fn turn_trace_predispatch_budget_with_healthy_and_full_sink() {
    for saturated in [false, true] {
        let (sender, receiver) = mpsc::sync_channel(CAPACITY);
        let turn = Uuid::new_v4().to_string();
        let root = TurnContext::for_turn(&turn).unwrap();
        if saturated {
            for _ in 0..CAPACITY {
                sender
                    .try_send(Record::new(root, Stage::Engine, Phase::Start))
                    .unwrap();
            }
        }
        let mut samples = Vec::with_capacity(2000);
        TEST_SINK
            .scope(sender, async {
                for _ in 0..2000 {
                    let started = Instant::now();
                    let ingress = Ingress::new();
                    ingress
                        .scope(async {
                            admitted(&turn, chrono::Utc::now(), None);
                            let root = ingress.admitted().unwrap();
                            let mut engine = Operation::start(Some(root), Stage::Engine);
                            let mut proxy = Operation::start(engine.context(), Stage::ProxyHeaders);
                            let mut headers = vec![];
                            proxy.context().unwrap().inject(&mut headers);
                            std::hint::black_box(audit_data(None));
                            proxy.finish(Outcome::Completed, Some(200), None);
                            engine.finish(Outcome::Completed, Some(200), None);
                        })
                        .await;
                    ingress.finish(200);
                    samples.push(started.elapsed().as_nanos());
                    if !saturated {
                        while receiver.try_recv().is_ok() {}
                    }
                }
            })
            .await;
        samples.sort_unstable();
        let p95 = samples[1900];
        eprintln!("turn trace saturated={saturated}: p95 pre-dispatch bookkeeping {p95} ns");
        assert!(
            p95 < 1_000_000,
            "pre-dispatch bookkeeping exceeds 1 ms: {p95} ns"
        );
    }
}

#[tokio::test]
async fn many_stream_chunks_do_not_produce_per_chunk_records() {
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    let (_, records) = capture(async {
        let result = response(Some(root), Stage::Engine, async {
            Ok(Response::new(Body::from_stream(futures::stream::iter(
                (0..1000)
                    .map(|_| Ok::<_, std::io::Error>(Bytes::from_static(b"private-token-canary"))),
            ))))
        })
        .await
        .unwrap();
        assert_eq!(
            to_bytes(result.into_body(), 100_000).await.unwrap().len(),
            20_000
        );
    })
    .await;
    assert_eq!(records.len(), 2);
    assert_eq!(records[1]["outcome"], "completed");
    assert!(!serde_json::to_string(&records).unwrap().contains("canary"));
}

/// Paired samples include actual HTTP requests, concurrent execution and
/// background serialization. Keep the mock delay stable so this measures the
/// turn-like workload rather than CPU-only synthetic span throughput.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual release performance check on the shared build host"]
async fn turn_trace_mocked_gateway_throughput_budget() {
    use crate::services::nyxbot_gateway_service::gateway_call_at_base;
    use futures::StreamExt;

    const REQUESTS: usize = 1024;
    const CONCURRENCY: usize = 32;
    let server = wiremock::MockServer::builder()
        .disable_request_recording()
        .start()
        .await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(20))
                .set_body_json(serde_json::json!({"accepted":true})),
        )
        .mount(&server)
        .await;
    let root = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    let base = server.uri();
    let (healthy, received) = mpsc::sync_channel::<Record>(CAPACITY);
    let writer = std::thread::spawn(move || {
        let mut bytes = Vec::with_capacity(MAX_RECORD_BYTES);
        while let Ok(record) = received.recv() {
            assert!(encode(&record, &mut bytes));
            std::hint::black_box(&bytes);
        }
    });
    let (full, _blocked_receiver) = mpsc::sync_channel(CAPACITY);
    for _ in 0..CAPACITY {
        full.try_send(Record::new(root, Stage::Engine, Phase::Start))
            .unwrap();
    }
    let mut samples = [Vec::new(), Vec::new(), Vec::new()];
    for sample in 0..14 {
        // Rotate ordering to avoid assigning a warm/cold interval to one mode.
        for offset in 0..3 {
            let mode = (sample + offset) % 3;
            let sink = if mode == 2 {
                full.clone()
            } else {
                healthy.clone()
            };
            let dropped_before = DROPPED.load(Ordering::Relaxed);
            let start = Instant::now();
            TEST_SINK
                .scope(sink, async {
                    futures::stream::iter((0..REQUESTS).map(|_| {
                        scope((mode > 0).then_some(root), async {
                            gateway_call_at_base(
                                &base,
                                reqwest::Method::POST,
                                "/reply",
                                "fixture",
                                None,
                                None,
                            )
                            .await
                            .unwrap();
                        })
                    }))
                    .buffer_unordered(CONCURRENCY)
                    .collect::<Vec<_>>()
                    .await;
                })
                .await;
            let elapsed = start.elapsed().as_nanos();
            let dropped = DROPPED.load(Ordering::Relaxed) - dropped_before;
            assert_eq!(
                dropped,
                if mode == 2 { (2 * REQUESTS) as u64 } else { 0 },
                "the healthy sink must not drop records; the full sink must stay full"
            );
            if sample > 1 {
                samples[mode].push(elapsed);
            }
        }
    }
    drop(healthy);
    writer.join().unwrap();
    for (mode, sample) in samples.iter_mut().enumerate() {
        sample.sort_unstable();
        eprintln!("turn trace mocked HTTP mode={mode} sorted samples (ns)={sample:?}");
    }
    let baseline = samples[0][samples[0].len() / 2] as f64;
    let mut within_budget = true;
    for mode in [1, 2] {
        let elapsed = samples[mode][samples[mode].len() / 2] as f64;
        let regression = (elapsed / baseline - 1.0) * 100.0;
        eprintln!(
            "turn trace mocked HTTP mode={mode} requests={REQUESTS} concurrency={CONCURRENCY} median={elapsed} ns baseline={baseline} ns throughput regression={regression:.3}%"
        );
        within_budget &= elapsed < baseline * 1.01;
    }
    assert!(
        within_budget,
        "mocked HTTP throughput exceeds 1% budget; see both sink measurements above"
    );
}

#[tokio::test]
async fn stale_callback_never_forwards_a_claim_or_adopts_an_ambient_turn() {
    let current = Uuid::new_v4().to_string();
    let stale = TurnContext::for_turn(&Uuid::new_v4().to_string()).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("traceparent", stale.traceparent().parse().unwrap());
    let outgoing = vec![
        ("traceparent".into(), stale.traceparent()),
        ("tracestate".into(), "private-state".into()),
        ("baggage".into(), "private-baggage".into()),
        ("x-request-id".into(), "private-request".into()),
        ("Idempotency-Key".into(), "unchanged".into()),
    ];
    let ingress = Ingress::new();
    ingress
        .scope(async {
            admitted(&current, chrono::Utc::now(), None);
            let callback = TurnContext::for_request(&headers, Some(&current));
            assert!(callback.context().is_none());
            callback
                .scope(async {
                    assert!(audit_data(None).is_none());
                    let mut forwarded = outgoing.clone();
                    inject_egress(&mut forwarded);
                    assert_eq!(forwarded.len(), 2);
                    assert_eq!(forwarded[0], ("Idempotency-Key".into(), "unchanged".into()));
                    assert!(Uuid::parse_str(&forwarded[1].1).is_ok());
                    let detached = spawn(async {
                        assert!(TurnContext::current().is_none());
                        assert!(audit_data(None).is_none());
                        turn_callback()
                    })
                    .await
                    .unwrap();
                    assert!(detached);
                    TurnContext::for_request(&headers, None)
                        .scope(async {
                            let mut nested = outgoing.clone();
                            inject_egress(&mut nested);
                            assert!(!nested.iter().any(|(key, _)| key == "traceparent"));
                        })
                        .await;
                })
                .await;
            // Unrelated traffic preserves the existing header forwarding contract.
            let ordinary = TurnContext::for_request(&headers, None);
            ordinary
                .scope(async {
                    let mut forwarded = outgoing.clone();
                    inject_egress(&mut forwarded);
                    assert_eq!(forwarded, outgoing);
                })
                .await;
            // A response body executes after the HTTP handler's task-local scope ends.
            let body = callback
                .response(Stage::Proxy, async {
                    Ok(Response::new(Body::from_stream(futures::stream::once(
                        async {
                            assert!(turn_callback());
                            assert!(TurnContext::current().is_none());
                            Ok::<_, std::io::Error>(Bytes::from_static(b"unchanged"))
                        },
                    ))))
                })
                .await
                .unwrap();
            assert_eq!(to_bytes(body.into_body(), 64).await.unwrap(), "unchanged");
        })
        .await;
}
