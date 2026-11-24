use super::*;
use crate::{
    models::{
        assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME as CLAIMS,
        channel_bot::ChannelBot,
    },
    services::{assistant_nyxagent as engine, channel_meta_service as meta},
};

async fn settle(f: &Fixture, row: &crate::models::assistant_conversation::AssistantConversation) {
    engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Finished".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}

async fn meta_turn(f: &Fixture) -> (ChannelBot, ChatAuthority) {
    let bot = crate::services::channel_adapters::meta::tests::registered_bot(
        &f.state,
        &f.owner,
        "instagram",
    )
    .await;
    let db = &f.state.db;
    crate::services::transaction_service::run_db_only(db, &(db, &bot), |tx, &(db, bot)| {
        Box::pin(meta::scopes::ensure_current(db, tx, bot, true))
    })
    .await
    .unwrap();
    settle(f, &engine::get(db, &f.owner, &f.row.id).await.unwrap()).await;
    let request: engine::TurnRequest = serde_json::from_value(
        json!({"conversation_id":&f.row.id,"text":"Draft my private weekly review skill"}),
    )
    .unwrap();
    let mut start = engine::TurnStart::from(&request);
    start.origin = crate::models::assistant_conversation::TurnOrigin::Channel;
    start.channel_context=Some(bson::from_document(doc! {"meta_grant":bson::to_bson(&meta::scopes::reference(&bot).unwrap()).unwrap(),
        "source_event_id":Uuid::new_v4().to_string(),"channel_id":Uuid::new_v4().to_string(),"channel_bot_id":&bot.id,
        "conversation_kind":"private","sender_kind":"human","guest":false,"original_message":true}).unwrap());
    let row = engine::begin_turn(db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    let chat =
        crate::services::assistant_authority_tests::chat_for_row(&f.state, &f.owner, &row).await;
    (bot, chat)
}

async fn erase(f: &Fixture, bot: &ChannelBot) -> bool {
    let db = &f.state.db;
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&bot.id},doc! {"$set":{"is_active":false,"meta_binding.provider_revoked":true,"meta_binding.subscription":"removed"}}).await.unwrap();
    let service = meta::MetaService::from(&f.state);
    let cutoff = bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(1));
    for _ in 0..100 {
        if meta::scope_erasure::pass(
            &f.state,
            &service,
            &bot.id,
            &meta::scopes::reference(bot).unwrap().grant_id,
            cutoff,
        )
        .await
        .unwrap()
        {
            return true;
        }
    }
    false
}

#[tokio::test]
async fn deleting_drafting_grant_retires_original_claim_and_unstarted_proposal_and_allows_fresh_web_draft()
 {
    let f = fixture("meta_draft_own_erasure").await;
    let (bot, chat) = meta_turn(&f).await;
    let first = draft_in_chat(&f, &chat).await.unwrap();
    let proposal = first["proposal_id"].as_str().unwrap();
    let card = first["acknowledgement_id"].as_str().unwrap();
    let db = &f.state.db;
    assert_eq!(
        db.collection::<bson::Document>(CLAIMS)
            .count_documents(doc! {"claim.proposal_id":proposal})
            .await
            .unwrap(),
        1
    );
    assert!(erase(&f, &bot).await);
    for (collection, filter) in [
        (CLAIMS, doc! {"claim.proposal_id":proposal}),
        (PROPOSALS_COLLECTION_NAME, doc! {"_id":proposal}),
        (ACKNOWLEDGEMENTS_COLLECTION_NAME, doc! {"_id":card}),
    ] {
        assert_eq!(
            db.collection::<bson::Document>(collection)
                .count_documents(filter)
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    let caller = another_authoring_chat(&f).await;
    let fresh = draft_in_chat(&f, &caller).await.unwrap();
    assert_ne!(fresh["proposal_id"], first["proposal_id"]);
    assert_ne!(fresh["acknowledgement_id"], first["acknowledgement_id"]);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn deleting_drafting_grant_preserves_verified_publication_but_retires_original_claim() {
    let f = fixture("meta_draft_published_erasure").await;
    let server = setup_ornn(&f).await;
    let (bot, chat) = meta_turn(&f).await;
    let first = draft_in_chat(&f, &chat).await.unwrap();
    let db = &f.state.db;
    let proposal = db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":first["proposal_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let card = db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let skill = Uuid::new_v4().to_string();
    mount_publication(&f, &server, &proposal, &skill, "POST").await;
    assert_eq!(finish(&f, card, true).await.unwrap().status, "used");
    let published = db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&proposal.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(published.get_str("status"), Ok("pinned"));
    assert!(erase(&f, &bot).await);
    assert_eq!(
        db.collection::<bson::Document>(CLAIMS)
            .count_documents(doc! {"claim.proposal_id":&proposal.id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":&proposal.id})
            .await
            .unwrap()
            .unwrap(),
        published
    );
    assert!(
        db.collection::<bson::Document>(
            crate::models::assistant_agent_learning::ROOTS_COLLECTION_NAME
        )
        .find_one(doc! {"proposal_id":&proposal.id,"skill_id":&skill})
        .await
        .unwrap()
        .is_some()
    );
    server.verify().await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn uncertain_draft_publication_keeps_erasure_pending_with_original_evidence() {
    let f = fixture("meta_draft_uncertain_erasure").await;
    let (bot, chat) = meta_turn(&f).await;
    let first = draft_in_chat(&f, &chat).await.unwrap();
    let db = &f.state.db;
    let id = first["proposal_id"].as_str().unwrap();
    // Model a crash after the durable dispatch marker, before its result.
    db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"publication.started":true,"publication.uncertain_dispatch":true}},
        )
        .await
        .unwrap();
    assert!(!erase(&f, &bot).await);
    assert_eq!(
        db.collection::<bson::Document>(CLAIMS)
            .count_documents(doc! {"claim.proposal_id":id})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .count_documents(doc! {"_id":id})
            .await
            .unwrap(),
        1
    );
    assert!(
        db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .find_one(doc! {"conversation_id":&chat.conversation_id,"turn_id":chat.turn_id})
            .await
            .unwrap()
            .is_some()
    );
    db.drop().await.unwrap();
}
