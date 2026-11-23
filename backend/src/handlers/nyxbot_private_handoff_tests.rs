use super::*;
use crate::{
    handlers::{
        assistant_handoff_test_upstream::{Cut, Upstream},
        assistant_nyxagent,
    },
    models::assistant_conversation::{ChatRestartSource, EngineWorkerInstance},
    services::{
        assistant_engine_worker as workers, assistant_turn_restart as restart,
        feature_flag_service as flags, security_readiness_service as readiness,
    },
};
use std::sync::atomic::Ordering;

async fn mock_chat(
    state: &AppState,
    channel: &NyxbotChannel,
    gateway: bool,
) -> wiremock::MockServer {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    if gateway {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = GATEWAY_SLUG.into();
        service.base_url = server.uri();
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(service)
            .await
            .unwrap();
        Mock::given(path("/events/cmaeg1.ev.sealed-reference/reply-target")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref":"cmaeg1.rt.original","expires_at":(Utc::now()+ChronoDuration::hours(23)).timestamp()}))).mount(&server).await;
        Mock::given(path("/reply-targets/cmaeg1.rt.original/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
            )
            .mount(&server)
            .await;
    } else {
        let encrypted = state.encryption_keys.encrypt(b"app:secret").await.unwrap();
        state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&channel.channel_bot_id},doc! {"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
        Mock::given(path("/open-apis/auth/v3/tenant_access_token/internal"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"code":0,"tenant_access_token":"test-token","expire":7200}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(path("/open-apis/im/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"test-reply"}})),
            )
            .mount(&server)
            .await;
        let adapter =
            crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                &server.uri(),
                "lark",
            );
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .insert(channel.channel_bot_id.clone(), Arc::new(adapter));
    }
    server
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    Gateway,
    Direct,
    Native,
}

async fn exercise(transport: Transport, cut: Cut, flag: bool, compatible: bool) {
    let gateway = transport != Transport::Direct;
    let upstream = Upstream::default();
    let db = connect_transaction_test_database("private_handoff").await;
    let (monitored, counts) = crate::services::durable_wake_tests::monitored(&db).await;
    let (first, _, server) = Box::pin(setup_with_engine(
        monitored,
        Value::Null,
        Some(upstream.router(cut, flag && compatible)),
    ))
    .await;
    let mut next = test_app_state(db.clone());
    next.encryption_keys = first.encryption_keys.clone();
    let (channel, key) = if transport == Transport::Native {
        Box::pin(native_tests::native_whatsapp_channel(&first)).await
    } else {
        Box::pin(channel(&first, if gateway { "gateway" } else { "direct" })).await
    };
    let chat = mock_chat(&first, &channel, gateway).await;
    let question = Uuid::new_v4().to_string();
    let sender = if gateway { "7" } else { "ou_alice" };
    db.collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&channel.id},
            doc! {"$set":{"owner_sender_ids":[sender]}},
        )
        .await
        .unwrap();
    if transport == Transport::Gateway {
        assert_eq!(
            put_binding(
                State(first.clone()),
                Path("bnd_chats".into()),
                bearer(&key),
                Json(binding_body(&key, OWNER))
            )
            .await
            .status(),
            StatusCode::OK
        );
        ensure_partition(&first, &key, PARTITION).await;
    }
    let home = engine::begin_turn(
        &db,
        OWNER,
        &engine::parse_turn(br#"{"text":"warm up"}"#).unwrap(),
        &first.encryption_keys,
    )
    .await
    .unwrap();
    engine::force_stop(
        &db,
        OWNER,
        &home.id,
        &home.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap();
    readiness::observe_test_handoff_readiness(&db, true).await;
    if !compatible {
        readiness::observe_v3_only_test_fleet(&db).await;
    }
    if flag {
        flags::set_platform_override(
            &db,
            flags::TURN_HANDOFF_FLAG_KEY,
            &flags::FlagTarget::User(OWNER.into()),
            true,
            OWNER,
        )
        .await
        .unwrap();
    }
    crate::services::assistant_notification_service::reset_flag_cache(&db);
    engine::require_enabled(&db, OWNER).await.unwrap();
    assistant_nyxagent::restart_metadata(
        &first,
        &crate::test_utils::test_auth_user(OWNER),
        Some(assistant_nyxagent::SERVER_TURN_POLICY),
    )
    .await;
    let initial = upstream.discovery.counts();
    let worker = Box::leak(Box::new(EngineWorkerInstance {
        id: Uuid::new_v4().to_string(),
        hostname: None,
    }));
    counts.lock().unwrap().clear();
    let response = workers::TEST_WORKER
        .scope(worker, async {
            if gateway {
                let request = if transport == Transport::Native {
                    native_tests::whatsapp_request(&channel, &question, sender, "answer this")
                } else {
                    event("answer this", sender, "private-question")
                };
                let response = respond(&first, &key, &request, &question).await;
                assert_eq!(response.status(), StatusCode::OK);
                Some(response)
            } else {
                assert!(
                    Box::pin(direct_private(
                        &first,
                        &channel,
                        &question,
                        sender,
                        "answer this"
                    ))
                    .await
                    .is_success()
                );
                None
            }
        })
        .await;
    let conversations = db.collection::<AssistantConversation>(CONVERSATIONS);
    if cut != Cut::None {
        let row = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if upstream.posts.load(Ordering::SeqCst) == 1
                    && let Some(row) = conversations
                        .find_one(doc! {"user_id":OWNER,"active_turn.origin":"channel"})
                        .await
                        .unwrap()
                    && (cut == Cut::BeforeCreated
                        || row.active_turn.as_ref().unwrap().running_response.is_some())
                {
                    break row;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("channel request reached upstream");
        let active = row.active_turn.as_ref().unwrap();
        let admission = active
            .restart_admission
            .as_ref()
            .expect("owner-private enrollment");
        assert_eq!(active.worker_instance.as_ref().unwrap().id, worker.id);
        assert!(
            matches!(admission.chat_source.as_deref(),Some(ChatRestartSource::PrivateChannel{event_id,origin}) if Some(event_id)==active.channel_event_id.as_ref() && Some(origin)==active.asked_from.as_ref())
        );
        if cut == Cut::BeforeCreated {
            first.shutdown.stop();
            first.shutdown.tasks_stopped().await;
            conversations
                .update_one(
                    doc! {"_id":&row.id,"active_turn.turn_id":&active.turn_id},
                    doc! {"$set":{"active_turn.heartbeat_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            assert!(
                engine::sweep_orphaned_turn_rows(&db)
                    .await
                    .unwrap()
                    .is_empty()
            );
        } else {
            assert!(
                restart::offer(&db, &row, restart::Loss::Shutdown(&worker.id))
                    .await
                    .unwrap()
            );
        }
        let offer = restart::due(&db, &workers::instance().id)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert!(
            assistant_nyxagent::survivor::take_one(&next, &offer, workers::instance())
                .await
                .unwrap()
        );
        first.shutdown.stop();
        first.shutdown.tasks_stopped().await;
        if cut == Cut::Stop {
            tokio::time::timeout(Duration::from_secs(25), async {
                while upstream.lookups.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .unwrap();
            assistant_nyxagent::stop(
                State(next.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Path(row.id.clone()),
            )
            .await
            .unwrap();
        }
        let native_response = response
            .as_ref()
            .and_then(|response| response.headers().get("x-cma-response-id"))
            .map(|value| value.to_str().unwrap().to_owned());
        drop(response);
        tokio::time::timeout(Duration::from_secs(35), next.shutdown.idle())
            .await
            .unwrap();
        let reply = db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&admission.reply_id})
            .await
            .unwrap()
            .unwrap();
        match cut {
            Cut::Lost => assert_eq!(reply.error_code.as_deref(), Some("outcome_unknown")),
            Cut::Stop => assert_eq!(reply.error_code.as_deref(), Some("cancelled")),
            _ => assert_eq!(reply.text, "answer @NyxBot"),
        }
        let event = active.channel_event_id.as_ref().unwrap();
        if transport == Transport::Native {
            let id = native_response.unwrap();
            let primary = db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"_id":event})
                .await
                .unwrap()
                .unwrap();
            let delivery = primary.delivery.unwrap();
            assert!(delivery.restart_owner_verified);
            assert!(!delivery.execution_owner_verified);
            let stored = crate::services::nyxbot_response_recovery::load(&db, &channel, &id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.event_key, *event);
            assert_eq!(
                active.asked_from.as_ref().unwrap().native_event_id.as_ref(),
                Some(event)
            );
            for _ in 0..2 {
                let replay = native_responses::events(
                    State(next.clone()),
                    Path(id.clone()),
                    axum::extract::Query(native_responses::Cursor {
                        starting_after: None,
                    }),
                    bearer(&key),
                )
                .await;
                assert_eq!(replay.status(), StatusCode::OK);
                let wire = body_text(replay).await;
                assert!(wire.contains("answer @NyxBot"), "{wire}");
            }
        } else {
            crate::services::channel_turn_delivery::detach(&db, event)
                .await
                .unwrap();
            Box::pin(late_delivery::process(&next, event)).await;
            Box::pin(late_delivery::process(&next, event)).await;
            if matches!(cut, Cut::MidAnswer | Cut::BeforeCreated) {
                let sent: Vec<_> = chat
                    .received_requests()
                    .await
                    .unwrap()
                    .into_iter()
                    .filter(|request| request.url.path().ends_with("/messages"))
                    .collect();
                assert_eq!(sent.len(), 1, "gateway={gateway} cut={cut:?}");
                assert!(String::from_utf8_lossy(&sent[0].body).contains("answer @NyxBot"));
            }
        }
    } else {
        if let Some(response) = response {
            assert!(body_text(response).await.contains("answer @NyxBot"));
        }
        tokio::time::timeout(Duration::from_secs(35), async {
            while upstream.posts.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            first.shutdown.idle().await;
        })
        .await
        .unwrap();
        let commands = crate::services::durable_wake_tests::take(&counts);
        eprintln!(
            "private chat gateway={gateway} enabled={flag} ready={compatible} commands={} counts={}",
            commands.values().sum::<usize>(),
            serde_json::to_string(&commands).unwrap()
        );
    }
    assert_eq!(upstream.posts.load(Ordering::SeqCst), 1);
    assert_eq!(upstream.discovery.counts(), initial);
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&channel.channel_bot_id);
    server.abort();
    let _ = server.await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn private_chat_handoff_four_cuts() {
    for transport in [Transport::Gateway, Transport::Direct] {
        for cut in [Cut::MidAnswer, Cut::BeforeCreated, Cut::Stop, Cut::Lost] {
            Box::pin(exercise(transport, cut, true, true)).await;
        }
    }
}

#[tokio::test]
async fn private_chat_handoff_native_event_replay() {
    Box::pin(exercise(Transport::Native, Cut::MidAnswer, true, true)).await;
}

#[tokio::test]
async fn private_chat_handoff_normal_output_and_commands() {
    for (enabled, ready) in [(false, true), (true, false), (true, true)] {
        Box::pin(exercise(Transport::Gateway, Cut::None, enabled, ready)).await;
    }
}

#[tokio::test]
async fn private_chat_handoff_manager_ingress() {
    for cut in [Cut::BeforeCreated, Cut::MidAnswer, Cut::Stop, Cut::Lost] {
        Box::pin(manager_handoff(cut)).await;
    }
}

async fn manager_handoff(cut: Cut) {
    use crate::services::{
        platform_manager_contract as contract, platform_personal_turn_service as personal,
    };
    let (fixture, admitted) = personal::tests::fixture("manager_handoff").await;
    let db = fixture.db.clone();
    let owner = admitted.authority.user_id.as_str();
    let upstream = Upstream::default();
    let (first, _, server) = Box::pin(setup_with_engine(
        db.clone(),
        Value::Null,
        Some(upstream.router(cut, true)),
    ))
    .await;
    let mut next = test_app_state(db.clone());
    next.encryption_keys = first.encryption_keys.clone();
    readiness::observe_test_handoff_readiness(&db, true).await;
    flags::set_platform_override(
        &db,
        flags::TURN_HANDOFF_FLAG_KEY,
        &flags::FlagTarget::User(owner.into()),
        true,
        owner,
    )
    .await
    .unwrap();
    let identity = crate::services::platform_personal_binding_service::producer(first.clone());
    let service = personal::producer(
        first.clone(),
        identity.clone(),
        Arc::new(crate::services::platform_manager_ingress_service::IngressTurnStore),
        crate::services::assistant_nyxagent_egress::producer(
            first.clone(),
            identity,
            crate::handlers::agent_skills::preload_context(first.clone()),
        ),
        crate::handlers::assistant_team::owner_turn_settled_hook(first.clone()),
    );
    let worker = Box::leak(Box::new(EngineWorkerInstance {
        id: Uuid::new_v4().to_string(),
        hostname: None,
    }));
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    let start = workers::TEST_WORKER
        .scope(
            worker,
            service.start_personal_turn(
                &db,
                &admitted,
                contract::TurnInput {
                    text: "please answer".to_owned(),
                    channel_uploads: vec![],
                },
                sender,
            ),
        )
        .await
        .unwrap();
    let contract::PersonalTurnStart::Started { turn, .. } = start else {
        panic!("expected manager turn")
    };
    let row = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let row = engine::get(&db, owner, &turn.conversation_id)
                .await
                .unwrap();
            let active = row.active_turn.as_ref().expect("manager turn still active");
            if upstream.posts.load(Ordering::SeqCst) == 1
                && (cut == Cut::BeforeCreated || active.running_response.is_some())
            {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let active = row.active_turn.as_ref().unwrap();
    let admission = active.restart_admission.as_ref().unwrap();
    assert!(
        matches!(admission.chat_source.as_deref(),Some(ChatRestartSource::PersonalManager {origin}) if Some(origin.as_ref()) == active.platform_origin.as_ref())
    );
    assert!(
        restart::offer(&db, &row, restart::Loss::Shutdown(&worker.id))
            .await
            .unwrap()
    );
    let offer = restart::due(&db, &workers::instance().id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        assistant_nyxagent::survivor::take_one(&next, &offer, workers::instance())
            .await
            .unwrap()
    );
    first.shutdown.stop();
    first.shutdown.tasks_stopped().await;
    if cut == Cut::Stop {
        tokio::time::timeout(Duration::from_secs(15), async {
            while upstream.lookups.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap();
        assistant_nyxagent::stop(
            State(next.clone()),
            crate::test_utils::test_auth_user(owner),
            Path(row.id.clone()),
        )
        .await
        .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(25), next.shutdown.idle())
        .await
        .unwrap();
    let reply = db
        .collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&admission.reply_id})
        .await
        .unwrap()
        .unwrap();
    match cut {
        Cut::Lost => assert_eq!(reply.error_code.as_deref(), Some("outcome_unknown")),
        Cut::Stop => assert_eq!(reply.error_code.as_deref(), Some("cancelled")),
        _ => assert_eq!(reply.text, "answer @NyxBot"),
    }
    assert_eq!(reply.platform_origin, active.platform_origin);
    assert_eq!(upstream.posts.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
    db.drop().await.unwrap();
}
