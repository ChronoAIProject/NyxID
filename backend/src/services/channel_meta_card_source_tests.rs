use super::*;
use crate::{
    models::assistant_conversation::{ChannelTurnContext, TurnOrigin},
    services::{assistant_acknowledgement_service as acks, assistant_nyxagent as engine},
};

pub(super) fn context(bot: &ChannelBot) -> ChannelTurnContext {
    bson::from_document(doc! {"meta_grant":bson::to_bson(&scopes::reference(bot).unwrap()).unwrap(),
        "source_event_id":uuid::Uuid::new_v4().to_string(),"channel_id":uuid::Uuid::new_v4().to_string(),"channel_bot_id":&bot.id,
        "conversation_kind":"private","sender_kind":"human","guest":false,"original_message":true}).unwrap()
}
pub(super) async fn begin(
    f: &Fixture,
    home: Option<&str>,
    meta: Option<ChannelTurnContext>,
    event: bool,
) -> AssistantConversation {
    let request: engine::TurnRequest = serde_json::from_value(
        serde_json::json!({"conversation_id":home,"text":"Captured question"}),
    )
    .unwrap();
    let mut start = engine::TurnStart::from(&request);
    if meta.is_some() {
        start.origin = TurnOrigin::Channel;
        start.channel_context = meta;
    }
    if event {
        start.origin = TurnOrigin::Event;
    }
    engine::begin_turn(&f.state.db, &f.bot.user_id, start, &f.state.encryption_keys)
        .await
        .unwrap()
}
fn request(summary: &str) -> acks::Request<'_> {
    acks::Request {
        kind: "action",
        service: None,
        tool: Some("private_operation"),
        arguments: None,
        summary,
        platform: false,
    }
}
async fn erase(f: &Fixture) {
    let db = &f.state.db;
    // Provider removal is already confirmed; exercise the actual scoped
    // worker through exact durable completion without external Graph calls.
    db.collection::<Document>(BOTS).update_one(doc! {"_id":&f.bot.id},doc! {"$set":{
        "is_active":false,"meta_binding.subscription":"removed","meta_binding.provider_revoked":true}}).await.unwrap();
    let service = MetaService::from(&f.state);
    let cutoff = bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::seconds(1));
    for _ in 0..80 {
        if pass(
            &f.state,
            &service,
            &f.bot.id,
            &scopes::reference(&f.bot).unwrap().grant_id,
            cutoff,
        )
        .await
        .unwrap()
        {
            assert!(
                grants::completion_for(db, &f.bot.id, &scopes::reference(&f.bot).unwrap().grant_id)
                    .await
                    .unwrap()
            );
            return;
        }
    }
    panic!("grant cleanup did not complete");
}

#[tokio::test]
async fn delayed_meta_action_cannot_borrow_later_web_turn_but_ordinary_action_keeps_legacy_behavior()
 {
    for meta in [true, false] {
        let f = fixture().await;
        let db = &f.state.db;
        engine::ensure_indexes(db).await.unwrap();
        let original = begin(&f, None, meta.then(|| context(&f.bot)), false).await;
        let chat = crate::services::assistant_authority_tests::chat_for_row(
            &f.state,
            &f.bot.user_id,
            &original,
        )
        .await;
        finish_source_turn(&f.state, &original, "Original answer").await;
        let web = begin(&f, Some(&original.id), None, false).await;
        assert_eq!(original.credential_api_key_id, web.credential_api_key_id);
        let result = acks::request(db, &chat, request("Delayed original summary")).await;
        if meta {
            assert!(result.is_err());
            assert_eq!(
                db.collection::<Document>(
                    crate::models::assistant_acknowledgement::COLLECTION_NAME
                )
                .count_documents(doc! {"summary":"Delayed original summary"})
                .await
                .unwrap(),
                0
            );
        } else {
            assert_eq!(
                result.unwrap().creation_turn_id,
                original
                    .active_turn
                    .as_ref()
                    .map(|turn| turn.turn_id.clone())
            );
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn queued_b_card_uses_exact_event_input_and_survives_a_erasure() {
    let f = fixture().await;
    let db = &f.state.db;
    engine::ensure_indexes(db).await.unwrap();
    let mut b = f.bot.clone();
    b.id = uuid::Uuid::new_v4().to_string();
    b.platform_bot_id = "444".into();
    b.meta_binding.as_mut().unwrap().grant_id = uuid::Uuid::new_v4().to_string();
    db.collection::<ChannelBot>(BOTS)
        .insert_one(&b)
        .await
        .unwrap();
    transaction_service::run_db_only(db, &(db, &b), |tx, &(db, b)| {
        Box::pin(scopes::ensure_current(db, tx, b, true))
    })
    .await
    .unwrap();
    let a = begin(&f, None, Some(context(&f.bot)), false).await;
    let mut event = crate::services::assistant_team_service::event(
        "message",
        "Independent B question".into(),
        None,
    );
    event.channel_message = Some("Independent B question".into());
    event.channel_contexts = vec![context(&b)];
    engine::push_events(db, &f.bot.user_id, &a.id, vec![event])
        .await
        .unwrap()
        .unwrap();
    finish_source_turn(&f.state, &a, "A answer").await;
    let b_turn = begin(&f, Some(&a.id), None, true).await;
    let chat =
        crate::services::assistant_authority_tests::chat_for_row(&f.state, &f.bot.user_id, &b_turn)
            .await;
    let card = acks::request(db, &chat, request("B pending action"))
        .await
        .unwrap();
    assert_eq!(
        card.requested_turn_id,
        b_turn.active_turn.as_ref().map(|turn| turn.turn_id.clone())
    );
    assert_eq!(card.creation_turn_id, card.requested_turn_id);
    crate::services::assistant_service_execution::publish_card_binding(db,&chat,&card.id,doc! {"kind":"card_binding","user_id":&f.bot.user_id,"conversation_id":&a.id,"turn_id":&card.creation_turn_id}).await.unwrap();
    finish_source_turn(&f.state, &b_turn, "B awaiting approval").await;
    erase(&f).await;
    let decided = acks::decide(db, &f.bot.user_id, &a.id, &card.id, true)
        .await
        .unwrap();
    assert_eq!(decided.status, "allowed");
    assert!(
        db.collection::<Document>(crate::services::assistant_service_execution::COLLECTION)
            .find_one(doc! {"_id":&card.id})
            .await
            .unwrap()
            .is_some()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn denied_meta_card_continuation_keeps_proof_and_cannot_publish_after_erasure() {
    let f = fixture().await;
    let db = &f.state.db;
    engine::ensure_indexes(db).await.unwrap();
    let original = begin(&f, None, Some(context(&f.bot)), false).await;
    let chat = crate::services::assistant_authority_tests::chat_for_row(
        &f.state,
        &f.bot.user_id,
        &original,
    )
    .await;
    let card = acks::request(db, &chat, request("Private denied action summary"))
        .await
        .unwrap();
    acks::decide(db, &f.bot.user_id, &original.id, &card.id, false)
        .await
        .unwrap();
    finish_source_turn(&f.state, &original, "Awaiting decision").await;
    let independent = crate::services::assistant_team_service::event(
        "independent_notice",
        "Independent notice".into(),
        None,
    );
    engine::push_events(db, &f.bot.user_id, &original.id, vec![independent.clone()])
        .await
        .unwrap()
        .unwrap();
    assert!(
        acks::source::publish_owner_denial(db, &f.bot.user_id, &card.id, &original.id)
            .await
            .unwrap()
    );
    let continued = begin(&f, Some(&original.id), None, true).await;
    let active = continued.active_turn.as_ref().unwrap();
    assert_eq!(active.events.len(), 1);
    assert_eq!(active.events[0].kind, "action_denied");
    let input = db
        .collection::<Document>("assistant_messages")
        .find_one(doc! {"conversation_id":&original.id,"turn_id":&active.turn_id,"role":"event"})
        .await
        .unwrap()
        .unwrap();
    assert!(
        input
            .get_str("text")
            .unwrap()
            .contains("Private denied action summary")
    );
    assert!(input.get_array("meta_grants").is_ok());
    let denial_chat = crate::services::assistant_authority_tests::chat_for_row(
        &f.state,
        &f.bot.user_id,
        &continued,
    )
    .await;
    let repeated = acks::request(db, &denial_chat, request("Private denied action summary"))
        .await
        .unwrap();
    assert_eq!(repeated.id, card.id);
    assert_eq!(repeated.status, "denied");
    finish_source_turn(&f.state, &continued, "Explained private denied action").await;
    erase(&f).await;
    assert!(
        !acks::source::publish_owner_denial(db, &f.bot.user_id, &card.id, &original.id)
            .await
            .unwrap()
    );
    let parent = engine::get(db, &f.bot.user_id, &original.id).await.unwrap();
    assert!(
        parent
            .pending_events
            .iter()
            .any(|event| event.id == independent.id)
    );
    assert!(
        !parent
            .pending_events
            .iter()
            .any(|event| event.kind == "action_denied")
    );
    assert_eq!(
        db.collection::<Document>("assistant_messages")
            .count_documents(doc! {"conversation_id":&original.id})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn explicit_web_retry_collecting_a_prior_denial_can_request_a_new_decision() {
    let f = fixture().await;
    let db = &f.state.db;
    let original = begin(&f, None, None, false).await;
    let chat = crate::services::assistant_authority_tests::chat_for_row(
        &f.state,
        &f.bot.user_id,
        &original,
    )
    .await;
    let denied = acks::request(db, &chat, request("Review operation"))
        .await
        .unwrap();
    acks::decide(db, &f.bot.user_id, &original.id, &denied.id, false)
        .await
        .unwrap();
    assert!(
        acks::source::publish_owner_denial(db, &f.bot.user_id, &denied.id, &original.id)
            .await
            .unwrap()
    );
    finish_source_turn(&f.state, &original, "Decision recorded").await;
    let retry = begin(&f, Some(&original.id), None, false).await;
    let turn = retry.active_turn.as_ref().unwrap();
    assert!(
        turn.events
            .iter()
            .any(|event| event.kind == "action_denied")
    );
    let chat =
        crate::services::assistant_authority_tests::chat_for_row(&f.state, &f.bot.user_id, &retry)
            .await;
    let pending = acks::request(db, &chat, request("Review operation"))
        .await
        .unwrap();
    assert_eq!(pending.status, "pending");
    assert_ne!(pending.id, denied.id);
    assert_eq!(pending.requested_turn_id.as_ref(), Some(&turn.turn_id));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn erased_captured_input_cannot_be_recreated_after_public_reset_but_retained_web_input_can_delay()
 {
    for meta in [true, false] {
        for later in [true, false] {
            let f = fixture().await;
            let db = &f.state.db;
            engine::ensure_indexes(db).await.unwrap();
            crate::services::feature_flag_service::set_platform_override(
                db,
                crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
                &crate::services::feature_flag_service::FlagTarget::User(f.bot.user_id.clone()),
                true,
                &f.bot.user_id,
            )
            .await
            .unwrap();
            let original = begin(&f, None, meta.then(|| context(&f.bot)), false).await;
            let captured = crate::services::assistant_authority_tests::chat_for_row(
                &f.state,
                &f.bot.user_id,
                &original,
            )
            .await;
            finish_source_turn(&f.state, &original, "Original answer").await;
            if meta {
                erase(&f).await;
            }
            let reset = crate::handlers::assistant_nyxagent::reset_context(
                axum::extract::State(f.state.clone()),
                crate::test_utils::test_auth_user(&f.bot.user_id),
                axum::extract::Path(original.id.clone()),
            )
            .await
            .unwrap();
            assert_eq!(reset.reset_reason.as_deref(), Some("user_reset"));
            assert_eq!(
                engine::get(db, &f.bot.user_id, &original.id)
                    .await
                    .unwrap()
                    .context_reset_reason
                    .as_deref(),
                Some("user_reset")
            );
            if later {
                begin(&f, Some(&original.id), None, false).await;
            }
            let result = acks::request(
                db,
                &captured,
                request("Captured private summary after reset"),
            )
            .await;
            assert_eq!(result.is_ok(), !meta);
            if meta {
                assert_eq!(
                    db.collection::<Document>(
                        crate::models::assistant_acknowledgement::COLLECTION_NAME
                    )
                    .count_documents(doc! {"summary":"Captured private summary after reset"})
                    .await
                    .unwrap(),
                    0
                );
            }
            db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn ordinary_event_retains_its_prior_meta_request_source_and_fences_each_grant_once() {
    let f = fixture().await;
    let db = &f.state.db;
    engine::ensure_indexes(db).await.unwrap();
    let original = begin(&f, None, Some(context(&f.bot)), false).await;
    let chat = crate::services::assistant_authority_tests::chat_for_row(
        &f.state,
        &f.bot.user_id,
        &original,
    )
    .await;
    let before = db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":&f.bot.id})
        .await
        .unwrap()
        .unwrap()
        .get_i64("meta_observation_revision")
        .unwrap_or(0);
    acks::request(db, &chat, request("Ordinary direct Meta action"))
        .await
        .unwrap();
    let after = db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":&f.bot.id})
        .await
        .unwrap()
        .unwrap()
        .get_i64("meta_observation_revision")
        .unwrap_or(0);
    assert_eq!(
        after - before,
        1,
        "one fresh source fence per distinct grant"
    );
    finish_source_turn(&f.state, &original, "Original answer").await;
    let event = crate::services::assistant_team_service::event(
        "channel_owner_verified",
        "Owner verified".into(),
        None,
    );
    engine::push_events(db, &f.bot.user_id, &original.id, vec![event])
        .await
        .unwrap()
        .unwrap();
    let ordinary = begin(&f, Some(&original.id), None, true).await;
    let chat = crate::services::assistant_authority_tests::chat_for_row(
        &f.state,
        &f.bot.user_id,
        &ordinary,
    )
    .await;
    // Different operation arguments keep this independent from the pending
    // direct card above while exercising the ordinary event's request source.
    let args = serde_json::json!({"notice":true});
    let mut asked = request("Action after owner verification");
    asked.arguments = Some(&args);
    let card = acks::request(db, &chat, asked).await.unwrap();
    assert_eq!(
        card.requested_turn_id,
        original
            .active_turn
            .as_ref()
            .map(|turn| turn.turn_id.clone())
    );
    assert_eq!(card.meta_contexts.len(), 1);
    acks::decide(db, &f.bot.user_id, &ordinary.id, &card.id, false)
        .await
        .unwrap();
    finish_source_turn(&f.state, &ordinary, "Awaiting owner decision").await;
    assert!(
        acks::source::publish_owner_denial(db, &f.bot.user_id, &card.id, &ordinary.id)
            .await
            .unwrap()
    );
    let continued = begin(&f, Some(&ordinary.id), None, true).await;
    assert_eq!(
        continued.active_turn.as_ref().unwrap().events[0].kind,
        "action_denied"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_denial_pruning_preserves_unresolved_proof_then_admits_healthy_b() {
    let f = fixture().await;
    let db = &f.state.db;
    engine::ensure_indexes(db).await.unwrap();
    let mut b = crate::services::channel_adapters::meta::tests::registered_bot(
        &f.state,
        &f.bot.user_id,
        "instagram",
    )
    .await;
    b.platform_bot_id = "444".into();
    db.collection::<ChannelBot>(BOTS)
        .replace_one(doc! {"_id":&b.id}, &b)
        .await
        .unwrap();
    transaction_service::run_db_only(db, &(db, &b), |tx, &(db, b)| {
        Box::pin(scopes::ensure_current(db, tx, b, true))
    })
    .await
    .unwrap();
    let original = begin(&f, None, Some(context(&f.bot)), false).await;
    let chat = crate::services::assistant_authority_tests::chat_for_row(
        &f.state,
        &f.bot.user_id,
        &original,
    )
    .await;
    let card = acks::request(db, &chat, request("Denied A operation"))
        .await
        .unwrap();
    acks::decide(db, &f.bot.user_id, &original.id, &card.id, false)
        .await
        .unwrap();
    finish_source_turn(&f.state, &original, "A answer").await;
    let web = begin(&f, Some(&original.id), None, false).await;
    assert!(
        acks::source::publish_owner_denial(db, &f.bot.user_id, &card.id, &original.id)
            .await
            .unwrap()
    );
    crate::services::channel_meta_service::set_bot_delivery(db, &f.bot, &f.bot.user_id, false)
        .await
        .unwrap();
    let held = db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id":&f.bot.id})
        .await
        .unwrap()
        .unwrap();
    crate::services::channel_meta_service::set_bot_delivery(db, &held, &f.bot.user_id, true)
        .await
        .unwrap();
    let mut event = crate::services::assistant_team_service::event(
        "message",
        "Healthy B question".into(),
        None,
    );
    event.channel_message = Some("Healthy B question".into());
    event.channel_contexts = vec![context(&b)];
    engine::push_events(db, &f.bot.user_id, &original.id, vec![event.clone()])
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":f.scope.get("_id").cloned()},
            doc! {"$set":{"from_generation":"unresolved"}},
        )
        .await
        .unwrap();
    assert!(
        crate::services::channel_thread_follow_service::prune_queue(
            db,
            &f.bot.user_id,
            &original.id
        )
        .await
        .is_err()
    );
    assert_eq!(
        engine::get(db, &f.bot.user_id, &original.id)
            .await
            .unwrap()
            .pending_events
            .len(),
        2
    );
    db.collection::<Document>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":f.scope.get("_id").cloned()},
            doc! {"$set":{"from_generation":f.scope.get("from_generation").cloned()}},
        )
        .await
        .unwrap();
    finish_source_turn(&f.state, &web, "Web answer").await;
    let admitted = begin(&f, Some(&original.id), None, true).await;
    assert_eq!(
        admitted.active_turn.as_ref().unwrap().events[0].id,
        event.id
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn clean_stream_eof_after_meta_turn_erasure_cannot_recreate_receipt_and_web_receipts_survive()
{
    use crate::services::assistant_service_execution::{Decision, HttpApprovalReceipt};
    use futures::StreamExt;
    for meta in [true, false] {
        let f = fixture().await;
        let db = &f.state.db;
        engine::ensure_indexes(db).await.unwrap();
        let original = begin(&f, None, meta.then(|| context(&f.bot)), false).await;
        let chat = crate::services::assistant_authority_tests::chat_for_row(
            &f.state,
            &f.bot.user_id,
            &original,
        )
        .await;
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let held = release.clone();
        let stream = async_stream::stream! {yield Ok::<_,std::io::Error>(bytes::Bytes::from_static(b"response"));held.notified().await;};
        let response = axum::response::Response::new(axum::body::Body::from_stream(stream));
        let decision = Decision {
            refusal: None,
            admission: None,
            automatic: None,
        };
        let receipt = HttpApprovalReceipt::new(
            Some(acks::AutoAllowed {
                class: acks::OwnerActionClass::Read,
                mode: crate::models::assistant_settings::AccessMode::Autopilot,
                reason: "approval",
            }),
            "service",
            "Receipt target",
            &crate::services::operation_descriptor::build_llm_descriptor("GET", "/result", None),
        );
        let response = decision
            .finish_http(response, db.clone(), Some(chat), "GET", receipt)
            .await;
        let mut body = response.into_body().into_data_stream();
        assert!(body.next().await.unwrap().is_ok());
        finish_source_turn(&f.state, &original, "Completed original turn").await;
        if meta {
            erase(&f).await;
        }
        release.notify_one();
        assert!(body.next().await.is_none());
        assert_eq!(
            db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
                .count_documents(doc! {"conversation_id":&original.id,"decided_by":"access_mode"})
                .await
                .unwrap(),
            u64::from(!meta)
        );
        db.drop().await.unwrap();
    }
}
