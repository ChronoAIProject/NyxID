use super::*;
use std::sync::OnceLock;

fn stalled_writes() -> &'static Mutex<std::collections::HashSet<String>> {
    static STALLED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    STALLED.get_or_init(Default::default)
}

pub(super) fn take_stalled_write(database: &str) -> bool {
    stalled_writes().lock().unwrap().remove(database)
}

async fn direct_stream_fault(fault: &str) {
    use crate::services::proxy_http_response::{
        HttpResponseContext, ProxyExchangeDiagnostics, relay_response,
    };
    use axum::http::{HeaderMap, Method, StatusCode};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

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
    let metered = f.open(true).await;
    match fault {
        "database_error" => {
            f.db.run_command(doc! {"collMod":USAGE_METER,
            "validator":{"usage_checkpoint":{"$exists":false}},"validationLevel":"strict"})
                .await
                .unwrap();
        }
        "no_match" => {
            // A competing normal finalizer wins before observation. Replaying
            // this stream's finalizer must retain its one charge.
            f.service
                .settle(&metered, PlatformUsage::llm_completion(0, 3), None, None)
                .await
                .unwrap();
        }
        "timeout" => {
            stalled_writes()
                .lock()
                .unwrap()
                .insert(f.db.name().to_owned());
        }
        _ => unreachable!(),
    }
    let server = MockServer::start().await;
    let mut body = b"data: {\"choices\":[{\"delta\":{\"content\":\"fixture\"}}]}\n\n".to_vec();
    body.extend(chat(3));
    body.extend(chat(3));
    body.extend(b"data: [DONE]\n\n");
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body.clone(), "text/event-stream"))
        .mount(&server)
        .await;
    let downstream = reqwest::get(format!("{}/v1/chat/completions", server.uri()))
        .await
        .unwrap();
    let mut state = crate::test_utils::test_app_state(f.db.clone());
    state.billing = Arc::new(f.service.clone());
    let context = HttpResponseContext {
        service_id: "checkpoint-fixture".into(),
        path: "chat/completions".into(),
        nyxagent_admission: false,
        model_request: None,
        response_builder: axum::response::Response::builder(),
        status: StatusCode::OK,
        is_sse: true,
        should_stream: true,
        usage_context: Some(crate::services::llm_usage_service::UsageAuditContext {
            db: f.db.clone(),
            user_id: f.owner.clone(),
            provider_slug: None,
            service_id: None,
            model: None,
            path: "chat/completions".into(),
            api_key_id: None,
            api_key_name: None,
        }),
        metered,
        request_body_len: 0,
        resale_metric: None,
        downstream_cancellation: tokio_util::sync::CancellationToken::new(),
        exchange_diagnostics: ProxyExchangeDiagnostics::new(
            std::time::Instant::now(),
            0,
            None,
            &Method::GET,
            StatusCode::OK,
            "sse",
            &HeaderMap::new(),
        ),
    };
    let started = std::time::Instant::now();
    let response = relay_response(&state, context, downstream).await.unwrap();
    let received = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await;
    assert!(received.is_ok(), "checkpoint stalled the caller: {fault}");
    let received = received.unwrap();
    assert!(
        received.is_ok(),
        "checkpoint broke the caller: {fault}: {received:?}"
    );
    assert_eq!(received.unwrap().as_ref(), body.as_slice());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let raw =
                f.db.collection::<bson::Document>(BILLING_WALLET)
                    .find_one(doc! {"owner_id": &f.owner})
                    .await
                    .unwrap()
                    .unwrap();
            let idle = raw
                .get("active_settlement")
                .is_none_or(|lock| lock == &bson::Bson::Null);
            let saved: BillingWallet = bson::from_document(raw).unwrap();
            if f.row().await.released
                && saved.reserved_credits == Credits::ZERO
                && saved.pending_lago_debits == Credits::from_whole(3)
                && idle
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.recover().await;
    f.charged(3).await;
    let logs = std::fs::read_to_string(capture.path()).unwrap();
    let failures: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("Stream usage checkpoint disabled"))
        .collect();
    assert_eq!(failures.len(), 1, "{logs}");
    assert!(failures[0].contains(fault));
    assert!(!failures[0].contains("fixture"));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_checkpoint_r2_database_error_preserves_direct_stream_and_settlement() {
    direct_stream_fault("database_error").await;
}
#[tokio::test]
async fn usage_checkpoint_r2_no_match_preserves_direct_stream_and_settlement() {
    direct_stream_fault("no_match").await;
}
#[tokio::test]
async fn usage_checkpoint_r2_timeout_preserves_direct_stream_and_settlement() {
    direct_stream_fault("timeout").await;
}

#[tokio::test]
async fn usage_checkpoint_r2_writes_only_recoverable_reports() {
    let mut counts = Vec::new();
    for protocol in ["xai", "gemini", "anthropic", "openai", "private"] {
        let f = Fixture::new().await;
        let metered = f.open(true).await;
        let eligible = protocol != "private"
            && crate::services::billing::usage_checkpoint::supports_path(if protocol == "gemini" {
                "models/gemini:streamGenerateContent"
            } else {
                "chat/completions"
            });
        let mut recorder = f.service.stream_checkpoints(&metered, eligible);
        assert_eq!(recorder.is_enabled(), eligible);
        let mut events = Vec::new();
        match protocol {
            "xai" | "gemini" => {
                for n in 1..=200 {
                    let value = if protocol == "xai" {
                        serde_json::json!({"choices":[{"delta":{"content":"fixture"},"finish_reason":if n==200 {Some("stop")} else {None}}],
                        "usage":{"prompt_tokens":1,"completion_tokens":n,"total_tokens":n+1}})
                    } else {
                        serde_json::json!({"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":n,"totalTokenCount":n+1}})
                    };
                    events.push(format!("data: {value}\n\n").into_bytes());
                }
            }
            "anthropic" => {
                events.push(message(
                    "message_start",
                    serde_json::json!({"input_tokens":1,"output_tokens":0}),
                    false,
                ));
                for n in 1..=4 {
                    events.push(message(
                        "message_delta",
                        serde_json::json!({"output_tokens":n}),
                        n == 4,
                    ));
                }
            }
            _ => events.push(chat(3)),
        }
        f.count();
        for event in events {
            recorder.observe(&event).await.unwrap();
        }
        let count = f.count();
        println!("CHECKPOINT_R2_WRITES {protocol}={count}");
        counts.push(count);
        f.db.drop().await.unwrap();
    }
    assert_eq!(counts, [1, 0, 1, 1, 0]);
}

#[cfg(unix)]
fn cpu_ns() -> u128 {
    let mut stamp = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid, writable timespec is supplied for this thread's clock.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut stamp) },
        0
    );
    stamp.tv_sec as u128 * 1_000_000_000 + stamp.tv_nsec as u128
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "CPU benchmark: run serially against the local fixture"]
async fn usage_checkpoint_r2_direct_proxy_cpu_per_megabyte() {
    use crate::services::{llm_usage_service, sse_parser};
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    let frame = format!(
        "data: {}\n\n",
        serde_json::json!({"choices":[{"delta":{"content":"x".repeat(256)}}]})
    );
    let body = frame.repeat((1024_usize * 1024).div_ceil(frame.len()));
    let mut samples = [Vec::new(), Vec::new(), Vec::new()];
    f.count();
    for trial in 0..5 {
        // Rotate order to avoid favoring one implementation's cache warmup.
        for offset in 0..3 {
            let mode = (trial + offset) % 3;
            let mut checkpoints = f.service.stream_checkpoints(&metered, true);
            let mut old_decoder = sse_parser::BoundedEventDecoder::default();
            let mut buffer = String::new();
            let mut accumulator = llm_usage_service::ReportedLlmUsageAccumulator::default();
            let start = cpu_ns();
            for _ in 0..16 {
                for chunk in body.as_bytes().chunks(4096) {
                    if mode == 2 {
                        crate::services::proxy_http_response::observe_sse_chunk(
                            &mut buffer,
                            &mut accumulator,
                            &mut checkpoints,
                            chunk,
                            true,
                        )
                        .await;
                    } else {
                        if mode == 1 {
                            // Replay Round 1's text-event hot loop: extra byte
                            // framing and three JSON reads (sequence inspection,
                            // event_inner, then the malformed-usage check).
                            for byte in chunk {
                                if let Some(Ok(event)) = old_decoder.push_byte(*byte) {
                                    std::hint::black_box(
                                        serde_json::from_str::<serde_json::Value>(&event.data)
                                            .unwrap(),
                                    );
                                    std::hint::black_box(
                                        serde_json::from_str::<serde_json::Value>(&event.data)
                                            .unwrap(),
                                    );
                                    std::hint::black_box(
                                        serde_json::from_str::<serde_json::Value>(&event.data)
                                            .unwrap(),
                                    );
                                }
                            }
                        }
                        buffer.push_str(&String::from_utf8_lossy(chunk));
                        while let Some(event) = sse_parser::parse_next_event(&mut buffer) {
                            if let Some((usage, mode)) =
                                llm_usage_service::extract_reported_usage_from_sse_event(
                                    event.event_type.as_deref(),
                                    &event.data,
                                )
                            {
                                accumulator.observe(usage, mode);
                            }
                        }
                    }
                }
            }
            std::hint::black_box(accumulator.finalize());
            let mb = body.len() as f64 * 16.0 / 1_000_000.0;
            samples[mode].push((cpu_ns() - start) as f64 / 1_000_000.0 / mb);
        }
    }
    assert_eq!(
        f.count(),
        0,
        "CPU measurement must contain no MongoDB commands"
    );
    for (label, mut values) in ["main", "round1", "round2"].into_iter().zip(samples) {
        values.sort_by(f64::total_cmp);
        println!(
            "CHECKPOINT_R2_CPU {label} median_ms_per_MB={:.3} samples={values:?}",
            values[2]
        );
    }
    f.db.drop().await.unwrap();
}
