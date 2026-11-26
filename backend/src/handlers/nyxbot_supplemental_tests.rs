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

#[tokio::test]
async fn supplemental_native_side_messages_lark_keeps_local_relay() {
    Box::pin(async {
        let (f, platform) = image_fixture("lark", "gateway").await;
        assert!(f.channel.gateway_source.is_none());
        Mock::given(method("POST"))
            .and(path("/open-apis/im/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"sent"}})),
            )
            .expect(1)
            .mount(&platform)
            .await;
        Mock::given(method("GET"))
            .and(path("/events/cmaeg1.ev.original/side-messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"available":true})))
            .expect(0)
            .mount(&f.gateway)
            .await;
        let conversation = engine::get(
            &f.state.db,
            OWNER,
            f.event.conversation_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
        let origin = &f.event.delivery.as_ref().unwrap().origin;
        deliver_to(&f.state, &conversation, origin, "Independent result", None).await;
        platform.verify().await;
        assert!(f.gateway.received_requests().await.unwrap().is_empty());
        let audits = audit_decisions(&f).await;
        assert_eq!(audits.len(), 1);
        let data = audits[0].get_document("event_data").unwrap();
        assert_eq!(data.get_str("egress"), Ok("local_relay"));
        assert_eq!(data.get_str("outcome"), Ok("confirmed"));
        f.state.db.drop().await.unwrap();
    })
    .await;
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
            let turn = row.active_turn.as_mut().unwrap();
            turn.origin = crate::models::assistant_conversation::TurnOrigin::Event;
            turn.channel_event_id = None;
            turn.asked_from = None;
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
        answer_delivery: None,
        revision: 1,
        payload_context_version: None,
        payload_ciphertext: payload.seal_legacy(&f.state.encryption_keys).await.unwrap(),
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
            assert_eq!(
                f.state
                    .db
                    .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
                    .count_documents(doc! {"supplemental_result_claims":{"$exists":true}})
                    .await
                    .unwrap(),
                0
            );
            assert!(f.gateway.received_requests().await.unwrap().is_empty());
            let legacy_chat = direct_post_turn_chat(&f, "124").await;
            let mut fields = doc! {"deliver_also":[bson::to_bson(&legacy_chat).unwrap()]};
            if transport == "gateway" {
                fields.insert("channel", bson::Bson::Null);
                fields.insert("reply_channel", bson::to_bson(origin).unwrap());
            }
            f.state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .update_one(doc! {"_id":&row.id}, doc! {"$set":fields})
                .await
                .unwrap();
            let row =
                settle_direct_post_turn(&f, TurnOrigin::Event, &[], "Useful answer", None).await;
            let conversation = engine::get(&f.state.db, OWNER, &row.id).await.unwrap();
            assert_eq!(
                conversation.deliver_also.as_slice(),
                std::slice::from_ref(&legacy_chat)
            );
            result_hook(&f, &row).await;
            result_hook(&f, &row).await;
            let requests = platform.received_requests().await.unwrap();
            assert_eq!(requests.len(), 3);
            let body = requests[2].body_json::<Value>().unwrap();
            assert_eq!(body["chat_id"], "123");
            assert_eq!(body["text"], "Useful answer");
            assert!(
                audit_decisions(&f)
                    .await
                    .iter()
                    .any(|audit| { result_reason(audit) == Some("already_claimed") })
            );
            let conversation = engine::get(&f.state.db, OWNER, &row.id).await.unwrap();
            assert_eq!(conversation.deliver_also, [legacy_chat]);
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
        let cipher=payload.seal_legacy(&f.state.encryption_keys).await.unwrap();
        f.state.db.collection::<bson::Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME).update_one(doc!{"response_id":&id},doc!{"$set":{"payload_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:cipher}}}).await.unwrap();
        let (mut baseline_fixture,_,_)=Box::pin(notice_fixture()).await;
        let baseline_id=stored_recovery(&mut baseline_fixture).await;
        let cipher=payload.seal_legacy(&baseline_fixture.state.encryption_keys).await.unwrap();
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

async fn result_fixture(age: ChronoDuration) -> (Fixture, MockServer, AssistantConversation) {
    let (f, platform, mut row) = Box::pin(notice_fixture()).await;
    let turn = row.active_turn.as_mut().unwrap();
    turn.origin = crate::models::assistant_conversation::TurnOrigin::Event;
    turn.channel_event_id = None;
    turn.asked_from = None;
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn":bson::Bson::Null}},
        )
        .await
        .unwrap();
    age_result_anchor(&f, age).await;
    (f, platform, row)
}

async fn age_result_anchor(f: &Fixture, age: ChronoDuration) {
    let at = Utc::now() - age;
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.event.relay_message_id},
            doc! {"$set":{"created_at":bson::DateTime::from_chrono(at)}},
        )
        .await
        .unwrap();
    let fields = sources::reference_fields(
        f.state
            .encryption_keys
            .encrypt(b"cmaeg1.ev.original")
            .await
            .unwrap(),
        at + ChronoDuration::minutes(29),
        f.event.relay_message_id.as_deref(),
    );
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(doc! {"channel_id":&f.channel.id}, doc! {"$set":fields})
        .await
        .unwrap();
}

async fn result_hook(f: &Fixture, row: &AssistantConversation) {
    Box::pin(crate::handlers::assistant_team::after_turn(
        &f.state,
        row,
        "Useful answer",
        None,
    ))
    .await;
}

async fn settle_direct_post_turn(
    f: &Fixture,
    origin: crate::models::assistant_conversation::TurnOrigin,
    also: &[ChannelOrigin],
    text: &str,
    error: Option<engine::TurnError>,
) -> AssistantConversation {
    let conversation = f.event.conversation_id.as_deref().unwrap();
    let turn_id = Uuid::new_v4().to_string();
    let db = &f.state.db;
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":conversation},
            doc! {"$set":{"active_turn":{
                "turn_id":&turn_id,
                "origin":bson::to_bson(&origin).unwrap(),
                "started_at":bson::DateTime::now(),
                "stop_requested":false,
            }}},
        )
        .await
        .unwrap();
    let row = engine::get(db, OWNER, conversation).await.unwrap();
    for origin in also {
        assert!(
            engine::also_deliver(db, OWNER, conversation, &turn_id, origin)
                .await
                .unwrap()
        );
    }
    let code = error.as_ref().map(|error| error.code);
    let result = engine::TurnResult {
        text: text.into(),
        session_id: None,
        response_id: None,
        error,
    };
    let reply_id = Uuid::new_v4().to_string();
    let error = engine::finish_turn(db, &row, &row.credential_api_key_id, &reply_id, &result)
        .await
        .unwrap();
    assert_eq!(error.as_ref().map(|error| error.code), code);
    row
}

async fn direct_post_turn_chat(f: &Fixture, chat_id: &str) -> ChannelOrigin {
    use crate::models::channel_message::COLLECTION_NAME as MESSAGES;
    let db = &f.state.db;
    let mut origin = f.event.delivery.as_ref().unwrap().origin.clone();
    let mut chat = thread(f).await;
    let messages = db.collection::<bson::Document>(MESSAGES);
    let inbound_id = Uuid::new_v4().to_string();
    let mut inbound = messages
        .find_one(doc! {"_id":&f.event.relay_message_id})
        .await
        .unwrap()
        .unwrap();
    inbound.insert("_id", &inbound_id);
    inbound.insert("platform_conversation_id", chat_id);
    messages.insert_one(inbound).await.unwrap();
    origin.partition = format!("private-{chat_id}");
    chat.id = Uuid::new_v4().to_string();
    chat.partition.clone_from(&origin.partition);
    chat.platform_chat_id = Some(chat_id.into());
    chat.last_message_id = Some(inbound_id.clone());
    chat.last_relay_message_id = Some(inbound_id);
    db.collection::<NyxbotThread>(THREADS)
        .insert_one(chat)
        .await
        .unwrap();
    origin
}

#[tokio::test]
async fn post_turn_result_direct_repeat_sends_once() {
    Box::pin(async {
        use crate::models::assistant_conversation::TurnOrigin;
        let (f, platform) = Box::pin(image_fixture("telegram", "direct")).await;
        mock_platform(&platform, "confirmed").await;
        let row = settle_direct_post_turn(&f, TurnOrigin::Event, &[], "Useful answer", None).await;
        tokio::join!(
            result_hook(&f, &row),
            Box::pin(crate::handlers::assistant_team::after_turn(
                &f.state,
                &row,
                "Uncommitted answer",
                None,
            )),
        );

        let requests = platform.received_requests().await.unwrap();
        let audits = audit_decisions(&f).await;
        f.state.db.drop().await.unwrap();
        assert_eq!(requests.len(), 1, "requests: {requests:?}");
        let body = requests[0].body_json::<Value>().unwrap();
        assert_eq!(body["chat_id"], "123");
        assert_eq!(body["text"], "Useful answer");
        let turn_id = row.active_turn.as_ref().unwrap().turn_id.as_str();
        assert_eq!(audits.len(), 2, "audits: {audits:?}");
        assert!(audits.iter().all(|audit| {
            audit.get_document("event_data").unwrap().get_str("turn_id") == Ok(turn_id)
        }));
        assert_eq!(
            audits
                .iter()
                .filter(|audit| result_reason(audit) == Some("already_claimed"))
                .count(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn post_turn_result_meta_reply_keeps_grant_and_claims_destination_once() {
    Box::pin(async {
        use crate::models::assistant_conversation::ChannelTurnContext;
        use crate::services::channel_adapters::meta::{self, tests};
        use wiremock::matchers::method;

        let (f, graph) = Box::pin(image_fixture("instagram", "direct")).await;
        let db = &f.state.db;
        let bot = tests::registered_bot(&f.state, OWNER, "instagram").await;
        let route = f.channel.route_id.as_deref().unwrap();
        let inbound = tests::inbound_for(
            &f.state,
            &bot,
            route,
            &f.channel.route_api_key_id,
            "333",
            "mid.settled-result",
        )
        .await;
        tests::admit_original(&f.state, &inbound).await;
        let chat = inbound.platform_conversation_id.as_deref().unwrap();
        db.collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id": &f.channel.id},
                doc! {"$set": {"channel_bot_id": &bot.id}},
            )
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id": route},
                doc! {"$set": {"channel_bot_id": &bot.id, "platform_conversation_id": chat}},
            )
            .await
            .unwrap();
        db.collection::<bson::Document>(THREADS)
            .update_one(
                doc! {"channel_id": &f.channel.id},
                doc! {"$set": {
                    "platform_chat_id": chat,
                    "last_message_id": &inbound.id,
                    "last_relay_message_id": &inbound.id,
                }},
            )
            .await
            .unwrap();

        let grant = crate::services::channel_meta_service::scopes::reference(&bot).unwrap();
        let origin = &f.event.delivery.as_ref().unwrap().origin;
        let conversation = f.event.conversation_id.as_deref().unwrap();
        let request: engine::TurnRequest = serde_json::from_value(json!({
            "conversation_id": conversation, "text": "Summarize the verified inbox context"
        }))
        .unwrap();
        let mut start = engine::TurnStart::from(&request);
        // Seed trusted Meta provenance and add the same verified chat through
        // the running-turn writer. Signed admission is covered by the Meta
        // integration fixtures.
        start.channel_context = Some(ChannelTurnContext {
            channel_guest: None,
            meta_grant: Some(grant.clone()),
            text_completeness: None,
            org_origin: None,
            display_name: None,
            conversation_label: None,
            followed: false,
            source_event_id: inbound.id.clone(),
            channel_id: f.channel.id.clone(),
            channel_bot_id: bot.id.clone(),
            conversation_kind: "private".into(),
            sender_kind: crate::models::channel_thread::ThreadSenderKind::Human,
            guest: false,
            original_message: true,
        });
        let row = engine::begin_turn(db, OWNER, start, &f.state.encryption_keys)
            .await
            .unwrap();
        let turn = &row.active_turn.as_ref().unwrap().turn_id;
        assert!(
            engine::also_deliver(db, OWNER, conversation, turn, origin)
                .await
                .unwrap()
        );
        assert!(
            engine::finish_turn(
                db,
                &row,
                &row.credential_api_key_id,
                &Uuid::new_v4().to_string(),
                &engine::TurnResult {
                    text: "Verified Meta result".into(),
                    session_id: None,
                    response_id: None,
                    error: None,
                },
            )
            .await
            .unwrap()
            .is_none()
        );
        let saved = result_message(&f, &row).await;
        assert_eq!(
            saved.get("meta_grants"),
            Some(&bson::to_bson(&[grant]).unwrap())
        );
        assert_eq!(
            saved.get("post_turn_deliver_also"),
            Some(&bson::to_bson(&[origin]).unwrap())
        );

        Mock::given(method("POST"))
            .and(path("/v25.0/222/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message_id": "sent"})))
            .expect(1)
            .mount(&graph)
            .await;
        meta::with_test_graph_base(graph.uri(), async {
            result_hook(&f, &row).await;
            result_hook(&f, &row).await;
        })
        .await;
        let requests = graph.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let sent = requests[0].body_json::<Value>().unwrap();
        assert_eq!(sent["recipient"]["id"], "333");
        assert_eq!(sent["message"]["text"], "Verified Meta result");
        let audits = audit_decisions(&f).await;
        assert_eq!(audits.len(), 2);
        let decisions = audits
            .iter()
            .map(|audit| audit.get_document("event_data").unwrap())
            .collect::<Vec<_>>();
        assert!(decisions.iter().all(|audit| {
            audit.get_str("turn_id") == Ok(turn.as_str())
                && audit.get_str("purpose") == Ok("team_additional_result")
        }));
        assert!(
            decisions
                .iter()
                .any(|audit| audit.get_str("outcome") == Ok("confirmed"))
        );
        assert_eq!(
            audits
                .iter()
                .filter(|audit| result_reason(audit) == Some("already_claimed"))
                .count(),
            1
        );
        db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn post_turn_result_older_hook_cannot_take_newer_additional_chat() {
    Box::pin(check_post_turn_result_destinations(false)).await;
}

#[tokio::test]
async fn post_turn_result_only_the_failed_turns_additional_chat_gets_a_notice() {
    Box::pin(check_post_turn_result_destinations(true)).await;
}

async fn check_post_turn_result_destinations(newer_failed: bool) {
    Box::pin(async {
        use crate::models::assistant_conversation::TurnOrigin;
        let (f, platform) = Box::pin(image_fixture("telegram", "direct")).await;
        mock_platform(&platform, "confirmed").await;
        f.state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&f.event.conversation_id},
                doc! {"$unset":{"channel":""}},
            )
            .await
            .unwrap();
        let chat_a = direct_post_turn_chat(&f, "124").await;
        let chat_b = direct_post_turn_chat(&f, "125").await;
        let a = settle_direct_post_turn(&f, TurnOrigin::User, &[chat_a], "Answer A", None).await;
        let b = settle_direct_post_turn(
            &f,
            TurnOrigin::User,
            &[chat_b],
            "Answer B",
            newer_failed.then(|| engine::TurnError::internal("upstream_failed", "upstream")),
        )
        .await;

        // A's hook reaches delivery after B has already committed settlement.
        Box::pin(crate::handlers::assistant_team::after_turn(
            &f.state, &a, "Answer A", None,
        ))
        .await;
        for _ in 0..2 {
            Box::pin(crate::handlers::assistant_team::after_turn(
                &f.state, &b, "Answer B", None,
            ))
            .await;
        }

        let requests = platform.received_requests().await.unwrap();
        let audits = audit_decisions(&f).await;
        f.state.db.drop().await.unwrap();
        let sent = requests
            .iter()
            .map(|request| {
                let body = request.body_json::<Value>().unwrap();
                (body["chat_id"].clone(), body["text"].clone())
            })
            .collect::<Vec<_>>();
        let answer_b = if newer_failed {
            "I couldn't finish answering that question. Please ask again."
        } else {
            "Answer B"
        };
        assert_eq!(
            sent,
            [
                (json!("124"), json!("Answer A")),
                (json!("125"), json!(answer_b))
            ]
        );
        assert_eq!(audits.len(), 3, "audits: {audits:?}");
        for row in [&a, &b] {
            let turn = row.active_turn.as_ref().unwrap();
            assert!(
                audits.iter().any(|audit| {
                    let data = audit.get_document("event_data").unwrap();
                    data.get_str("turn_id") == Ok(turn.turn_id.as_str())
                        && data.get_str("purpose") == Ok("team_additional_result")
                        && data.get_str("outcome") == Ok("confirmed")
                }),
                "audits: {audits:?}"
            );
        }
        let duplicate = audits
            .iter()
            .find(|audit| result_reason(audit) == Some("already_claimed"))
            .unwrap();
        assert_eq!(
            duplicate
                .get_document("event_data")
                .unwrap()
                .get_str("turn_id"),
            Ok(b.active_turn.as_ref().unwrap().turn_id.as_str())
        );
    })
    .await;
}

#[tokio::test]
async fn post_turn_result_direct_preparation_and_claim_error_leave_no_marker() {
    Box::pin(async {
        let (f, platform) = Box::pin(image_fixture("telegram", "direct")).await;
        mock_platform(&platform, "confirmed").await;
        let row = settle_direct_post_turn(&f, TurnOrigin::Event, &[], "Useful answer", None).await;
        let db = &f.state.db;
        let threads = db.collection::<bson::Document>(THREADS);
        let chat = doc! {"channel_id":&f.channel.id};
        threads
            .update_one(
                chat.clone(),
                doc! {"$set":{"last_message_id":Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
        result_hook(&f, &row).await;
        threads
            .update_one(
                chat,
                doc! {"$set":{"last_message_id":&f.event.relay_message_id}},
            )
            .await
            .unwrap();
        let collection = crate::models::assistant_message::COLLECTION_NAME;
        db.run_command(doc! {
            "collMod":collection,
            "validator":{"supplemental_result_claims":{"$exists":false}},
        })
        .await
        .unwrap();
        result_hook(&f, &row).await;
        assert!(platform.received_requests().await.unwrap().is_empty());
        assert!(
            !result_message(&f, &row)
                .await
                .contains_key("supplemental_result_claims")
        );
        let audits = audit_decisions(&f).await;
        assert_eq!(audits.len(), 2);
        for reason in ["authority_refused", "source_unavailable"] {
            assert!(
                audits
                    .iter()
                    .any(|audit| result_reason(audit) == Some(reason))
            );
        }

        let origin = row.channel.as_ref().unwrap();
        deliver_to_for(
            &f.state,
            &row,
            origin,
            "Progress",
            None,
            Purpose::MachineDesktopNotice,
        )
        .await;
        assert!(
            !result_message(&f, &row)
                .await
                .contains_key("supplemental_result_claims")
        );
        db.run_command(doc! {"collMod":collection,"validator":{}})
            .await
            .unwrap();
        result_hook(&f, &row).await;
        let requests = platform.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        for (request, expected) in requests.iter().zip(["Progress", "Useful answer"]) {
            let body = request.body_json::<Value>().unwrap();
            assert_eq!(body["chat_id"], "123");
            assert_eq!(body["text"], expected);
        }
        db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn private_chat_handoff_direct_anchor_is_saved_only() {
    let (mut fixture, _) = Box::pin(image_fixture("telegram", "direct")).await;
    let thread = fixture
        .state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&fixture.channel.id})
        .await
        .unwrap()
        .unwrap();
    fixture.event.relay_message_id = None;
    for saved in [false, true] {
        let mut decision = supplemental::Decision::new(Purpose::TeamResult);
        decision.saved_result = saved;
        let anchor = supplemental::event_anchor(
            &fixture.state,
            &fixture.event,
            &fixture.channel,
            &thread,
            &mut decision,
        )
        .await
        .unwrap();
        assert_eq!(
            anchor.as_deref(),
            saved.then_some(fixture.event.event_id.as_str())
        );
    }
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn private_chat_handoff_ordinary_subscriber_close_keeps_failure_copy() {
    let fixture = Box::pin(fixture("closed_channel_subscriber")).await;
    let closed = || {
        let (sender, receiver) = broadcast::channel(1);
        drop(sender);
        receiver
    };
    let error = final_reply(closed()).await.unwrap_err();
    let expected = engine::TurnError::new("turn_lost");
    assert_eq!(
        (error.code, error.message),
        (expected.code, expected.message)
    );
    let wire = body_text(late_delivery::provider_stream(
        fixture.state.clone(),
        fixture.event.id.clone(),
        "response".into(),
        json!({"type":"response.created"}),
        closed(),
        Utc::now(),
    ))
    .await;
    assert!(
        wire.contains("response.failed")
            && wire.contains("turn_lost")
            && wire.contains("NyxBot could not finish this turn."),
        "{wire}"
    );
    assert_eq!(status(&fixture).await, "streamed");
    assert!(sends(&fixture).await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_wake_expired_saved_delivery_preserves_owner_history() {
    let (fixture, platform, row) = Box::pin(result_fixture(ChronoDuration::minutes(1))).await;
    let saved = result_message(&fixture, &row).await;
    let reply = saved.get_str("_id").unwrap();
    let origin = &fixture.event.delivery.as_ref().unwrap().origin;
    let source = sources::ResultSource::capture(row.active_turn.as_ref().unwrap(), origin);
    let messages = fixture
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME);
    messages.update_one(doc! {"_id":reply},doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::hours(24)),"handoff_delivery":{"agent_id":&row.agent_id,"targets":[{"source":bson::to_bson(&source).unwrap(),"origin":bson::to_bson(origin).unwrap(),"additional":false}]},"handoff_delivery_due_at":bson::DateTime::from_millis(0)}}).await.unwrap();
    Box::pin(handoff_delivery::deliver(&fixture.state, reply))
        .await
        .unwrap();
    let after = messages
        .find_one(doc! {"_id":reply})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !after.contains_key("handoff_delivery_due_at")
            && !after.contains_key("handoff_delivery_lease")
    );
    let history = Box::pin(crate::handlers::assistant_nyxagent::history(
        State(fixture.state.clone()),
        crate::handlers::assistant_team::owner_auth(OWNER).unwrap(),
        Path(row.id),
        axum::extract::Query(Default::default()),
    ))
    .await
    .unwrap()
    .0;
    let history = serde_json::to_value(history).unwrap();
    let messages = history["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["text"], "Useful answer");
    assert!(platform.received_requests().await.unwrap().is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn private_chat_handoff_settlement_parity() {
    Box::pin(settlement_parity(false)).await;
}

#[tokio::test]
async fn nyxbot_wake_settlement_parity() {
    Box::pin(settlement_parity(true)).await;
}

async fn settlement_parity(wake: bool) {
    use crate::models::assistant_conversation::{
        EngineWorkerInstance, ReattachDeployment, TurnOrigin,
    };
    use crate::services::{
        assistant_team_service, feature_flag_service as flags, security_readiness_service,
    };
    let (fixture, platform, mut initial) = Box::pin(notice_fixture()).await;
    let db = &fixture.state.db;
    mock_platform(&platform, "confirmed").await;
    security_readiness_service::observe_test_handoff_readiness(db, true).await;
    let messages =
        db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME);
    let conversations = db.collection::<bson::Document>(CONVERSATIONS);
    let events = db.collection::<bson::Document>(EVENTS);
    let original_event = events
        .find_one(doc! {"_id":&fixture.event.id})
        .await
        .unwrap()
        .unwrap();
    let origin = fixture.event.delivery.as_ref().unwrap().origin.clone();
    initial.channel = None;
    initial.reply_channel = Some(origin.clone());
    let active = initial.active_turn.as_mut().unwrap();
    active.initiating_message_seq = Some(1);
    active.worker_instance = Some(EngineWorkerInstance {
        id: Uuid::new_v4().to_string(),
        hostname: None,
    });
    let mut start =
        engine::TurnStart::from(&engine::parse_turn(br#"{"text":"Same question"}"#).unwrap());
    if wake {
        active.origin = TurnOrigin::Event;
        active.channel_event_id = None;
        active.asked_from = None;
        active.events.push(assistant_team_service::event(
            "subagent_settled",
            "done".into(),
            None,
        ));
        initial.event_streak = 1;
    } else {
        active.channel_context = Some(serde_json::from_value(json!({"source_event_id":fixture.event.id,"channel_id":fixture.channel.id,"channel_bot_id":fixture.channel.channel_bot_id,"conversation_kind":"private","sender_kind":"human","guest":false,"original_message":true})).unwrap());
        start.owner_instruction_source =
            Some(engine::OwnerInstructionSource::VerifiedPrivateChannel);
    }
    start.origin = active.origin;
    start.reply_channel = initial.reply_channel.clone();
    let reply_id = Uuid::new_v4().to_string();
    let mut baseline = None;
    for enabled in [false, true] {
        let mut row = initial.clone();
        flags::set_platform_override(
            db,
            flags::TURN_HANDOFF_FLAG_KEY,
            &flags::FlagTarget::User(OWNER.into()),
            enabled,
            OWNER,
        )
        .await
        .unwrap();
        if enabled {
            engine::enroll_restart(&mut row, &start, "question", ReattachDeployment {
                destination_hash: "local-test".into(),
                limits: serde_json::from_value(json!({"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"max_events":64,"recent_events":32,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32})).unwrap(),
            }, false);
            let admission = row
                .active_turn
                .as_mut()
                .unwrap()
                .restart_admission
                .as_mut()
                .unwrap();
            assert_eq!(admission.version, 4);
            admission.reply_id.clone_from(&reply_id);
        }
        messages
            .delete_many(doc! {"conversation_id":&row.id,"role":"assistant"})
            .await
            .unwrap();
        conversations
            .replace_one(doc! {"_id":&row.id}, bson::to_document(&row).unwrap())
            .await
            .unwrap();
        events
            .replace_one(doc! {"_id":&fixture.event.id}, &original_event)
            .await
            .unwrap();
        let result = engine::TurnResult {
            text: "Useful answer".into(),
            session_id: Some("session".into()),
            response_id: Some("response".into()),
            error: None,
        };
        assert!(
            engine::finish_turn(db, &row, &row.credential_api_key_id, &reply_id, &result)
                .await
                .unwrap()
                .is_none()
        );
        let mut reply = messages
            .find_one(doc! {"_id":&reply_id})
            .await
            .unwrap()
            .unwrap();
        let mut conversation = conversations
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        let queued_event = events
            .find_one(doc! {"_id":&fixture.event.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply.remove("settled_by_attempt").is_some(), enabled);
        let saved_route = reply.remove("handoff_delivery");
        assert_eq!(saved_route.is_some(), enabled && wake);
        if let Some(saved_route) = saved_route {
            let active = row.active_turn.as_ref().unwrap();
            let source = sources::ResultSource::capture(active, &origin);
            assert_eq!(
                saved_route,
                bson::Bson::Document(
                    doc! {"agent_id":&row.agent_id,"targets":[{"origin":bson::to_bson(&origin).unwrap(),"source":bson::to_bson(&source).unwrap(),"additional":false}]}
                )
            );
            assert_eq!(
                reply.remove("handoff_delivery_due_at").as_ref(),
                reply.get("created_at")
            );
        }
        let settled_at = reply.remove("created_at").unwrap();
        assert_eq!(conversation.remove("updated_at"), Some(settled_at.clone()));
        if conversation.get_datetime("context_reset_at").is_ok() {
            assert_eq!(
                conversation.remove("context_reset_at"),
                Some(settled_at.clone())
            );
        }
        let metadata = reply.get_document_mut("turn_metadata").unwrap();
        assert_eq!(
            metadata
                .get_document_mut("work")
                .unwrap()
                .remove("settled_at"),
            Some(settled_at.clone())
        );
        let duration = settled_at.as_datetime().unwrap().timestamp_millis()
            - row
                .active_turn
                .as_ref()
                .unwrap()
                .started_at
                .timestamp_millis();
        assert_eq!(
            metadata
                .get_document_mut("timing")
                .unwrap()
                .remove("total_ms"),
            Some(bson::Bson::Int64(duration))
        );
        let saved_repeat_audit = doc! {
            "event_type": "nyxbot_channel_supplemental_delivery",
            "event_data.turn_id": &row.active_turn.as_ref().unwrap().turn_id,
            "event_data.conversation_id": &row.id,
            "event_data.purpose": "team_result",
            "event_data.reason": "already_claimed",
        };
        let audit = db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
        if enabled && wake {
            let due_at = settled_at.as_datetime().unwrap();
            messages
                .update_one(
                    doc! {"_id": &reply_id},
                    doc! {"$set": {"handoff_delivery_due_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::minutes(2)),
                                    "handoff_delivery_lease": "deferred-worker"}},
                )
                .await
                .unwrap();
            Box::pin(crate::handlers::assistant_team::after_turn(
                &fixture.state,
                &row,
                &result.text,
                None,
            ))
            .await;
            assert_eq!(platform.received_requests().await.unwrap().len(), 1);
            assert_eq!(
                audit
                    .count_documents(saved_repeat_audit.clone())
                    .await
                    .unwrap(),
                0
            );
            messages
                .update_one(
                    doc! {"_id": &reply_id},
                    doc! {"$set": {"handoff_delivery_due_at": due_at}, "$unset": {"handoff_delivery_lease": ""}},
                )
                .await
                .unwrap();
        }
        Box::pin(crate::handlers::assistant_team::after_turn(
            &fixture.state,
            &row,
            &result.text,
            None,
        ))
        .await;
        let wire = if wake {
            let sends = platform.received_requests().await.unwrap();
            assert_eq!(sends.len(), if enabled { 2 } else { 1 });
            assert_eq!(
                serde_json::from_slice::<Value>(&sends.last().unwrap().body).unwrap()["text"],
                result.text
            );
            if enabled {
                Box::pin(crate::handlers::assistant_team::after_turn(
                    &fixture.state,
                    &row,
                    &result.text,
                    None,
                ))
                .await;
                assert_eq!(platform.received_requests().await.unwrap().len(), 2);
                assert_eq!(audit.count_documents(saved_repeat_audit).await.unwrap(), 1);
            }
            sends.last().unwrap().body.clone()
        } else {
            assert!(
                crate::services::channel_turn_delivery::detach(db, &fixture.event.id)
                    .await
                    .is_ok()
            );
            Box::pin(late_delivery::process(&fixture.state, &fixture.event.id)).await;
            let sends = sends(&fixture).await;
            assert_eq!(sends.len(), if enabled { 2 } else { 1 });
            assert!(String::from_utf8_lossy(&sends.last().unwrap().body).contains(&result.text));
            sends.last().unwrap().body.clone()
        };
        let actual = (reply, conversation, queued_event, wire);
        if let Some(expected) = baseline.take() {
            assert_eq!(actual, expected)
        } else {
            baseline = Some(actual)
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn saved_wake_recovers_the_unsent_suffix_and_preserves_uncertain_sends() {
    for uncertain in [false, true] {
        Box::pin(saved_wake_delivery_race(uncertain, false)).await;
    }
}

#[tokio::test]
async fn saved_wake_recovers_while_the_predecessor_is_still_alive() {
    Box::pin(saved_wake_delivery_race(false, true)).await;
}

async fn saved_wake_delivery_race(uncertain: bool, overlapping: bool) {
    use crate::services::{channel_text_delivery as journal, durable_wake_tests as crash};
    let (fixture, platform, mut row) = Box::pin(result_fixture(ChronoDuration::minutes(1))).await;
    mock_platform(&platform, "confirmed").await;
    let origin = fixture.event.delivery.as_ref().unwrap().origin.clone();
    let turn = row.active_turn.as_mut().unwrap();
    let mut event = crate::services::assistant_team_service::event("result", "done".into(), None);
    event.reply_to.push(origin.clone());
    event.source_event_ids.push(fixture.event.id.clone());
    turn.events.push(event);
    let target = sources::SavedDestination {
        source: sources::ResultSource::capture(turn, &origin),
        origin: origin.clone(),
        additional: true,
    };
    let key = sources::saved_key(&turn.turn_id, &origin);
    let db = &fixture.state.db;
    journal::ensure_indexes(db).await.unwrap();
    let saved = result_message(&fixture, &row).await;
    let reply = saved.get_str("_id").unwrap();
    let messages =
        db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME);
    messages.update_one(doc! {"_id":reply},doc! {"$set":{"text":"x".repeat(5000),"handoff_delivery":{"agent_id":&row.agent_id,"targets":[bson::to_bson(&target).unwrap()]},"handoff_delivery_due_at":bson::DateTime::from_millis(0)}}).await.unwrap();
    db.collection::<bson::Document>(THREADS).update_one(doc! {"channel_id":&fixture.channel.id},doc! {"$set":{"last_message_id":"newer-inbound","last_relay_message_id":"newer-inbound"}}).await.unwrap();
    let site = if uncertain {
        "text_part_provider_accepted"
    } else {
        "text_suffix_prepared"
    };
    let original = Box::pin(handoff_delivery::deliver(&fixture.state, reply));
    if overlapping {
        crash::resume_after(site, original, async {
            let before = messages
                .find_one(doc! {"_id":reply})
                .await
                .unwrap()
                .unwrap();
            messages
                .update_one(
                    doc! {"_id":reply},
                    doc! {"$set":{"handoff_delivery_due_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            Box::pin(handoff_delivery::deliver(&fixture.state, reply))
                .await
                .unwrap();
            let after = messages
                .find_one(doc! {"_id":reply})
                .await
                .unwrap()
                .unwrap();
            assert_ne!(
                before.get_str("handoff_delivery_lease").unwrap(),
                after.get_str("handoff_delivery_lease").unwrap()
            );
        })
        .await;
        let receipt = journal::load(db, &key).await.unwrap().unwrap();
        assert_eq!(
            receipt.parts[0].state,
            crate::models::channel_text_delivery::TextPartState::Sent
        );
        assert_eq!(
            receipt.parts[1].state,
            crate::models::channel_text_delivery::TextPartState::Pending
        );
    } else {
        crash::drop_after(site, original).await;
    }
    assert_eq!(platform.received_requests().await.unwrap().len(), 1);
    messages
        .update_one(
            doc! {"_id":reply},
            doc! {"$set":{"handoff_delivery_due_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
        .update_one(
            doc! {"logical_key":&key,"parts.1.preparation.version":1},
            doc! {"$set":{"parts.1.preparation.lease_until":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        Box::pin(handoff_delivery::deliver(&fixture.state, reply)),
        Box::pin(handoff_delivery::deliver(&fixture.state, reply))
    );
    first.unwrap();
    second.unwrap();
    let sent = platform.received_requests().await.unwrap();
    assert_eq!(sent.len(), if uncertain { 1 } else { 2 });
    for request in sent {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reply_to_message_id"], 17);
    }
    let receipt = journal::load(db, &key).await.unwrap().unwrap();
    assert_eq!(receipt.parts.len(), 2);
    use crate::models::channel_text_delivery::TextPartState;
    assert_eq!(
        receipt.parts[0].state,
        if uncertain {
            TextPartState::Sending
        } else {
            TextPartState::Sent
        }
    );
    assert_eq!(
        receipt.parts[1].state,
        if uncertain {
            TextPartState::Pending
        } else {
            TextPartState::Sent
        }
    );
    db.drop().await.unwrap();
}

async fn result_message(f: &Fixture, row: &AssistantConversation) -> bson::Document {
    f.state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"conversation_id":&row.id,"turn_id":&row.active_turn.as_ref().unwrap().turn_id,"role":"assistant"}).await.unwrap().unwrap()
}

fn result_reason(audit: &bson::Document) -> Option<&str> {
    audit
        .get_document("event_data")
        .unwrap()
        .get_str("reason")
        .ok()
}

async fn assert_result_once(age: ChronoDuration) {
    let (f, platform, row) = Box::pin(result_fixture(age)).await;
    mock_platform(&platform, "confirmed").await;
    result_hook(&f, &row).await;
    let requests = platform.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["text"], "Useful answer");
    assert_eq!(body["chat_id"], "123");
    assert_eq!(body["reply_to_message_id"], 17);
    assert_eq!(body["message_thread_id"], 77);
    let audits = audit_decisions(&f).await;
    assert_eq!(audits.len(), 1);
    let data = audits[0].get_document("event_data").unwrap();
    assert_eq!(data.get_str("egress"), Ok("local_relay"));
    assert_eq!(data.get_str("outcome"), Ok("confirmed"));
    assert_eq!(
        result_message(&f, &row)
            .await
            .get_array("supplemental_result_claims")
            .unwrap()
            .len(),
        1
    );
    no_event_replies(&f).await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn supplemental_result_window_unbound_at_30_minutes() {
    Box::pin(assert_result_once(ChronoDuration::minutes(30))).await;
}
#[tokio::test]
async fn supplemental_result_window_unbound_at_23_hours_59_minutes() {
    Box::pin(assert_result_once(ChronoDuration::minutes(1439))).await;
}
#[tokio::test]
async fn supplemental_result_window_at_and_past_24_hours_skips() {
    Box::pin(async {
        for age in [ChronoDuration::hours(24), ChronoDuration::hours(25)] {
            let (f, platform, row) = result_fixture(age).await;
            result_hook(&f, &row).await;
            assert!(platform.received_requests().await.unwrap().is_empty());
            assert_eq!(
                result_reason(&audit_decisions(&f).await[0]),
                Some("anchor_expired")
            );
            assert!(
                !result_message(&f, &row)
                    .await
                    .contains_key("supplemental_result_claims")
            );
            f.state.db.drop().await.unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn supplemental_result_window_expired_absent_and_undecryptable_reference() {
    Box::pin(async {
        for reference in ["expired","absent","undecryptable"] {
            let (f,platform,row)=result_fixture(ChronoDuration::minutes(30)).await;
            mock_platform(&platform,"confirmed").await;
            let fields=match reference {
                "absent"=>doc! {"event_ref_ciphertext":bson::Bson::Null,"event_ref_expires_at":bson::Bson::Null,"last_message_id":bson::Bson::Null},
                "undecryptable"=>doc! {"event_ref_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2,3]}},
                _=>doc! {},
            };
            f.state.db.collection::<bson::Document>(THREADS).update_one(doc! {"channel_id":&f.channel.id},doc! {"$set":fields}).await.unwrap();
            result_hook(&f,&row).await;
            assert_eq!(platform.received_requests().await.unwrap().len(),1,"{reference}");
            no_event_replies(&f).await;
            f.state.db.drop().await.unwrap();
        }
    }).await;
}

#[tokio::test]
async fn supplemental_result_window_expired_fallback_intact_or_stale_digest() {
    Box::pin(async {
        for stale in [false, true] {
            let (f, platform, row) = result_fixture(ChronoDuration::minutes(30)).await;
            mock_platform(&platform, "confirmed").await;
            let mut fields = doc! {"last_relay_message_id":bson::Bson::Null};
            if stale {
                fields.insert("event_ref_relay_binding", "stale");
            }
            f.state
                .db
                .collection::<bson::Document>(THREADS)
                .update_one(doc! {"channel_id":&f.channel.id}, doc! {"$set":fields})
                .await
                .unwrap();
            result_hook(&f, &row).await;
            assert_eq!(
                platform.received_requests().await.unwrap().len(),
                usize::from(!stale)
            );
            let audits = audit_decisions(&f).await;
            if stale {
                assert_eq!(result_reason(&audits[0]), Some("stale_reference_anchor"));
            } else {
                assert_eq!(
                    audits[0]
                        .get_document("event_data")
                        .unwrap()
                        .get_str("anchor"),
                    Ok("chat_reference_event")
                );
            }
            f.state.db.drop().await.unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn supplemental_result_window_source_bound_additional_at_30_minutes() {
    Box::pin(async {
        let (f, platform, mut row) = result_fixture(ChronoDuration::minutes(30)).await;
        mock_platform(&platform, "confirmed").await;
        let origin = f.event.delivery.as_ref().unwrap().origin.clone();
        let mut event =
            crate::services::assistant_team_service::event("result", "done".into(), None);
        event.reply_to = vec![origin.clone()];
        event.source_event_ids = vec![f.event.id.clone()];
        row.active_turn.as_mut().unwrap().events = vec![event];
        deliver_to_for(
            &f.state,
            &row,
            &origin,
            "Useful answer",
            None,
            Purpose::TeamAdditionalResult,
        )
        .await;
        assert_eq!(platform.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            audit_decisions(&f).await[0]
                .get_document("event_data")
                .unwrap()
                .get_str("anchor"),
            Ok("event_source")
        );
        no_event_replies(&f).await;
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_result_window_progress_keeps_all_three_original_windows() {
    Box::pin(async {
        for (purpose, name) in [
            (Purpose::CheckingNotice, "checking_notice"),
            (Purpose::CapacityNotice, "capacity_notice"),
            (Purpose::MachineDesktopNotice, "machine_desktop_notice"),
        ] {
            let (f, platform, row) = result_fixture(ChronoDuration::minutes(30)).await;
            deliver_to_for(
                &f.state,
                &row,
                row.channel.as_ref().unwrap(),
                "progress",
                None,
                purpose,
            )
            .await;
            assert!(platform.received_requests().await.unwrap().is_empty());
            let audits = audit_decisions(&f).await;
            assert_eq!(result_reason(&audits[0]), Some("reference_expired"));
            assert_eq!(
                audits[0]
                    .get_document("event_data")
                    .unwrap()
                    .get_str("purpose"),
                Ok(name)
            );
            assert!(
                !result_message(&f, &row)
                    .await
                    .contains_key("supplemental_result_claims")
            );
            f.state.db.drop().await.unwrap();
        }
    })
    .await;
}

async fn repeated_result(concurrent: bool, also: bool) {
    let (f, platform, row) = Box::pin(result_fixture(ChronoDuration::minutes(30))).await;
    mock_platform(&platform, "confirmed").await;
    if also {
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .update_one(
                doc! {"conversation_id":&row.id,"turn_id":&row.active_turn.as_ref().unwrap().turn_id,"role":"assistant"},
                doc! {"$set":{"post_turn_deliver_also":[bson::to_bson(row.channel.as_ref().unwrap()).unwrap()]}},
            )
            .await
            .unwrap();
        result_hook(&f, &row).await;
    } else if concurrent {
        tokio::join!(result_hook(&f, &row), result_hook(&f, &row));
    } else {
        result_hook(&f, &row).await;
        // A refreshed retained pointer is not a new logical destination.
        let mut inbound = f
            .state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .find_one(doc! {"_id":&f.event.relay_message_id})
            .await
            .unwrap()
            .unwrap();
        inbound.insert("_id", Uuid::new_v4().to_string());
        let id = inbound.get_str("_id").unwrap().to_owned();
        f.state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(inbound)
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(THREADS)
            .update_one(
                doc! {"channel_id":&f.channel.id},
                doc! {"$set":{"last_relay_message_id":id}},
            )
            .await
            .unwrap();
        result_hook(&f, &row).await;
    }
    assert_eq!(platform.received_requests().await.unwrap().len(), 1);
    let audits = audit_decisions(&f).await;
    assert_eq!(audits.len(), 2);
    assert_eq!(
        audits
            .iter()
            .filter(|a| result_reason(a) == Some("already_claimed"))
            .count(),
        1
    );
    assert_eq!(
        result_message(&f, &row)
            .await
            .get_array("supplemental_result_claims")
            .unwrap()
            .len(),
        1
    );
    no_event_replies(&f).await;
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn supplemental_result_claim_hook_twice_sequentially() {
    Box::pin(repeated_result(false, false)).await;
}
#[tokio::test]
async fn supplemental_result_claim_hook_twice_concurrently() {
    Box::pin(repeated_result(true, false)).await;
}
#[tokio::test]
async fn supplemental_result_claim_primary_and_additional_share_destination() {
    Box::pin(repeated_result(false, true)).await;
}

#[tokio::test]
async fn supplemental_result_claim_rejected_and_timed_out_attempts_never_retry() {
    Box::pin(async {
        for timeout in [false,true] {
            let (f,platform,row)=result_fixture(ChronoDuration::minutes(30)).await;
            let response=if timeout { ResponseTemplate::new(200).set_delay(Duration::from_secs(21)).set_body_json(json!({"ok":true,"result":{"message_id":99}})) }
                else { ResponseTemplate::new(400).set_body_json(json!({"ok":false,"error_code":400,"description":"Bad Request: message refused"})) };
            Mock::given(path("/bottoken/sendMessage")).respond_with(response).mount(&platform).await;
            let before=result_message(&f,&row).await;
            result_hook(&f,&row).await;
            result_hook(&f,&row).await;
            assert_eq!(platform.received_requests().await.unwrap().len(),1);
            let audits=audit_decisions(&f).await;
            assert_eq!(audits.len(),2);
            assert!(audits.iter().any(|a|a.get_document("event_data").unwrap().get_str("outcome")==Ok(if timeout {"unknown"}else{"rejected"})));
            assert_eq!(audits.iter().filter(|a|result_reason(a)==Some("already_claimed")).count(),1);
            let mut after=result_message(&f,&row).await;
            after.remove("supplemental_result_claims");
            assert_eq!(before,after);
            f.state.db.drop().await.unwrap();
        }
    }).await;
}

#[tokio::test]
async fn supplemental_result_claim_database_error_and_missing_reply_send_nothing() {
    Box::pin(async {
        for missing in [false,true] {
            let (f,platform,row)=result_fixture(ChronoDuration::minutes(30)).await;
            mock_platform(&platform,"confirmed").await;
            let collection=crate::models::assistant_message::COLLECTION_NAME;
            if missing { f.state.db.collection::<bson::Document>(collection).delete_many(doc! {}).await.unwrap(); }
            else { f.state.db.run_command(doc! {"collMod":collection,"validator":{"supplemental_result_claims":{"$exists":false}},"validationLevel":"strict","validationAction":"error"}).await.unwrap(); }
            result_hook(&f,&row).await;
            assert!(platform.received_requests().await.unwrap().is_empty());
            assert_eq!(result_reason(&audit_decisions(&f).await[0]),Some("source_unavailable"));
            assert_eq!(f.state.db.collection::<bson::Document>(collection).count_documents(doc! {"supplemental_result_claims":{"$exists":true}}).await.unwrap(),0);
            if !missing {
                f.state.db.run_command(doc! {"collMod":collection,"validator":{}}).await.unwrap();
                result_hook(&f,&row).await;
                assert_eq!(platform.received_requests().await.unwrap().len(),1);
            }
            f.state.db.drop().await.unwrap();
        }
    }).await;
}

#[tokio::test]
async fn supplemental_result_claim_two_turns_and_private_legacy_field() {
    Box::pin(async {
        let (f, platform, mut row) = result_fixture(ChronoDuration::minutes(30)).await;
        mock_platform(&platform, "confirmed").await;
        let legacy = result_message(&f, &row).await;
        let model: crate::models::assistant_message::AssistantMessage =
            bson::from_document(legacy).unwrap();
        assert!(model.supplemental_result_claims.is_empty());
        assert!(
            !bson::to_document(&model)
                .unwrap()
                .contains_key("supplemental_result_claims")
        );
        let baseline = serde_json::to_value(
            crate::handlers::assistant_nyxagent::MessageResponse::from(model),
        )
        .unwrap();
        result_hook(&f, &row).await;
        let saved = result_message(&f, &row).await;
        let model: crate::models::assistant_message::AssistantMessage =
            bson::from_document(saved.clone()).unwrap();
        assert!(!format!("{model:?}").contains(&model.supplemental_result_claims[0]));
        let response = serde_json::to_value(
            crate::handlers::assistant_nyxagent::MessageResponse::from(model),
        )
        .unwrap();
        assert_eq!(response, baseline);
        let api = Box::pin(crate::handlers::assistant_nyxagent::history(
            State(f.state.clone()),
            crate::handlers::assistant_team::owner_auth(OWNER).unwrap(),
            Path(row.id.clone()),
            axum::extract::Query(Default::default()),
        ))
        .await
        .unwrap()
        .0;
        let api = serde_json::to_value(api).unwrap();
        assert_eq!(api["messages"][0], baseline);
        assert!(!api.to_string().contains("supplemental_result_claims"));
        assert!(
            audit_decisions(&f)
                .await
                .iter()
                .all(|a| !a.to_string().contains("supplemental_result_claims"))
        );
        let mut second = saved;
        let turn = Uuid::new_v4().to_string();
        second.insert("_id", Uuid::new_v4().to_string());
        second.insert("turn_id", &turn);
        second.insert("seq", 2_i64);
        second.remove("supplemental_result_claims");
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(second)
            .await
            .unwrap();
        row.active_turn.as_mut().unwrap().turn_id = turn;
        result_hook(&f, &row).await;
        assert_eq!(platform.received_requests().await.unwrap().len(), 2);
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_result_window_images_precede_text_and_are_not_resent() {
    Box::pin(async {
        let (f, platform, row) = result_fixture(ChronoDuration::minutes(30)).await;
        mock_platform(&platform, "confirmed").await;
        mock_sends(&platform, "telegram", false, Duration::ZERO).await;
        add_images(&f).await;
        result_hook(&f, &row).await;
        result_hook(&f, &row).await;
        let requests = platform.received_requests().await.unwrap();
        let sends: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.url.path().ends_with("sendPhoto") || r.url.path().ends_with("sendMessage")
            })
            .collect();
        assert_eq!(sends.len(), 3);
        assert!(sends[0].url.path().ends_with("sendPhoto"));
        assert!(sends[1].url.path().ends_with("sendPhoto"));
        assert!(sends[2].url.path().ends_with("sendMessage"));
        no_event_replies(&f).await;
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_result_window_late_authority_guest_org_key_and_relink() {
    Box::pin(async {
        for mode in ["guest","org","guest_revoked","org_revoked","route_revoked","platform_manager","relinked","gateway_key_missing"] {
            let (f,platform,mut row)=Box::pin(notice_fixture()).await;
            age_result_anchor(&f,ChronoDuration::minutes(30)).await;
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
            if mode=="gateway_key_missing" { f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id":&f.channel.id},doc! {"$set":{"agent_key_ciphertext":bson::Bson::Null}}).await.unwrap(); }
            if mode=="relinked" { f.state.db.collection::<bson::Document>(THREADS).update_one(doc! {"channel_id":&f.channel.id},doc! {"$set":{"conversation_id":"different-conversation"}}).await.unwrap(); }
            deliver_to_for(&f.state,&row,&f.event.delivery.as_ref().unwrap().origin,"Useful answer",None,Purpose::TeamResult).await;
            assert_eq!(platform.received_requests().await.unwrap().len(),usize::from(matches!(mode,"guest"|"org")),"{mode}");
            no_event_replies(&f).await;
            assert_eq!(status(&f).await,"waiting");
            if mode=="guest" { assert!(gateway.final_reply(&f,"guest answer").await); }
            let saved=result_message(&f,&row).await;
            assert_eq!(saved.contains_key("supplemental_result_claims"),matches!(mode,"guest"|"org"),"{mode}");
            f.state.db.drop().await.unwrap();
        }
    }).await;
}

#[tokio::test]
async fn supplemental_result_claim_interrupted_before_dispatch_is_not_reclaimed() {
    Box::pin(async {
        let (f, platform, row) = result_fixture(ChronoDuration::minutes(30)).await;
        let claim = sources::claim_result(
            &f.state.db,
            OWNER,
            &row.id,
            row.active_turn.as_ref().map(|turn| turn.turn_id.as_str()),
            row.channel.as_ref().unwrap(),
        )
        .await
        .unwrap();
        assert!(claim.is_ok());
        result_hook(&f, &row).await;
        assert!(platform.received_requests().await.unwrap().is_empty());
        assert_eq!(
            result_reason(&audit_decisions(&f).await[0]),
            Some("already_claimed")
        );
        assert_eq!(
            result_message(&f, &row)
                .await
                .get_array("supplemental_result_claims")
                .unwrap()
                .len(),
            1
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_result_window_source_bound_expiry_is_anchor_expired() {
    Box::pin(async {
        let (f, platform, mut row) = result_fixture(ChronoDuration::hours(24)).await;
        row.active_turn.as_mut().unwrap().channel_event_id = Some(f.event.id.clone());
        patch(
            &f,
            doc! {"expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))},
        )
        .await;
        deliver_to_for(
            &f.state,
            &row,
            row.channel.as_ref().unwrap(),
            "late answer",
            None,
            Purpose::TeamAdditionalResult,
        )
        .await;
        assert!(platform.received_requests().await.unwrap().is_empty());
        assert_eq!(
            result_reason(&audit_decisions(&f).await[0]),
            Some("anchor_expired")
        );
        assert!(
            !result_message(&f, &row)
                .await
                .contains_key("supplemental_result_claims")
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn supplemental_result_preparation_failure_does_not_claim() {
    Box::pin(async {
        let (f, platform, row) = result_fixture(ChronoDuration::minutes(30)).await;
        mock_platform(&platform, "confirmed").await;
        let bots = f.state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME);
        let bot = bots.find_one(doc! {"_id":&f.channel.channel_bot_id}).await.unwrap().unwrap();
        bots.update_one(doc! {"_id":&f.channel.channel_bot_id}, doc! {"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2,3]}}}).await.unwrap();
        result_hook(&f, &row).await;
        assert!(platform.received_requests().await.unwrap().is_empty());
        assert!(!result_message(&f, &row).await.contains_key("supplemental_result_claims"));
        assert_eq!(result_reason(&audit_decisions(&f).await[0]), Some("source_unavailable"));
        bots.update_one(doc! {"_id":&f.channel.channel_bot_id}, doc! {"$set":{"bot_token_encrypted":bot.get("bot_token_encrypted").unwrap()}}).await.unwrap();
        result_hook(&f, &row).await;
        assert_eq!(platform.received_requests().await.unwrap().len(), 1);
        f.state.db.drop().await.unwrap();
    }).await;
}

#[path = "nyxbot_thread_reference_tests.rs"]
mod thread_reference_tests;
