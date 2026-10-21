//! Actual conversion, retained native command and final billed X delivery.
//! The owned TCP provider distinguishes no headers from a stalled 201 body.
use super::*;
use crate::{
    models::{
        billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
        credits::Credits,
        platform_manager_delivery::{
            COLLECTION_NAME as DELIVERIES, DeliveryState, PlatformManagerDelivery,
        },
        usage_meter::{COLLECTION_NAME as METERS, UsageMeterRow, UsageStatus},
    },
    services::{
        platform_manager_command_service::PersonalCommandKind,
        platform_manager_contract::DeliveryOutcome,
        platform_manager_delivery as delivery,
        platform_native_reply_service::{NativeTestTransport, with_transport_for_test},
        platform_personal_binding_service::IdentityProducer,
    },
};
use futures::TryStreamExt;
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
enum ProviderMode {
    Success,
    Unfunded,
    HeadersStall,
    BodyStall,
}
struct LocalXProvider {
    base: String,
    accepted: Arc<AtomicUsize>,
    entered: Arc<AtomicBool>,
    headers_written: Arc<AtomicBool>,
    requests: Arc<tokio::sync::Mutex<Vec<Value>>>,
    stop: tokio::sync::oneshot::Sender<()>,
    worker: tokio::task::JoinHandle<()>,
}
impl LocalXProvider {
    async fn new(mode: ProviderMode) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let accepted = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(AtomicBool::new(false));
        let headers_written = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let worker_accepted = accepted.clone();
        let worker_entered = entered.clone();
        let worker_headers = headers_written.clone();
        let worker_requests = requests.clone();
        let worker = tokio::spawn(async move {
            let mut stopped = stopped;
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    joined = connections.join_next(), if !connections.is_empty() => { joined.unwrap().unwrap(); },
                    socket = listener.accept() => {
                        let (mut stream, _) = socket.unwrap();
                        worker_accepted.fetch_add(1, Ordering::SeqCst);
                        let entered = worker_entered.clone();
                        let headers_written = worker_headers.clone();
                        let requests = worker_requests.clone();
                        connections.spawn(async move {
                            let mut bytes = Zeroizing::new(Vec::new());
                            let (header_end, length) = loop {
                                let mut chunk = [0u8; 4096];
                                let count = stream.read(&mut chunk).await.unwrap();
                                assert_ne!(count, 0);
                                bytes.extend_from_slice(&chunk[..count]);
                                assert!(bytes.len() <= 32_768);
                                if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                                    let headers = std::str::from_utf8(&bytes[..index]).unwrap();
                                    assert!(headers.starts_with("POST /2/dm_conversations/2-10/messages HTTP/1.1\r\n"));
                                    assert!(headers.lines().any(|line| line.to_ascii_lowercase().starts_with("authorization: bearer ")));
                                    let length = headers.lines().find_map(|line| line.split_once(':')
                                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())).unwrap();
                                    break (index + 4, length);
                                }
                            };
                            while bytes.len() < header_end + length {
                                let mut chunk = [0u8; 4096];
                                let count = stream.read(&mut chunk).await.unwrap();
                                assert_ne!(count, 0);
                                bytes.extend_from_slice(&chunk[..count]);
                                assert!(bytes.len() <= 32_768);
                            }
                            let body = serde_json::from_slice(&bytes[header_end..header_end+length]).unwrap();
                            requests.lock().await.push(body);
                            entered.store(true, Ordering::SeqCst);
                            match mode {
                                ProviderMode::Success => {
                                    let body = r#"{"data":{"dm_event_id":"990"}}"#;
                                    stream.write_all(format!("HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                                    headers_written.store(true, Ordering::SeqCst);
                                }
                                ProviderMode::BodyStall => {
                                    stream.write_all(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();
                                    stream.flush().await.unwrap();
                                    headers_written.store(true, Ordering::SeqCst);
                                    std::future::pending::<()>().await;
                                }
                                ProviderMode::HeadersStall => std::future::pending::<()>().await,
                                ProviderMode::Unfunded => panic!("unfunded native delivery reached provider"),
                            }
                        });
                    }
                }
            }
            connections.abort_all();
            while let Some(joined) = connections.join_next().await {
                if let Err(error) = joined {
                    assert!(error.is_cancelled());
                }
            }
        });
        Self {
            base,
            accepted,
            entered,
            headers_written,
            requests,
            stop,
            worker,
        }
    }
    async fn finish(self) -> (usize, bool, bool, Vec<Value>) {
        let _ = self.stop.send(());
        tokio::time::timeout(Duration::from_secs(2), self.worker)
            .await
            .unwrap()
            .unwrap();
        let requests = self.requests.lock().await.clone();
        (
            self.accepted.load(Ordering::SeqCst),
            self.entered.load(Ordering::SeqCst),
            self.headers_written.load(Ordering::SeqCst),
            requests,
        )
    }
}

#[tokio::test]
async fn actual_paid_native_x_final_delivery_fences_funding_and_preserves_header_body_uncertainty_without_resend()
 {
    for mode in [
        ProviderMode::Success,
        ProviderMode::Unfunded,
        ProviderMode::HeadersStall,
        ProviderMode::BodyStall,
    ] {
        let fixture = Fixture::new().await;
        let NativeConversionOutcome::Converted(profile) = fixture
            .convert(NativeTransportChoice::Direct)
            .await
            .unwrap()
        else {
            panic!("direct conversion")
        };
        let controls = enable_personal(&fixture, &profile).await;
        let admission = x_admission(
            &fixture,
            &profile,
            controls.manager_authority_epoch,
            "help",
            "810001",
        )
        .await;
        let command = execute_command(&fixture.state, &admission, PersonalCommandKind::Help)
            .await
            .unwrap()
            .unwrap();
        let ingress = command.ingress.clone();
        let expected = command
            .reply
            .render(&fixture.state.config.frontend_url, &command.manager.entry)
            .unwrap();
        let baseline = crate::services::channel_x_tests::billing::settled(&fixture.state).await;
        assert_eq!(
            baseline.len(),
            1,
            "actual conversion has one billed account verification"
        );
        if matches!(mode, ProviderMode::Unfunded) {
            fixture
                .state
                .db
                .collection::<Document>(WALLETS)
                .update_one(
                    doc! {"owner_id":&fixture.bot.user_id},
                    doc! {"$set":{"balance_credits":Credits::ZERO.decimal()}},
                )
                .await
                .unwrap();
        }
        let provider = LocalXProvider::new(mode).await;
        let transport = Arc::new(NativeTestTransport {
            adapter: Arc::new(crate::services::channel_adapters::x::XAdapter {
                api_base: Some(provider.base.clone()),
            }),
            discord_base: provider.base.clone(),
        });
        let api = crate::services::telegram_new_api::TelegramApi {
            http: &fixture.state.http_client,
            base_url: &provider.base,
        };
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(8),
            with_transport_for_test(
                transport.clone(),
                delivery::TEST_NATIVE_SEND_BUDGET.scope(
                    Duration::from_secs(3),
                    delivery::deliver_command(
                        &fixture.state,
                        Arc::new(IdentityProducer),
                        command,
                        &api,
                    ),
                ),
            ),
        )
        .await;
        let elapsed = started.elapsed();
        let stored = fixture
            .state
            .db
            .collection::<PlatformManagerDelivery>(DELIVERIES)
            .find_one(doc! {"ingress_id":&ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        let producers = crate::services::platform_manager_transport::compose(
            fixture.state.clone(),
            Arc::new(Settled),
            crate::handlers::agent_skills::preload_context(fixture.state.clone()),
        );
        fixture.state.db.collection::<PlatformManagerDelivery>(DELIVERIES).update_one(doc! {"_id":&stored.id},
            doc! {"$set":{"updated_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::minutes(3))}}).await.unwrap();
        with_transport_for_test(
            transport,
            delivery::sweep(
                &fixture.state,
                producers.identity.clone(),
                producers.public.clone(),
                10,
            ),
        )
        .await
        .unwrap();
        let replay = execute_command(&fixture.state, &admission, PersonalCommandKind::Help)
            .await
            .unwrap();
        let recovered = fixture
            .state
            .db
            .collection::<PlatformManagerDelivery>(DELIVERIES)
            .find_one(doc! {"_id":&stored.id})
            .await
            .unwrap()
            .unwrap();
        let (accepted, entered, headers, requests) = provider.finish().await;
        let outcome = result
            .expect("native paid send has one absolute bounded lifetime")
            .unwrap();
        assert!(elapsed < Duration::from_secs(6));
        assert!(replay.is_none());
        assert!(if matches!(mode, ProviderMode::Success) {
            matches!(outcome, DeliveryOutcome::NativeSent { .. })
        } else if matches!(mode, ProviderMode::Unfunded) {
            matches!(outcome, DeliveryOutcome::Refused(_))
        } else {
            matches!(outcome, DeliveryOutcome::Uncertain)
        });
        let no_attempt = matches!(mode, ProviderMode::Unfunded);
        assert_eq!(
            accepted,
            usize::from(!no_attempt),
            "accept loop stays live during either response stall"
        );
        assert_eq!(entered, !no_attempt);
        assert_eq!(
            requests.len(),
            usize::from(!no_attempt),
            "sweep and duplicate command never resend"
        );
        assert_eq!(
            headers,
            matches!(mode, ProviderMode::Success | ProviderMode::BodyStall)
        );
        if !no_attempt {
            assert_eq!(requests[0]["text"], expected.as_str());
        }
        assert_eq!(
            fixture.server.received_requests().await.unwrap().len(),
            1,
            "setup egress remains account verification only"
        );
        assert_eq!(
            recovered.state,
            if matches!(mode, ProviderMode::Success) {
                DeliveryState::Sent
            } else if no_attempt {
                DeliveryState::Refused
            } else {
                DeliveryState::Uncertain
            }
        );
        let rows: Vec<UsageMeterRow> = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let rows: Vec<UsageMeterRow> = fixture
                    .state
                    .db
                    .collection::<UsageMeterRow>(METERS)
                    .find(doc! {})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                if no_attempt
                    || matches!(mode, ProviderMode::HeadersStall)
                    || rows.iter().all(|row| row.released)
                {
                    break rows;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(rows.len(), if no_attempt { 1 } else { 2 });
        let verification = rows
            .iter()
            .filter(|row| row.operation.as_deref() == Some("get_me"))
            .collect::<Vec<_>>();
        assert_eq!(verification.len(), 1);
        assert_eq!(verification[0].id, baseline[0].id);
        assert_eq!(
            verification[0].billing_request_id,
            baseline[0].billing_request_id
        );
        assert_eq!(verification[0].status, baseline[0].status);
        assert_eq!(verification[0].quantity, baseline[0].quantity);
        assert_eq!(
            verification[0].billing_owner_id,
            baseline[0].billing_owner_id
        );
        let sends = rows
            .iter()
            .filter(|row| row.operation.as_deref() == Some("channel_dm_send"))
            .collect::<Vec<_>>();
        assert_eq!(
            sends.len(),
            usize::from(!no_attempt),
            "each attempted send has exactly one attributed meter"
        );
        if !no_attempt {
            let send = sends[0];
            assert_eq!(send.released, !matches!(mode, ProviderMode::HeadersStall));
            assert_eq!(send.billing_owner_id, fixture.bot.user_id);
            assert_eq!(send.actor_user_id, fixture.bot.user_id);
            assert_eq!(
                send.status,
                if matches!(mode, ProviderMode::HeadersStall) {
                    UsageStatus::Forwarded
                } else {
                    UsageStatus::Finalized
                }
            );
            assert_eq!(
                send.quantity,
                if matches!(mode, ProviderMode::HeadersStall) {
                    None
                } else {
                    Some(1)
                }
            );
        }
        let wallet: BillingWallet = fixture
            .state
            .db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"owner_id":&fixture.bot.user_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            wallet.reserved_credits,
            if matches!(mode, ProviderMode::HeadersStall) {
                Credits::from_whole(2)
            } else {
                Credits::ZERO
            }
        );
        assert_eq!(
            wallet.pending_lago_debits,
            Credits::from_whole(
                if matches!(mode, ProviderMode::Success | ProviderMode::BodyStall) {
                    4
                } else {
                    2
                }
            )
        );
        fixture.state.db.drop().await.unwrap();
    }
}
