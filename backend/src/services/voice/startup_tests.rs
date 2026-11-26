//! Request cancellation closes admitted work and settles only provider-confirmed usage.
use super::{credential_tests, credentials, openai, runtime, session, tests};
use crate::models::{
    assistant_voice::{VoiceKeySource, VoiceWindow, WINDOWS},
    assistant_voice_session::{COLLECTION_NAME, HEARTBEAT_SECONDS, VoiceSession},
    billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as LEDGER},
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    credits::Credits,
    service_billing::BillingMetric,
    usage_meter::{COLLECTION_NAME as USAGE, CredentialClass, UsageMeterRow, UsageStatus},
};
use axum::{
    Json, Router,
    extract::{WebSocketUpgrade, ws::Message},
    routing::{get, post},
};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use uuid::Uuid;

const CLOSE_BOUND: Duration = Duration::from_secs(12);

async fn request_row(db: &mongodb::Database, request: &str) -> VoiceSession {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(row) = db
                .collection::<VoiceSession>(COLLECTION_NAME)
                .find_one(doc! {"client_request_id":request})
                .await
                .unwrap()
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

async fn released(db: &mongodb::Database, id: &str) -> VoiceSession {
    assert!(CLOSE_BOUND.as_secs() < HEARTBEAT_SECONDS as u64 / 2);
    tokio::time::timeout(CLOSE_BOUND, async {
        loop {
            let row = db
                .collection::<VoiceSession>(COLLECTION_NAME)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap();
            if !row.live_slot {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("confirmed closure must release the slot before the heartbeat lapse")
}

#[derive(Clone, Copy, PartialEq)]
enum Window {
    BeforeAdmission,
    ProviderCreate,
    ProviderAttach,
    Connected,
}

async fn assert_settled(db: &mongodb::Database, call: &VoiceSession, paid: bool) {
    let request = format!("voice:{}:0", call.id);
    let windows: Vec<VoiceWindow> = db
        .collection(WINDOWS)
        .find(doc! {"session_id":&call.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        windows.len(),
        1,
        "one initial window, never a heartbeat extension"
    );
    let window = &windows[0];
    assert_eq!(window.billing_request_id, request);
    assert_eq!(window.reserved_seconds, 30);
    assert_eq!(
        window.observed_seconds, 4,
        "the normal-end fixture floors provider usage 4.9 to 4"
    );
    assert!(window.sealed && window.settled && !window.uncertain);
    let usage: Vec<UsageMeterRow> = db
        .collection(USAGE)
        .find(doc! {"billing_request_id":&request})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(usage.len(), if paid { 2 } else { 1 });
    assert!(
        usage
            .iter()
            .all(|row| row.status == UsageStatus::Finalized && row.forwarded && row.released)
    );
    let seconds: Vec<_> = usage
        .iter()
        .filter(|row| row.metric == BillingMetric::VoiceSeconds)
        .collect();
    assert_eq!(seconds.len(), 1, "exactly one duration meter row");
    let row = seconds[0];
    if paid {
        let tokens = usage
            .iter()
            .find(|row| row.metric == BillingMetric::InputTokens)
            .unwrap();
        assert_eq!(
            tokens.quantity,
            Some(0),
            "no native inference ran during the unconnected short call"
        );
        let funding = tokens.funding.as_ref().unwrap();
        assert!(funding.settled);
        assert_eq!(funding.wallet_funded, Some(Credits::ZERO));
        assert_eq!(
            db.collection::<BillingLedgerEntry>(LEDGER)
                .count_documents(doc! {"reference_id":&tokens.id,"movement":"usage_settled"})
                .await
                .unwrap(),
            0
        );
    }
    assert_eq!(row.status, UsageStatus::Finalized);
    assert_eq!(row.quantity, Some(4));
    assert!(row.forwarded && row.released);
    assert_eq!(
        row.credential_class,
        if paid {
            CredentialClass::NyxidManagedMaster
        } else {
            CredentialClass::UserOwned
        }
    );
    let entries: Vec<BillingLedgerEntry> = db
        .collection(LEDGER)
        .find(doc! {"reference_id":&row.id,"movement":"usage_settled"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    if paid {
        let funding = row.funding.as_ref().unwrap();
        assert!(funding.settled && funding.settled_at.is_some());
        assert_eq!(funding.wallet_funded, Some(Credits::from_whole(4)));
        assert_eq!(funding.wallet_charge_credits, Some(Credits::from_whole(4)));
        assert!(funding.allowance_reservations.is_empty() && funding.grant_reservations.is_empty());
        assert_eq!(
            entries.len(),
            1,
            "one charge, including after replayed finalization"
        );
        assert_eq!(entries[0].quantity, Some(4));
        let wallet = db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"_id":row.wallet_id.as_ref().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.balance_credits, Credits::from_whole(10_000));
        assert_eq!(wallet.pending_lago_debits, Credits::from_whole(4));
        assert_eq!(
            wallet.available_credits().unwrap(),
            Credits::from_whole(9996)
        );
    } else {
        assert!(row.wallet_id.is_none() && row.funding.is_none());
        assert!(entries.is_empty());
        assert_eq!(
            db.collection::<bson::Document>(WALLETS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<bson::Document>(LEDGER)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    assert!(
        call.responses.finalized(),
        "the default native Responses hold must also settle"
    );
    let responses_request = format!("voice:{}:responses", call.id);
    let response_rows: Vec<UsageMeterRow> = db
        .collection(USAGE)
        .find(doc! {"billing_request_id":&responses_request})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(response_rows.len(), 1);
    let response_row = &response_rows[0];
    assert_eq!(response_row.quantity, Some(0));
    assert_eq!(response_row.status, UsageStatus::Finalized);
    assert!(response_row.forwarded && response_row.released);
    assert_eq!(response_row.credential_class, row.credential_class);
    assert_eq!(response_row.wallet_id.is_some(), paid);
    if paid {
        let funding = response_row.funding.as_ref().unwrap();
        assert!(funding.settled);
        assert_eq!(funding.wallet_funded, Some(Credits::ZERO));
    } else {
        assert!(response_row.funding.is_none());
    }
    assert_eq!(
        db.collection::<BillingLedgerEntry>(LEDGER)
            .count_documents(doc! {"reference_id":&response_row.id,"movement":"usage_settled"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        if paid { 3 } else { 2 }
    );
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE)
            .count_documents(
                doc! {"billing_request_id":{"$in":[&request,&responses_request]},"status":{"$in":["reserved","forwarded"]}}
            )
            .await
            .unwrap(),
        0
    );
}

async fn cancelled_provider_start(
    window: Window,
    client_end: bool,
    paid: bool,
    invalid_answer: bool,
) {
    let (f, service) = credential_tests::fixture("voice_cancelled_start").await;
    let mut preferences = credential_tests::platform(&service);
    if !paid {
        preferences.key_source = VoiceKeySource::Own;
        let connection = credential_tests::connection(&f, &service, "cancelled-own").await;
        crate::services::assistant_team_service::set_grants(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            crate::services::assistant_team_service::GrantChange::Add(
                crate::models::assistant_agent::AgentGrants {
                    service_ids: vec![connection.clone()],
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
        preferences.connection_id = Some(connection);
    }
    let resolved = credentials::resolve(&f.state, &f.owner, &f.row.id, &preferences)
        .await
        .unwrap();
    let received = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let attaching = Arc::new(Notify::new());
    let attach_release = Arc::new(Notify::new());
    let closes = Arc::new(AtomicUsize::new(0));
    let create_received = received.clone();
    let create_release = release.clone();
    let attach_received = attaching.clone();
    let attach_gate = attach_release.clone();
    let close_count = closes.clone();
    let db = f.state.db.clone();
    let app = Router::new().route("/v1/live/sessions", post(move || {
        let received = create_received.clone();
        let release = create_release.clone();
        let db = db.clone();
        async move {
            let reservation = db.collection::<UsageMeterRow>(USAGE)
                .find_one(doc! {"metric":"voice_seconds"}).await.unwrap().unwrap();
            assert!(reservation.forwarded, "reserve and mark forwarding before provider create");
            assert_eq!(reservation.status, UsageStatus::Forwarded);
            assert_eq!(reservation.wallet_id.is_some(), paid);
            received.notify_one();
            release.notified().await;
            Json(json!({"session":{"id":"live_cancelled"},"transport":{"sdp":
                if invalid_answer { "invalid" } else { "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n" }}}))
        }
    })).route("/v1/live/sessions/live_cancelled/attach", get(move |ws: WebSocketUpgrade| {
        let received = attach_received.clone();
        let release = attach_gate.clone();
        let closes = close_count.clone();
        async move {
            received.notify_one();
            if window == Window::ProviderAttach { release.notified().await; }
            ws.on_upgrade(move |mut socket| async move {
                while let Some(Ok(Message::Text(text))) = socket.recv().await {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == "session.close" {
                        closes.fetch_add(1, Ordering::SeqCst);
                        socket.send(Message::Text(json!({"type":"session.closed","usage":{"seconds":4.9}})
                            .to_string().into())).await.unwrap();
                        break;
                    }
                }
            })
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let request = Uuid::new_v4().to_string();
    let mut start = Box::pin(runtime::start_with_provider(
        &f.state,
        runtime::StartInput {
            execution_enrolled: false,
            responses_billing: Some(resolved.token_billing),
            client_key: None,
            user: &f.owner,
            conversation: &f.row.id,
            client_request_id: &request,
            preferences,
            sdp: "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
        },
        resolved.identity,
        resolved.billing,
        openai::OpenAi::fixture(resolved.key, address)
            .with_delegation_model(resolved.delegation_model),
        None,
    ));
    // The current-thread runtime cannot run admission until this test yields.
    assert!(futures::poll!(start.as_mut()).is_pending());
    let closing_at;
    if window == Window::BeforeAdmission {
        closing_at = std::time::Instant::now();
        // Drop synchronously, before yielding to the spawned admission task.
        drop(start);
        tokio::time::timeout(Duration::from_secs(15), received.notified())
            .await
            .unwrap();
    } else {
        tokio::time::timeout(Duration::from_secs(15), received.notified())
            .await
            .unwrap();
        if window == Window::ProviderAttach || window == Window::Connected {
            release.notify_one();
            tokio::time::timeout(Duration::from_secs(15), attaching.notified())
                .await
                .unwrap();
        }
        closing_at = std::time::Instant::now();
        if window == Window::Connected {
            let response = start.await.unwrap();
            assert!(response.sdp.starts_with("v=0"));
            assert!(!response.session.end_requested);
        } else {
            drop(start);
        }
    }
    let call = request_row(&f.state.db, &request).await;
    if window == Window::ProviderAttach {
        assert_eq!(call.provider_session_id.as_deref(), Some("live_cancelled"));
    } else if window != Window::Connected {
        assert!(
            call.provider_session_id.is_none(),
            "provider create response is still gated"
        );
    }
    if client_end {
        session::control(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &call.id,
            &Uuid::new_v4().to_string(),
            0,
            "end",
        )
        .await
        .unwrap();
    }
    release.notify_one();
    attach_release.notify_one();
    let ended = released(&f.state.db, &call.id).await;
    assert!(closing_at.elapsed() < CLOSE_BOUND);
    assert_eq!(ended.provider_session_id.as_deref(), Some("live_cancelled"));
    assert!(ended.final_usage_confirmed && ended.billing_finalized);
    assert_eq!(ended.observed_seconds, 4);
    if !client_end && !invalid_answer {
        assert_eq!(ended.end_reason.as_deref(), Some("client_lost"));
        assert!(ended.end_requested);
    }
    assert_ne!(ended.end_reason.as_deref(), Some("close_unconfirmed"));
    assert_eq!(closes.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"action":"assistant_voice_close_unconfirmed"})
            .await
            .unwrap(),
        0
    );
    assert_settled(&f.state.db, &ended, paid).await;
    // Recovery/finalization replay cannot create another usage row or charge.
    crate::services::billing::voice::finalize(&f.state.db, &format!("voice:{}:0", ended.id), false)
        .await
        .unwrap();
    runtime::settle_windows(&f.state, &ended, false)
        .await
        .unwrap();
    assert_settled(&f.state.db, &ended, paid).await;
    server.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn dropped_during_provider_create_with_client_end_closes_and_settles() {
    cancelled_provider_start(Window::ProviderCreate, true, false, false).await;
}

#[tokio::test]
async fn dropped_before_admission_with_client_end_closes_and_settles() {
    cancelled_provider_start(Window::BeforeAdmission, true, false, false).await;
}

#[tokio::test]
async fn dropped_start_still_closes_provider_and_settles_an_invalid_answer() {
    cancelled_provider_start(Window::ProviderCreate, true, false, true).await;
}

#[tokio::test]
async fn dropped_before_admission_without_client_end_closes_and_settles() {
    cancelled_provider_start(Window::BeforeAdmission, false, false, false).await;
}

#[tokio::test]
async fn dropped_during_provider_create_without_client_end_closes_and_settles() {
    cancelled_provider_start(Window::ProviderCreate, false, false, false).await;
}

#[tokio::test]
async fn dropped_during_attach_without_client_end_closes_and_settles() {
    cancelled_provider_start(Window::ProviderAttach, false, false, false).await;
}

#[tokio::test]
async fn dropped_during_attach_with_client_end_closes_and_settles() {
    cancelled_provider_start(Window::ProviderAttach, true, false, false).await;
}

#[tokio::test]
async fn cancelled_platform_start_closes_and_charges_confirmed_usage_once() {
    for window in [
        Window::BeforeAdmission,
        Window::ProviderCreate,
        Window::ProviderAttach,
    ] {
        for client_end in [false, true] {
            cancelled_provider_start(window, client_end, true, false).await;
        }
    }
}

#[tokio::test]
async fn connected_platform_start_charges_the_same_confirmed_usage_once() {
    cancelled_provider_start(Window::Connected, true, true, false).await;
}

#[tokio::test]
async fn dropped_grok_preparation_records_protocol_and_releases_its_unstarted_slot() {
    let (state, thread, old) = tests::setup("voice_cancelled_grok_prepare").await;
    session::close(&state.db, &old, "fixture_setup", true)
        .await
        .unwrap();
    let request = Uuid::new_v4().to_string();
    let mut prepare = Box::pin(super::grok_runtime::prepare(
        &state,
        &old.user_id,
        &thread.id,
        &request,
        old.preferences.clone(),
        "identity".into(),
        false,
        None,
    ));
    assert!(futures::poll!(prepare.as_mut()).is_pending());
    drop(prepare);
    let call = request_row(&state.db, &request).await;
    // Admission and its protocol write are one detached operation; observe its completion.
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let call = session::get(&state.db, &old.user_id, &thread.id, &call.id)
                .await
                .unwrap();
            if call.protocol == Some(crate::models::downstream_service::VoiceProtocol::XaiRealtime)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let ended = released(&state.db, &call.id).await;
    assert!(ended.final_usage_confirmed && ended.billing_finalized);
    assert_eq!(ended.end_reason.as_deref(), Some("client_lost"));
    assert!(ended.provider_session_id.is_none());
    assert_ne!(ended.end_reason.as_deref(), Some("close_unconfirmed"));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn undelivered_start_end_survives_lease_transfer_and_preserves_existing_end() {
    let (state, _, call) = tests::setup("voice_undelivered_lease").await;
    let rows = state.db.collection::<VoiceSession>(COLLECTION_NAME);
    rows.update_one(
        doc! {"_id":&call.id},
        doc! {"$inc":{"generation":1},"$set":{"lease_owner":"older-replica"}},
    )
    .await
    .unwrap();
    let ended = session::end_undelivered_start(&state.db, &call)
        .await
        .unwrap()
        .unwrap();
    assert!(ended.end_requested);
    assert_eq!(ended.generation, call.generation + 1);
    assert_eq!(ended.control_revision, call.control_revision + 1);
    assert_eq!(ended.end_reason.as_deref(), Some("client_lost"));
    assert!(
        session::end_undelivered_start(&state.db, &call)
            .await
            .unwrap()
            .is_none()
    );
    rows.update_one(
        doc! {"_id":&call.id},
        doc! {"$set":{"end_reason":"user_ended"}},
    )
    .await
    .unwrap();
    assert!(
        session::end_undelivered_start(&state.db, &call)
            .await
            .unwrap()
            .is_none()
    );
    let unchanged = rows.find_one(doc! {"_id":&call.id}).await.unwrap().unwrap();
    assert_eq!(unchanged.end_reason.as_deref(), Some("user_ended"));
    assert_eq!(unchanged.control_revision, ended.control_revision);
    session::close(&state.db, &unchanged, "fixture_setup", true)
        .await
        .unwrap();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn detached_start_failure_is_a_499_disconnect_without_server_fault_audit() {
    use crate::errors::{AppError, voice_start::Stage};
    use std::{io::Write, sync::Mutex};
    use tracing::instrument::WithSubscriber;

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let (state, _, old) = tests::setup("voice_disconnected_diagnostics").await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || Capture(writer.clone()))
        .finish();
    let received = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let ready = received.clone();
    let gate = release.clone();
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    use tower_http::trace::TraceLayer;
    let handler_state = state.clone();
    let user = old.user_id.clone();
    let app = Router::new()
        .route(
            "/voice-sessions",
            post(move || {
                let state = handler_state.clone();
                let user = user.clone();
                let ready = ready.clone();
                let gate = gate.clone();
                async move {
                    let result =
                        runtime::deliver_start(state.clone(), Stage::Transport, async move {
                            ready.notify_one();
                            gate.notified().await;
                            Err(Stage::ProviderCreate.error(AppError::VoiceProviderUnavailable))
                        })
                        .await
                        .map(|_| StatusCode::NO_CONTENT);
                    super::diagnostics::finish(&state.db, &user, result, Stage::Transport).await
                }
            }),
        )
        .layer(TraceLayer::new_for_http().make_span_with(crate::mw::request_trace::span::<Body>));
    let request = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/voice-sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .with_subscriber(subscriber);
    let mut request = Box::pin(request);
    assert!(futures::poll!(request.as_mut()).is_pending());
    received.notified().await;
    drop(request);
    release.notify_one();
    let trace = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let trace = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
            if trace.contains("Voice startup failed after caller disconnected") {
                break trace;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(trace.contains("http_request"));
    assert!(trace.contains("status_code=499") && trace.contains("error_code=8012"));
    assert!(
        trace.contains("WARN")
            && trace.contains("status_code=503")
            && trace.contains("error_code=12501")
    );
    assert!(
        !trace.contains("finished processing request"),
        "no HTTP response was produced"
    );
    for forbidden in [
        "ERROR",
        "Voice start failed",
        "assistant_voice_start_failed",
    ] {
        assert!(
            !trace.contains(forbidden),
            "disconnected startup misclassified: {trace}"
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"action":"assistant_voice_start_failed"})
            .await
            .unwrap(),
        0
    );
    session::close(&state.db, &old, "fixture_setup", true)
        .await
        .unwrap();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_start_delivery_is_tracked_as_detached_until_acknowledged() {
    let (mut state, _, call) = tests::setup("voice_start_shutdown_tracking").await;
    state.shutdown = crate::services::shutdown::Runtime::default();
    let baseline = state.shutdown.counts().detached;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let ready = entered.clone();
    let gate = release.clone();
    let result = call.clone();
    let mut start = Box::pin(runtime::deliver_start(
        state.clone(),
        crate::errors::voice_start::Stage::Transport,
        async move {
            ready.notify_one();
            gate.notified().await;
            Ok(runtime::Started {
                session: result,
                sdp: "answer".into(),
            })
        },
    ));
    assert!(futures::poll!(start.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("start work entered the detached delivery task");
    assert_eq!(
        state.shutdown.counts().detached,
        baseline + 1,
        "the in-flight start owns exactly one detached shutdown work item"
    );

    release.notify_one();
    let started = tokio::time::timeout(Duration::from_secs(5), start)
        .await
        .expect("released start is delivered")
        .unwrap();
    assert_eq!(started.session.id, call.id);
    assert_eq!(started.sdp, "answer");
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.shutdown.counts().detached != baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("acknowledged start releases its detached shutdown work item");
    assert_eq!(state.shutdown.counts().detached, baseline);
    session::close(&state.db, &call, "fixture_setup", true)
        .await
        .unwrap();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn dropping_buffered_start_response_requests_end_without_client_control() {
    let (state, _, call) = tests::setup("voice_buffered_start").await;
    let completed = Arc::new(Notify::new());
    let ready = completed.clone();
    let result = call.clone();
    let mut start = Box::pin(runtime::deliver_start(
        state.clone(),
        crate::errors::voice_start::Stage::Transport,
        async move {
            ready.notify_one();
            Ok(runtime::Started {
                session: result,
                sdp: "answer".into(),
            })
        },
    ));
    assert!(futures::poll!(start.as_mut()).is_pending());
    // The task sends its result synchronously after notifying and then awaits
    // acknowledgment. Leave that buffered result unread before dropping it.
    completed.notified().await;
    drop(start);
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = state
                .db
                .collection::<VoiceSession>(COLLECTION_NAME)
                .find_one(doc! {"_id":&call.id})
                .await
                .unwrap()
                .unwrap();
            if row.end_requested {
                break row;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(ended.end_reason.as_deref(), Some("client_lost"));
    assert_eq!(ended.control_revision, call.control_revision + 1);
    session::close(&state.db, &ended, "fixture_setup", true)
        .await
        .unwrap();
    state.db.drop().await.unwrap();
}
