//! Real relay/provider handlers and MongoDB; CMA's tool marker is an external boundary.
use super::*;
use crate::handlers::{channel_relay, nyxbot::supplemental::Purpose};
use crate::models::assistant_conversation::AssistantConversation;
use crate::services::channel_supplemental_delivery as sources;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn notice_fixture() -> (Fixture, MockServer, AssistantConversation) {
    let (f, platform) = Box::pin(image_fixture("telegram", "gateway")).await;
    let origin = f.event.delivery.as_ref().unwrap().origin.clone();
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&f.event.conversation_id},doc! {"$set":{"message_count":1,"active_turn":{
        "turn_id":&f.event.turn_id,"origin":"channel","asked_from":bson::to_bson(&origin).unwrap(),
        "channel_event_id":&f.event.id,"started_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(10)),
        "lease_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::minutes(5)),
        "external_check_started":true,"stop_requested":false,
    }}}).await.unwrap();
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(b"cmaeg1.ev.original")
        .await
        .unwrap();
    let mut fields = sources::reference_fields(
        encrypted,
        Utc::now() + ChronoDuration::minutes(29),
        f.event.relay_message_id.as_deref(),
    );
    fields.insert("platform_thread_id", "77");
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.event.relay_message_id},
            doc! {"$set":{"thread_id":"77"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(doc! {"channel_id":&f.channel.id}, doc! {"$set":fields})
        .await
        .unwrap();
    let row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    (f, platform, row)
}

async fn mock_platform(server: &MockServer, notice_outcome: &str) {
    let ids = Arc::new(AtomicUsize::new(100));
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(
                json!({"ok":true,"result":{"message_id":ids.fetch_add(1,Ordering::SeqCst)}}),
            )
        })
        .mount(server)
        .await;
    let response = match notice_outcome {
        "rejected" => ResponseTemplate::new(400).set_body_json(
            json!({"ok":false,"error_code":400,"description":"Bad Request: message refused"}),
        ),
        "unknown" => ResponseTemplate::new(200)
            .set_delay(Duration::from_secs(21))
            .set_body_json(json!({"ok":true,"result":{"message_id":99}})),
        _ => {
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":99}}))
        }
    };
    Mock::given(path("/bottoken/sendMessage"))
        .and(body_string_contains(
            crate::services::assistant_checking_notice::TEXT,
        ))
        .respond_with(response)
        .with_priority(1)
        .mount(server)
        .await;
}

struct GatewayContract {
    marker: Arc<AtomicUsize>,
}
impl GatewayContract {
    async fn mount(f: &Fixture, unknown: bool) -> Self {
        let marker = Arc::new(AtomicUsize::new(0));
        let shared = marker.clone();
        Mock::given(path("/events/cmaeg1.ev.original/replies"))
            .respond_with(move |_: &wiremock::Request| {
                shared.store(if unknown { 2 } else { 1 }, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(
                    json!({"delivery":{"outcome":if unknown {"unknown"} else {"confirmed"}}}),
                )
            })
            .mount(&f.gateway)
            .await;
        Self { marker }
    }
    async fn final_reply(&self, f: &Fixture, text: &str) -> bool {
        // CMA delivery/step and routed replay both suppress confirmed tools;
        // routed unknown settles without the automatic answer as well.
        if self.marker.load(Ordering::SeqCst) != 0 {
            return false;
        }
        let token = crate::crypto::jwt::generate_relay_reply_token(
            &f.state.jwt_keys,
            &f.state.config,
            &f.channel.route_api_key_id,
            f.channel.route_id.as_deref().unwrap(),
            f.event.relay_message_id.as_deref().unwrap(),
            "telegram",
        )
        .unwrap();
        let _ = channel_relay::async_reply(
            State(f.state.clone()),
            bearer(&token),
            crate::mw::auth::OptionalAuthUser(None),
            Json(channel_relay::AsyncReplyRequest {
                message_id: f.event.relay_message_id.clone().unwrap(),
                thread_reply: false,
                reply: channel_relay::AsyncReplyBody {
                    text: Some(text.into()),
                    metadata: None,
                    attachments: Vec::new(),
                },
            }),
        )
        .await
        .unwrap();
        true
    }
}

async fn audit_decisions(f: &Fixture) -> Vec<bson::Document> {
    f.state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":"nyxbot_channel_supplemental_delivery"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn notice(f: &Fixture, row: &AssistantConversation) {
    assert!(
        crate::handlers::assistant_nyxagent::send_checking_notice(
            &f.state,
            row,
            &f.event.delivery.as_ref().unwrap().origin
        )
        .await
    );
}
async fn live(f: &Fixture, failed: bool) -> String {
    let (sender, receiver) = broadcast::channel(8);
    if !failed {
        sender
            .send(json!({"event":"block.completed","block":{"text":"Useful answer"}}))
            .unwrap();
    }
    sender
        .send(json!({"event":"turn.completed","status":if failed {"failed"} else {"completed"}}))
        .unwrap();
    TEST_MESSAGE_ITEM_ID
        .scope(
            "msg_test_baseline".into(),
            body_text(late_delivery::provider_stream(
                f.state.clone(),
                f.event.id.clone(),
                "response".into(),
                json!({"type":"response.created"}),
                receiver,
                Utc::now(),
            )),
        )
        .await
}
async fn no_event_replies(f: &Fixture) {
    assert!(
        f.gateway
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.url.path().ends_with("/replies"))
    );
}

#[tokio::test]
async fn supplemental_checking_live_outcomes_preserve_stream_and_final() {
    Box::pin(async {
        for outcome in ["confirmed", "rejected", "unknown"] {
            let (f, platform, row) = Box::pin(notice_fixture()).await;
            let gateway = GatewayContract::mount(&f, outcome == "unknown").await;
            mock_platform(&platform, outcome).await;
            let baseline = live(&f, false).await;
            patch(&f, doc! {"delivery.state":"waiting"}).await;
            let transcript_before: Vec<bson::Document> = f
                .state
                .db
                .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
                .find(doc! {})
                .sort(doc! {"_id":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            notice(&f, &row).await;
            let transcript_after: Vec<bson::Document> = f
                .state
                .db
                .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
                .find(doc! {})
                .sort(doc! {"_id":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(transcript_before, transcript_after);
            assert_eq!(status(&f).await, "waiting");
            assert_eq!(
                f.state
                    .db
                    .collection::<bson::Document>("reply_token_uses")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            let with_notice = live(&f, false).await;
            assert_eq!(with_notice, baseline);
            assert!(with_notice.contains("Useful answer"));
            assert!(gateway.final_reply(&f, "Useful answer").await);
            no_event_replies(&f).await;
            late_delivery::process(&f.state, &f.event.id).await;
            assert_eq!(
                platform
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .filter(|r| r.url.path().ends_with("sendMessage"))
                    .count(),
                2
            );
            let audits = audit_decisions(&f).await;
            assert_eq!(audits.len(), 1, "{audits:?}");
            let metadata = audits[0].get_document("event_data").unwrap();
            assert_eq!(metadata.get_str("egress").unwrap(), "local_relay");
            assert_eq!(metadata.get_str("anchor").unwrap(), "event_source");
            assert_eq!(metadata.get_str("outcome").unwrap(), outcome, "{audits:?}");
            f.state.db.drop().await.unwrap();
        }
    })
    .await
}

#[tokio::test]
async fn supplemental_gateway_mock_confirmed_and_unknown_suppress_auto_final() {
    for unknown in [false, true] {
        let (f, platform, _) = Box::pin(notice_fixture()).await;
        let gateway = GatewayContract::mount(&f, unknown).await;
        f.state
            .http_client
            .post(format!(
                "{}/events/cmaeg1.ev.original/replies",
                f.gateway.uri()
            ))
            .json(&json!({"text":"notice"}))
            .send()
            .await
            .unwrap();
        assert!(!gateway.final_reply(&f, "final").await);
        assert!(platform.received_requests().await.unwrap().is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn supplemental_notice_failure_capacity_and_two_replicas() {
    Box::pin(async {
        for capacity in [false,true] {
            let (f,platform,row)=Box::pin(notice_fixture()).await;
            mock_platform(&platform,"confirmed").await;
            let gateway=GatewayContract::mount(&f,false).await;
            if capacity {
                f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc!{"_id":&row.id},doc!{"$set":{"active_turn.capacity_wait_note_origin":"nyxid_system","active_turn.engine_request_started_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(30))}}).await.unwrap();
                crate::handlers::assistant_nyxagent::capacity_notice(&f.state,&row).await;
            } else {
                let other=f.state.clone();
                let origin=&f.event.delivery.as_ref().unwrap().origin;
                let (a,b)=tokio::join!(crate::handlers::assistant_nyxagent::send_checking_notice(&f.state,&row,origin),crate::handlers::assistant_nyxagent::send_checking_notice(&other,&row,origin));
                assert_ne!(a,b);
            }
            let wire=live(&f,!capacity).await;
            if capacity {assert!(wire.contains("Useful answer"));} else {assert!(wire.contains("response.failed"));}
            assert!(gateway.final_reply(&f,if capacity {"Useful answer"} else {"NyxBot could not finish this turn."}).await);
            assert_eq!(platform.received_requests().await.unwrap().len(),2);
            no_event_replies(&f).await;
            f.state.db.drop().await.unwrap();
        }
    }).await
}

#[tokio::test]
async fn supplemental_chat_reference_anchor_missing_deleted_mismatched_stale_and_expired() {
    Box::pin(async {
        for case in [
            "paired",
            "pointer_deleted",
            "absent",
            "deleted",
            "mismatch",
            "stale_ciphertext",
            "stale_expiry",
            "expired",
            "pointer_mismatch",
        ] {
            let (f, platform, mut row) = Box::pin(notice_fixture()).await;
            mock_platform(&platform, "confirmed").await;
            row.active_turn = None;
            let mut fields = doc! {"last_relay_message_id":bson::Bson::Null};
            match case {
                "pointer_deleted" => {
                    fields.insert("last_relay_message_id", "deleted-pointer");
                }
                "absent" => {
                    fields.insert("event_ref_relay_message_id", bson::Bson::Null);
                }
                "deleted" => {
                    f.state
                        .db
                        .collection::<bson::Document>(
                            crate::models::channel_message::COLLECTION_NAME,
                        )
                        .delete_one(doc! {"_id":&f.event.relay_message_id})
                        .await
                        .unwrap();
                }
                "mismatch" => {
                    f.state
                        .db
                        .collection::<bson::Document>(
                            crate::models::channel_message::COLLECTION_NAME,
                        )
                        .update_one(
                            doc! {"_id":&f.event.relay_message_id},
                            doc! {"$set":{"platform_conversation_id":"other"}},
                        )
                        .await
                        .unwrap();
                }
                "stale_ciphertext" => {
                    fields.insert(
                        "event_ref_ciphertext",
                        bson::Binary {
                            subtype: bson::spec::BinarySubtype::Generic,
                            bytes: f.state.encryption_keys.encrypt(b"newer").await.unwrap(),
                        },
                    );
                }
                "stale_expiry" => {
                    fields.insert(
                        "event_ref_expires_at",
                        bson::DateTime::from_chrono(Utc::now() + ChronoDuration::minutes(28)),
                    );
                }
                "expired" => {
                    fields.insert(
                        "event_ref_expires_at",
                        bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(1)),
                    );
                }
                "pointer_mismatch" => {
                    let mut invalid = f
                        .state
                        .db
                        .collection::<bson::Document>(
                            crate::models::channel_message::COLLECTION_NAME,
                        )
                        .find_one(doc! {"_id":&f.event.relay_message_id})
                        .await
                        .unwrap()
                        .unwrap();
                    invalid.insert("_id", "wrong-pointer");
                    invalid.insert("user_id", "other-owner");
                    f.state
                        .db
                        .collection::<bson::Document>(
                            crate::models::channel_message::COLLECTION_NAME,
                        )
                        .insert_one(invalid)
                        .await
                        .unwrap();
                    fields.insert("last_relay_message_id", "wrong-pointer");
                }
                _ => {}
            }
            f.state
                .db
                .collection::<bson::Document>(THREADS)
                .update_one(doc! {"channel_id":&f.channel.id}, doc! {"$set":fields})
                .await
                .unwrap();
            deliver_to(
                &f.state,
                &row,
                &f.event.delivery.as_ref().unwrap().origin,
                "team result",
                None,
            )
            .await;
            let requests = platform.received_requests().await.unwrap();
            let delivered = matches!(case, "paired" | "pointer_deleted");
            assert_eq!(requests.len(), usize::from(delivered), "{case}");
            no_event_replies(&f).await;
            let audits = audit_decisions(&f).await;
            assert_eq!(audits.len(), 1);
            let data = audits[0].get_document("event_data").unwrap();
            if delivered {
                let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
                assert_eq!(body["message_thread_id"], 77);
                assert_eq!(body["reply_to_message_id"], 17);
                assert_eq!(data.get_str("anchor").unwrap(), "chat_reference_event");
            } else {
                assert_eq!(data.get_str("egress").unwrap(), "skipped");
                assert!(data.get_str("reason").is_ok());
            }
            f.state.db.drop().await.unwrap();
        }
    })
    .await
}

#[tokio::test]
async fn supplemental_progress_does_not_use_reference_fallback_or_newer_pointer() {
    Box::pin(async {
        let (mut f, platform, row) = Box::pin(notice_fixture()).await;
        mock_platform(&platform, "confirmed").await;
        let gateway = GatewayContract::mount(&f, false).await;
        // B becomes the current chat pointer and reference, while A remains admitted.
        let original = f.event.relay_message_id.as_ref().unwrap();
        let mut newer = f
            .state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .find_one(doc! {"_id":original})
            .await
            .unwrap()
            .unwrap();
        newer.insert("_id", "newer-inbound");
        newer.insert("platform_message_id", "18");
        f.state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(newer)
            .await
            .unwrap();
        let mut fields = sources::reference_fields(
            f.state
                .encryption_keys
                .encrypt(b"newer-event")
                .await
                .unwrap(),
            Utc::now() + ChronoDuration::minutes(29),
            Some("newer-inbound"),
        );
        fields.insert("last_relay_message_id", "newer-inbound");
        f.state
            .db
            .collection::<bson::Document>(THREADS)
            .update_one(doc! {"channel_id":&f.channel.id}, doc! {"$set":fields})
            .await
            .unwrap();
        notice(&f, &row).await;
        let requests = platform.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["reply_to_message_id"], 17);
        deliver_to(
            &f.state,
            &row,
            &f.event.delivery.as_ref().unwrap().origin,
            "A team result",
            None,
        )
        .await;
        let team: Value =
            serde_json::from_slice(&platform.received_requests().await.unwrap()[1].body).unwrap();
        assert_eq!(team["reply_to_message_id"], 17);
        patch(&f, doc! {"relay_message_id":bson::Bson::Null}).await;
        deliver_to_for(
            &f.state,
            &row,
            &f.event.delivery.as_ref().unwrap().origin,
            "machine notice",
            None,
            Purpose::MachineDesktopNotice,
        )
        .await;
        assert_eq!(platform.received_requests().await.unwrap().len(), 2);
        // B's final still goes through its original single-use callback.
        f.event.relay_message_id = Some("newer-inbound".into());
        assert!(gateway.final_reply(&f, "B answer").await);
        let final_body: Value =
            serde_json::from_slice(&platform.received_requests().await.unwrap()[2].body).unwrap();
        assert_eq!(final_body["reply_to_message_id"], 18);
        no_event_replies(&f).await;
        f.state.db.drop().await.unwrap();
    })
    .await
}

#[tokio::test]
async fn supplemental_reference_pairing_clears_nonrelay_and_debug_is_redacted() {
    let (f, _, _) = Box::pin(notice_fixture()).await;
    let fields = sources::reference_fields(
        vec![1, 2, 3],
        Utc::now() + ChronoDuration::minutes(29),
        None,
    );
    assert_eq!(
        fields.get("event_ref_relay_message_id"),
        Some(&bson::Bson::Null)
    );
    assert_eq!(
        fields.get("event_ref_relay_binding"),
        Some(&bson::Bson::Null)
    );
    let thread = thread(&f).await;
    assert!(!format!("{thread:?}").contains(f.event.relay_message_id.as_deref().unwrap()));
    let mut legacy = bson::to_document(&thread).unwrap();
    legacy.remove("event_ref_relay_message_id");
    legacy.remove("event_ref_relay_binding");
    let legacy: NyxbotThread = bson::from_document(legacy).unwrap();
    assert!(legacy.event_ref_relay_message_id.is_none());
    f.state.db.drop().await.unwrap();
}

async fn stored_recovery(f: &mut Fixture) -> String {
    use crate::{
        models::nyxbot_gateway_response::{COLLECTION_NAME, GatewayResponse},
        services::nyxbot_response_recovery as recovery,
    };
    f.channel.binding_id = Some("notice-recovery".into());
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    let mut payload = recovery::Payload::new("notice-response", "nyxbot-test", Default::default());
    payload.finish(Some("Useful answer"), false);
    let stored = GatewayResponse {
        id: Uuid::new_v4().to_string(),
        response_id: "notice-response".into(),
        user_id: OWNER.into(),
        channel_id: f.channel.id.clone(),
        binding_id: "notice-recovery".into(),
        partition: PARTITION.into(),
        event_key: f.event.id.clone(),
        request_hash: "fixture".into(),
        conversation_id: f.event.conversation_id.clone(),
        terminal: true,
        revision: 1,
        payload_ciphertext: payload.seal(&f.state.encryption_keys).await.unwrap(),
        created_at: Utc::now(),
        expires_at: Utc::now() + ChronoDuration::hours(1),
    };
    f.state
        .db
        .collection::<GatewayResponse>(COLLECTION_NAME)
        .insert_one(&stored)
        .await
        .unwrap();
    stored.response_id
}
async fn recovery_bytes(f: &Fixture, id: &str) -> (String, String) {
    let snapshot = Box::pin(native_responses::snapshot(
        State(f.state.clone()),
        Path(id.to_owned()),
        bearer(&f.key),
    ))
    .await;
    assert_eq!(snapshot.status(), StatusCode::OK);
    let replay = Box::pin(native_responses::events(
        State(f.state.clone()),
        Path(id.to_owned()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: None,
        }),
        bearer(&f.key),
    ))
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    (body_text(snapshot).await, body_text(replay).await)
}

#[tokio::test]
async fn supplemental_recovery_snapshot_and_replay_are_byte_identical() {
    Box::pin(async {
        for outcome in ["confirmed", "rejected", "unknown"] {
            let (mut f, platform, row) = Box::pin(notice_fixture()).await;
            mock_platform(&platform, outcome).await;
            let gateway = GatewayContract::mount(&f, false).await;
            let id = stored_recovery(&mut f).await;
            let (mut baseline_fixture, _, _) = Box::pin(notice_fixture()).await;
            let baseline_id = stored_recovery(&mut baseline_fixture).await;
            let baseline = recovery_bytes(&baseline_fixture, &baseline_id).await;
            assert!(baseline.0.contains("Useful answer"));
            notice(&f, &row).await;
            let after = recovery_bytes(&f, &id).await;
            assert_eq!(baseline, after);
            assert_eq!(status(&f).await, "replay");
            assert!(gateway.final_reply(&f, "Useful answer").await);
            late_delivery::process(&f.state, &f.event.id).await;
            assert!(sends(&f).await.is_empty());
            assert_eq!(platform.received_requests().await.unwrap().len(), 2);
            no_event_replies(&f).await;
            f.state.db.drop().await.unwrap();
            baseline_fixture.state.db.drop().await.unwrap();
        }
    })
    .await
}

#[tokio::test]
async fn supplemental_recovery_get_races_unchanged_late_sender() {
    Box::pin(async {
        let (mut f, platform, row) = Box::pin(notice_fixture()).await;
        mock_platform(&platform, "confirmed").await;
        let gateway = GatewayContract::mount(&f, false).await;
        let id = stored_recovery(&mut f).await;
        notice(&f, &row).await;
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        let (snapshot, ()) = tokio::join!(
            native_responses::snapshot(State(f.state.clone()), Path(id), bearer(&f.key)),
            late_delivery::process(&f.state, &f.event.id)
        );
        assert_eq!(snapshot.status(), StatusCode::OK);
        let text = body_text(snapshot).await;
        let state = status(&f).await;
        assert!(matches!(state.as_str(), "sent" | "replay"));
        assert_eq!(text.contains("Useful answer"), state == "replay");
        assert_eq!(sends(&f).await.len(), usize::from(state == "sent"));
        if state == "replay" {
            assert!(gateway.final_reply(&f, "Useful answer").await);
        }
        assert_eq!(
            sends(&f).await.len() + platform.received_requests().await.unwrap().len() - 1,
            1
        );
        no_event_replies(&f).await;
        f.state.db.drop().await.unwrap();
    })
    .await
}

#[tokio::test]
async fn supplemental_notice_then_multilingual_multipart_and_images() {
    Box::pin(async {
        let (f, platform, row) = Box::pin(notice_fixture()).await;
        mock_platform(&platform, "confirmed").await;
        mock_sends(&platform, "telegram", false, Duration::ZERO).await;
        let gateway = GatewayContract::mount(&f, false).await;
        add_images(&f).await;
        notice(&f, &row).await;
        let wire = live(&f, false).await;
        assert!(wire.contains("Useful answer"));
        let text = "中文 English မြန်မာစာ\n".repeat(1200);
        assert!(gateway.final_reply(&f, &text).await);
        late_delivery::process(&f.state, &f.event.id).await;
        let requests = platform.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.url.path().ends_with("sendPhoto"))
                .count(),
            2
        );
        let parts: Vec<Value> = requests
            .iter()
            .filter(|r| r.url.path().ends_with("sendMessage"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert!(parts.len() > 3);
        let reconstructed: String = parts
            .iter()
            .skip(1)
            .map(|p| p["text"].as_str().unwrap())
            .collect();
        assert_eq!(reconstructed, text);
        no_event_replies(&f).await;
        f.state.db.drop().await.unwrap();
    })
    .await
}

#[tokio::test]
async fn supplemental_direct_and_relay_era_keep_identical_request_bytes() {
    Box::pin(async {
        for transport in ["direct", "gateway"] {
            let (f, platform) = image_fixture("telegram", transport).await;
            mock_platform(&platform, "confirmed").await;
            let row = engine::get(
                &f.state.db,
                OWNER,
                f.event.conversation_id.as_deref().unwrap(),
            )
            .await
            .unwrap();
            let origin = &f.event.delivery.as_ref().unwrap().origin;
            direct_reply(
                &f.state,
                &f.channel,
                f.event.relay_message_id.as_deref().unwrap(),
                "unchanged",
                None,
            )
            .await
            .unwrap();
            deliver_to(&f.state, &row, origin, "unchanged", None).await;
            let requests = platform.received_requests().await.unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].body, requests[1].body);
            assert!(f.gateway.received_requests().await.unwrap().is_empty());
            f.state.db.drop().await.unwrap();
        }
    })
    .await
}

#[tokio::test]
async fn supplemental_missing_delivery_target_is_a_routine_skip() {
    let (f, platform, mut row) = Box::pin(notice_fixture()).await;
    row.active_turn = None;
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .delete_many(doc! {"channel_id": &f.channel.id, "conversation_id": &row.id})
        .await
        .unwrap();
    deliver_to(
        &f.state,
        &row,
        &f.event.delivery.as_ref().unwrap().origin,
        "routine skip",
        None,
    )
    .await;
    assert!(platform.received_requests().await.unwrap().is_empty());
    let audits = audit_decisions(&f).await;
    assert_eq!(audits.len(), 1);
    let data = audits[0].get_document("event_data").unwrap();
    assert_eq!(data.get_str("egress"), Ok("skipped"));
    assert_eq!(data.get_str("reason"), Ok("no_destination"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn supplemental_guest_and_org_route_authority_are_revalidated() {
    Box::pin(async {
        for mode in ["guest","org","guest_revoked","org_revoked","route_revoked","platform_manager"] {
            let (f,platform,mut row)=Box::pin(notice_fixture()).await;
            mock_platform(&platform,"confirmed").await;
            let gateway=GatewayContract::mount(&f,false).await;
            if mode.starts_with("guest") {
                patch(&f,doc!{"delivery.guest":true,"delivery.sender_id":"guest"}).await;
                f.state.db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME).update_one(doc!{"_id":&f.event.relay_message_id},doc!{"$set":{"sender_platform_id":"guest"}}).await.unwrap();
                f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc!{"_id":&f.channel.id},doc!{"$set":{"private_chats":if mode=="guest" {"everyone"}else{"owner"}}}).await.unwrap();
                f.state.db.collection::<bson::Document>(THREADS).update_one(doc!{"channel_id":&f.channel.id},doc!{"$set":{"owner_chat":false,"allow_posts":false}}).await.unwrap();
                row.guest_turn=true;
            }
            if mode.starts_with("org") {
                let org=Uuid::new_v4().to_string();
                f.state.db.collection(USERS).insert_one(test_user(&org,UserType::Org)).await.unwrap();
                f.state.db.collection::<bson::Document>("org_memberships").insert_one(doc!{"_id":Uuid::new_v4().to_string(),"org_user_id":&org,"member_user_id":OWNER,"role":"admin","allowed_service_ids":bson::Bson::Null,"created_at":bson::DateTime::now(),"revoked_at":bson::Bson::Null}).await.unwrap();
                f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc!{"_id":&f.channel.id},doc!{"$set":{"bot_owner_id":&org}}).await.unwrap();
                for (collection,filter) in [("channel_bots",doc!{"_id":&f.channel.channel_bot_id}),("channel_conversations",doc!{"_id":&f.channel.route_id}),("api_keys",doc!{"_id":&f.channel.route_api_key_id}),("channel_messages",doc!{"_id":&f.event.relay_message_id})] {
                    f.state.db.collection::<bson::Document>(collection).update_one(filter,doc!{"$set":{"user_id":&org}}).await.unwrap();
                }
                if mode=="org_revoked" {f.state.db.collection::<bson::Document>("org_memberships").delete_many(doc!{"org_user_id":&org}).await.unwrap();}
            }
            if mode=="route_revoked" {f.state.db.collection::<bson::Document>("api_keys").update_one(doc!{"_id":&f.channel.route_api_key_id},doc!{"$set":{"is_active":false}}).await.unwrap();}
            if mode=="platform_manager" {f.state.db.collection::<bson::Document>("channel_bots").update_one(doc!{"_id":&f.channel.channel_bot_id},doc!{"$set":{"platform_profile_id":"managed"}}).await.unwrap();}
            deliver_to_for(&f.state,&row,&f.event.delivery.as_ref().unwrap().origin,"progress",None,Purpose::MachineDesktopNotice).await;
            assert_eq!(platform.received_requests().await.unwrap().len(),usize::from(matches!(mode,"guest"|"org")),"{mode}");
            no_event_replies(&f).await;
            assert_eq!(status(&f).await,"waiting");
            if mode=="guest" { assert!(gateway.final_reply(&f,"guest answer").await); }
            f.state.db.drop().await.unwrap();
        }
    }).await;
}

#[tokio::test]
async fn supplemental_no_notice_stream_and_recovery_have_no_supplemental_reads() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::{collections::BTreeMap, sync::Mutex};
    Box::pin(async {
        let (mut f, platform, row) = Box::pin(notice_fixture()).await;
        mock_platform(&platform, "confirmed").await;
        let counts = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let observed = counts.clone();
        let monitored = Box::pin(
            crate::test_utils::connect_transaction_test_database_with_command_handler(
                "supplemental_read_counts",
                EventHandler::callback(move |event| {
                    if let CommandEvent::Started(event) = event
                        && matches!(event.command_name.as_str(), "find" | "aggregate" | "count")
                    {
                        let collection = event.command.get_str(&event.command_name).unwrap_or("");
                        *observed
                            .lock()
                            .unwrap()
                            .entry(format!("{}:{collection}", event.command_name))
                            .or_default() += 1;
                    }
                }),
            ),
        )
        .await;
        f.state.db = monitored.client().database(f.state.db.name());
        counts.lock().unwrap().clear();
        let before = live(&f, false).await;
        let before_reads = counts.lock().unwrap().clone();
        assert!(
            !before_reads
                .keys()
                .any(|key| key.contains("channel_messages"))
        );
        patch(&f, doc! {"delivery.state":"waiting"}).await;
        notice(&f, &row).await;
        counts.lock().unwrap().clear();
        assert_eq!(live(&f, false).await, before);
        assert_eq!(*counts.lock().unwrap(), before_reads);
        let id = stored_recovery(&mut f).await;
        let first = recovery_bytes(&f, &id).await;
        counts.lock().unwrap().clear();
        assert_eq!(recovery_bytes(&f, &id).await, first);
        let recovery_reads = counts.lock().unwrap().clone();
        assert!(
            !recovery_reads
                .keys()
                .any(|key| key.contains("channel_messages"))
        );
        assert!(platform.received_requests().await.unwrap().len() == 1);
        f.state.db.drop().await.unwrap();
        monitored.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_settled_turn_cannot_claim_a_checking_notice() {
    let (f, platform, row) = Box::pin(notice_fixture()).await;
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&row.id}, doc! {"$unset":{"active_turn":""}})
        .await
        .unwrap();
    assert!(
        !crate::handlers::assistant_nyxagent::send_checking_notice(
            &f.state,
            &row,
            &f.event.delivery.as_ref().unwrap().origin
        )
        .await
    );
    assert!(platform.received_requests().await.unwrap().is_empty());
    no_event_replies(&f).await;
    assert!(audit_decisions(&f).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn supplemental_two_notice_replicas_race_the_provider_stream_claim() {
    Box::pin(async {
        let (f, platform, row) = Box::pin(notice_fixture()).await;
        mock_platform(&platform, "confirmed").await;
        let gateway = GatewayContract::mount(&f, false).await;
        let origin = &f.event.delivery.as_ref().unwrap().origin;
        let replica = f.state.clone();
        let (first, second, wire) = tokio::join!(
            crate::handlers::assistant_nyxagent::send_checking_notice(&f.state, &row, origin),
            crate::handlers::assistant_nyxagent::send_checking_notice(&replica, &row, origin),
            live(&f, false)
        );
        assert_ne!(first, second);
        assert!(wire.contains("Useful answer"));
        assert_eq!(status(&f).await, "streamed");
        assert!(gateway.final_reply(&f, "Useful answer").await);
        assert_eq!(platform.received_requests().await.unwrap().len(), 2);
        assert_eq!(audit_decisions(&f).await.len(), 1);
        no_event_replies(&f).await;
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_notice_preserves_recovered_failure_and_one_failure_delivery() {
    Box::pin(async {
        let (mut f,platform,row)=Box::pin(notice_fixture()).await;
        mock_platform(&platform,"confirmed").await;
        let gateway=GatewayContract::mount(&f,false).await;
        let id=stored_recovery(&mut f).await;
        let mut payload=crate::services::nyxbot_response_recovery::Payload::new(&id,"nyxbot-test",Default::default());
        payload.finish(None,true);
        let cipher=payload.seal(&f.state.encryption_keys).await.unwrap();
        f.state.db.collection::<bson::Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME).update_one(doc!{"response_id":&id},doc!{"$set":{"payload_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:cipher}}}).await.unwrap();
        let (mut baseline_fixture,_,_)=Box::pin(notice_fixture()).await;
        let baseline_id=stored_recovery(&mut baseline_fixture).await;
        let cipher=payload.seal(&baseline_fixture.state.encryption_keys).await.unwrap();
        baseline_fixture.state.db.collection::<bson::Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME).update_one(doc!{"response_id":&baseline_id},doc!{"$set":{"payload_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:cipher}}}).await.unwrap();
        let baseline=recovery_bytes(&baseline_fixture,&baseline_id).await;
        assert!(baseline.0.contains("failed"));
        notice(&f,&row).await;
        assert_eq!(recovery_bytes(&f,&id).await,baseline);
        assert!(gateway.final_reply(&f,"NyxBot could not finish this turn.").await);
        late_delivery::process(&f.state,&f.event.id).await;
        assert!(sends(&f).await.is_empty());
        assert_eq!(platform.received_requests().await.unwrap().len(),2);
        no_event_replies(&f).await;
        f.state.db.drop().await.unwrap();
        baseline_fixture.state.db.drop().await.unwrap();
    }).await;
}
