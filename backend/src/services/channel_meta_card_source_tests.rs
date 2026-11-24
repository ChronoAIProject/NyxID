use super::*;
use crate::{
    models::assistant_conversation::{ChannelTurnContext, TurnOrigin},
    services::{assistant_acknowledgement_service as acks, assistant_nyxagent as engine},
};

fn context(bot: &ChannelBot) -> ChannelTurnContext {
    bson::from_document(doc! {"meta_grant":bson::to_bson(&scopes::reference(bot).unwrap()).unwrap(),
        "source_event_id":uuid::Uuid::new_v4().to_string(),"channel_id":uuid::Uuid::new_v4().to_string(),"channel_bot_id":&bot.id,
        "conversation_kind":"private","sender_kind":"human","guest":false,"original_message":true}).unwrap()
}
async fn begin(
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
                web.active_turn.as_ref().map(|turn| turn.turn_id.clone())
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
