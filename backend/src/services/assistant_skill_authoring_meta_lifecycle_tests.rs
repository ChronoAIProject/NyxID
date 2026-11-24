use super::*;
use crate::{
    models::{
        assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME as CLAIMS,
        channel_bot::ChannelBot,
    },
    services::{assistant_nyxagent as engine, channel_meta_service as meta},
};

#[tokio::test]
async fn expired_skill_review_renews_after_real_registry_write_refusal_and_restored_access() {
    for meta_source in [true, false] {
        let f = fixture("skill_renew_restored_registry_access").await;
        let server = setup_ornn(&f).await;
        let chat = if meta_source {
            meta_turn(&f).await.1
        } else {
            f.chat.clone()
        };
        let first = draft_in_chat(&f, &chat).await.unwrap();
        let db = &f.state.db;
        let proposal = db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":first["proposal_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        let cards = db.collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME);
        let card = cards
            .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v1/skill-format/validate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/skills"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({"code":"forbidden"})))
            .expect(1)
            .mount(&server)
            .await;
        assert!(finish(&f, card.clone(), true).await.is_err());
        let failed = stored(&f, &proposal.id).await;
        assert_eq!(failed.failure_code.as_deref(), Some("ornn_write_forbidden"));
        assert!(!failed.publication.as_ref().unwrap().started);
        db.collection::<bson::Document>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":&card.id},
                doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let preview = review::authored_preview_for_card(
            &f.state,
            &f.owner,
            &proposal.agent_id,
            &proposal.id,
            &card.id,
        )
        .await
        .unwrap();
        assert!(
            preview["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("renew"))
        );
        server.verify().await;
        server.reset().await;
        let skill = Uuid::new_v4().to_string();
        mount_publication(&f, &server, &proposal, &skill, "POST").await;
        let axum::Json(result)=crate::handlers::assistant_agent_learning::approve(
            axum::extract::State(f.state.clone()),crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path((proposal.agent_id.clone(),proposal.id.clone())),
            axum::Json(serde_json::from_value(json!({"revision":proposal.revision,"agent_skills_revision":proposal.agent_skills_revision,"renewal_of":&card.id})).unwrap()),
        ).await.unwrap();
        let renewed = cards
            .find_one(
                doc! {"_id":result["acknowledgement"]["acknowledgement_id"].as_str().unwrap()},
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renewed.arguments_digest, card.arguments_digest);
        assert_eq!(renewed.creation_turn_id, card.creation_turn_id);
        assert_eq!(
            bson::to_bson(&renewed.meta_contexts).unwrap(),
            bson::to_bson(&card.meta_contexts).unwrap()
        );
        assert_eq!(finish(&f, renewed, true).await.unwrap().status, "used");
        let pinned = stored(&f, &proposal.id).await;
        assert_eq!(pinned.status, "pinned");
        assert_eq!(
            pinned.publication.as_ref().unwrap().operation_id,
            proposal.publication.as_ref().unwrap().operation_id
        );
        server.verify().await;
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn full_access_renewal_receipt_keeps_b_source_while_a_is_active_in_both_erasure_orders() {
    for a_first in [true, false] {
        let f = fixture("meta_full_access_renewal_receipt").await;
        let server = setup_ornn(&f).await;
        let (b, b_chat) = meta_turn(&f).await;
        let first = draft_in_chat(&f, &b_chat).await.unwrap();
        let db = &f.state.db;
        let proposal = stored(&f, first["proposal_id"].as_str().unwrap()).await;
        let cards = db.collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME);
        let original = cards
            .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        db.collection::<bson::Document>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":&original.id},
                doc! {"$set":{"status":"expired","expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let (a, a_chat) = meta_turn(&f).await;
        crate::services::assistant_settings_service::update(
            db,
            &f.owner,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let skill = Uuid::new_v4().to_string();
        mount_publication(&f, &server, &proposal, &skill, "POST").await;
        let axum::Json(result)=crate::handlers::assistant_agent_learning::approve(
            axum::extract::State(f.state.clone()),crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path((proposal.agent_id.clone(),proposal.id.clone())),
            axum::Json(serde_json::from_value(json!({"revision":proposal.revision,"agent_skills_revision":proposal.agent_skills_revision,"renewal_of":&original.id})).unwrap()),
        ).await.unwrap();
        assert_eq!(result["status"], "pinned");
        let receipt = cards
            .find_one(doc! {"user_id":&f.owner,"tool_name":review::TOOL,"decided_by":"access_mode"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.creation_turn_id, original.creation_turn_id);
        assert_eq!(receipt.requested_turn_id, original.requested_turn_id);
        assert_ne!(receipt.creation_turn_id, a_chat.turn_id);
        assert_eq!(
            bson::to_bson(&receipt.meta_contexts).unwrap(),
            bson::to_bson(&original.meta_contexts).unwrap()
        );
        let a_turn = engine::get(db, &f.owner, &a_chat.conversation_id)
            .await
            .unwrap();
        assert_eq!(
            a_turn.active_turn.as_ref().unwrap().turn_id,
            a_chat.turn_id.as_deref().unwrap()
        );
        settle(&f, &a_turn).await;
        let (first_bot, second_bot) = if a_first { (&a, &b) } else { (&b, &a) };
        assert!(erase(&f, first_bot).await);
        assert_eq!(
            cards
                .count_documents(doc! {"_id":&receipt.id})
                .await
                .unwrap(),
            u64::from(a_first)
        );
        if !a_first {
            assert!(
                db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
                    .find_one(
                        doc! {"conversation_id":&a_chat.conversation_id,"turn_id":&a_chat.turn_id}
                    )
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        let retained = stored(&f, &proposal.id).await;
        assert_eq!(retained.status, "pinned");
        assert_eq!(
            retained.publication.as_ref().unwrap().skill_id.as_deref(),
            Some(skill.as_str())
        );
        assert!(erase(&f, second_bot).await);
        assert!(
            cards
                .find_one(doc! {"_id":&receipt.id})
                .await
                .unwrap()
                .is_none()
        );
        server.verify().await;
        db.drop().await.unwrap();
    }
}

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
    let mut bot = crate::services::channel_adapters::meta::tests::registered_bot(
        &f.state,
        &f.owner,
        "instagram",
    )
    .await;
    let db = &f.state.db;
    bot.platform_bot_id = Uuid::new_v4().as_u128().to_string();
    db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .replace_one(doc! {"_id":&bot.id}, &bot)
        .await
        .unwrap();
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
async fn deleting_draft_grant_removes_its_queued_repair_notice_and_preserves_web_notice() {
    let f = fixture("meta_draft_repair_notice").await;
    let server = setup_ornn(&f).await;
    let (bot, chat) = meta_turn(&f).await;
    let first = draft_in_chat(&f, &chat).await.unwrap();
    let db = &f.state.db;
    let card = db
        .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
        .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let grant = meta::scopes::reference(&bot).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":{"valid":false,"violations":[{"rule":"allowed-root-items"}]}}),
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    assert!(matches!(finish(&f, card, true).await,
        Err(AppError::Conflict(code)) if code == "ornn_validation_failed"));
    let home = engine::get(db, &f.owner, &f.row.id).await.unwrap();
    let notice = home
        .pending_events
        .iter()
        .find(|event| event.kind == "skill_publication")
        .unwrap();
    assert_eq!(notice.channel_contexts.len(), 1);
    assert_eq!(notice.channel_contexts[0].meta_grant.as_ref(), Some(&grant));
    let unrelated = crate::services::assistant_team_service::event(
        "notice",
        "Independent web notice".into(),
        None,
    );
    engine::push_events(db, &f.owner, &f.row.id, vec![unrelated.clone()])
        .await
        .unwrap()
        .unwrap();
    assert!(erase(&f, &bot).await);
    let home = engine::get(db, &f.owner, &f.row.id).await.unwrap();
    assert!(
        home.pending_events
            .iter()
            .all(|event| event.id != notice.id)
    );
    assert!(
        home.pending_events
            .iter()
            .any(|event| event.id == unrelated.id)
    );
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
    let mut published = db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":&proposal.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(published.get_str("status"), Ok("pinned"));
    published.remove("authoring_origin");
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

#[tokio::test]
async fn renew_meta_skill_review_keeps_original_b_source_while_a_or_web_uses_home() {
    for meta_active in [true, false] {
        let f = fixture("meta_draft_renew_original_source").await;
        let (b, chat) = meta_turn(&f).await;
        let first = draft_in_chat(&f, &chat).await.unwrap();
        let db = &f.state.db;
        let proposal = db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":first["proposal_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        let cards = db.collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME);
        let original = cards
            .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        db.collection::<bson::Document>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .update_one(
                doc! {"_id":&original.id},
                doc! {"$set":{"status":"expired","expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let a = if meta_active {
            Some(meta_turn(&f).await.0)
        } else {
            settle(&f, &engine::get(db, &f.owner, &f.row.id).await.unwrap()).await;
            let request: engine::TurnRequest = serde_json::from_value(
                json!({"conversation_id":&f.row.id,"text":"Independent web work"}),
            )
            .unwrap();
            engine::begin_turn(db, &f.owner, &request, &f.state.encryption_keys)
                .await
                .unwrap();
            None
        };
        let axum::Json(result) = crate::handlers::assistant_agent_learning::approve(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path((proposal.agent_id.clone(), proposal.id.clone())),
            axum::Json(
                serde_json::from_value(json!({"revision":proposal.revision,
                "agent_skills_revision":proposal.agent_skills_revision,"renewal_of":&original.id}))
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        let renewed = cards
            .find_one(
                doc! {"_id":result["acknowledgement"]["acknowledgement_id"].as_str().unwrap()},
            )
            .await
            .unwrap()
            .unwrap();
        assert_ne!(renewed.id, original.id);
        assert_eq!(renewed.creation_turn_id, original.creation_turn_id);
        assert_eq!(renewed.requested_turn_id, original.requested_turn_id);
        assert_eq!(renewed.arguments_digest, original.arguments_digest);
        assert_eq!(
            bson::to_bson(&renewed.meta_contexts).unwrap(),
            bson::to_bson(&original.meta_contexts).unwrap()
        );
        settle(&f, &engine::get(db, &f.owner, &f.row.id).await.unwrap()).await;
        if let Some(a) = a {
            assert!(erase(&f, &a).await);
            assert!(
                cards
                    .find_one(doc! {"_id":&renewed.id,"status":"pending"})
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        assert!(erase(&f, &b).await);
        assert!(
            cards
                .find_one(doc! {"_id":&renewed.id})
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn prepared_renewal_cannot_recreate_review_after_original_grant_cleanup() {
    let f = fixture("meta_draft_late_renewal").await;
    let (bot, chat) = meta_turn(&f).await;
    let first = draft_in_chat(&f, &chat).await.unwrap();
    let db = &f.state.db;
    let proposal = db
        .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":first["proposal_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    let binding = publication::binding(
        &proposal,
        proposal.publication.as_ref().unwrap(),
        proposal.agent_skills_revision,
    );
    assert!(erase(&f, &bot).await);
    assert!(
        renew(
            db,
            &chat,
            first["acknowledgement_id"].as_str().unwrap(),
            &binding
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<bson::Document>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .count_documents(doc! {"authored_skill.proposal_id":&proposal.id})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn contradictory_mixed_grant_claim_keeps_both_original_operations_for_repair() {
    let f = fixture("meta_draft_mixed_claim_repair").await;
    let (a, chat) = meta_turn(&f).await;
    let first = draft_in_chat(&f, &chat).await.unwrap();
    let (b, b_chat) = meta_turn(&f).await;
    let db = &f.state.db;
    let b_row = engine::get(db, &f.owner, &b_chat.conversation_id)
        .await
        .unwrap();
    let proof = b_row
        .active_turn
        .as_ref()
        .unwrap()
        .channel_context
        .as_ref()
        .unwrap();
    db.collection::<bson::Document>(CLAIMS)
        .update_one(
            doc! {"claim.proposal_id":first["proposal_id"].as_str().unwrap()},
            doc! {"$push":{"claim.meta_contexts":bson::to_bson(proof).unwrap()}},
        )
        .await
        .unwrap();
    let before = db
        .collection::<bson::Document>(CLAIMS)
        .find_one(doc! {"claim.proposal_id":first["proposal_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    for bot in [&a, &b] {
        let scope = db
            .collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
            .find_one(
                doc! {"bot_id":&bot.id,"grant_id":meta::scopes::reference(bot).unwrap().grant_id},
            )
            .await
            .unwrap()
            .unwrap();
        let result = crate::services::transaction_service::run_db_only(
            db,
            &(db, &scope),
            |tx, &(db, scope)| Box::pin(erasure::erase_page(db, tx, scope, None)),
        )
        .await;
        assert!(
            matches!(result,Err(AppError::Conflict(message)) if message.contains("Meta grant evidence is incomplete"))
        );
        assert_eq!(
            db.collection::<bson::Document>(CLAIMS)
                .find_one(doc! {"_id":before.get("_id").cloned()})
                .await
                .unwrap()
                .unwrap(),
            before
        );
        assert!(
            db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                .find_one(doc! {"_id":first["proposal_id"].as_str().unwrap()})
                .await
                .unwrap()
                .is_some()
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn original_draft_erasure_releases_only_exact_reserved_target_and_retains_contradictions() {
    use crate::models::assistant_agent_learning::PUBLICATION_TARGETS_COLLECTION_NAME as TARGETS;
    for disposition in ["exact", "sibling", "contradictory"] {
        let f = fixture("meta_draft_exact_target_cleanup").await;
        let (bot, chat) = meta_turn(&f).await;
        let first = draft_in_chat(&f, &chat).await.unwrap();
        let db = &f.state.db;
        let id = first["proposal_id"].as_str().unwrap();
        // Defensive inventory fixture for a reserved, not yet dispatched,
        // update. No provider request is sent by this cleanup operation.
        db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME).update_one(doc! {"_id":id},
            doc! {"$set":{"publication.target_kind":"update","publication.target_skill_id":"source-skill"}}).await.unwrap();
        let proposal = db
            .collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        let p = proposal.publication.as_ref().unwrap();
        let target = format!("source-skill:{}", p.version);
        let mut holder = doc! {"_id":&target,"owner_id":&proposal.owner_id,"agent_id":&proposal.agent_id,
        "proposal_id":id,"operation_id":&p.operation_id,"package_sha256":&p.sha256,"state":"reserved"};
        if disposition == "sibling" {
            holder.insert("proposal_id", Uuid::new_v4().to_string());
            holder.insert("operation_id", Uuid::new_v4().to_string());
        } else if disposition == "contradictory" {
            holder.insert("package_sha256", "contradictory-digest");
        }
        db.collection::<bson::Document>(TARGETS)
            .insert_one(&holder)
            .await
            .unwrap();
        let scope = db
            .collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
            .find_one(
                doc! {"bot_id":&bot.id,"grant_id":meta::scopes::reference(&bot).unwrap().grant_id},
            )
            .await
            .unwrap()
            .unwrap();
        let result = crate::services::transaction_service::run_db_only(
            db,
            &(db, &scope),
            |tx, &(db, scope)| Box::pin(erasure::erase_page(db, tx, scope, None)),
        )
        .await;
        if disposition == "contradictory" {
            assert!(matches!(result, Err(AppError::Conflict(_))));
        } else {
            let page = result.unwrap();
            assert!(!page.pending);
            assert!(page.after.is_none());
        }
        for (collection, filter) in [
            (CLAIMS, doc! {"claim.proposal_id":id}),
            (PROPOSALS_COLLECTION_NAME, doc! {"_id":id}),
        ] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(filter)
                    .await
                    .unwrap(),
                u64::from(disposition == "contradictory")
            );
        }
        let retained = db
            .collection::<bson::Document>(TARGETS)
            .find_one(doc! {"_id":&target})
            .await
            .unwrap();
        if disposition == "exact" {
            assert!(retained.is_none());
        } else {
            assert_eq!(retained.unwrap(), holder);
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn original_draft_claim_inventory_resumes_after_one_hundred_rows() {
    let f = fixture("meta_draft_claim_page_bound").await;
    let (bot, chat) = meta_turn(&f).await;
    let first = draft_in_chat(&f, &chat).await.unwrap();
    let db = &f.state.db;
    let original = db
        .collection::<bson::Document>(CLAIMS)
        .find_one(doc! {"claim.proposal_id":first["proposal_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    // Retained orphan claims remain attributable even if their proposals have
    // already disappeared. Seed the page boundary without 101 provider calls.
    let mut rows = Vec::new();
    for _ in 0..100 {
        let mut row = original.clone();
        row.insert("_id", Uuid::new_v4().to_string());
        row.get_document_mut("claim")
            .unwrap()
            .insert("proposal_id", Uuid::new_v4().to_string());
        rows.push(row);
    }
    db.collection::<bson::Document>(CLAIMS)
        .insert_many(rows)
        .await
        .unwrap();
    let scope = db
        .collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
        .find_one(
            doc! {"bot_id":&bot.id,"grant_id":meta::scopes::reference(&bot).unwrap().grant_id},
        )
        .await
        .unwrap()
        .unwrap();
    let first =
        crate::services::transaction_service::run_db_only(db, &(db, &scope), |tx, &(db, scope)| {
            Box::pin(erasure::erase_page(db, tx, scope, None))
        })
        .await
        .unwrap();
    assert!(!first.pending);
    assert!(first.after.is_some());
    assert_eq!(
        db.collection::<bson::Document>(CLAIMS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let next = crate::services::transaction_service::run_db_only(
        db,
        &(db, &scope, first.after.as_deref()),
        |tx, &(db, scope, after)| Box::pin(erasure::erase_page(db, tx, scope, after)),
    )
    .await
    .unwrap();
    assert!(!next.pending);
    assert!(next.after.is_none());
    assert_eq!(
        db.collection::<bson::Document>(CLAIMS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn signed_cleanup_finds_superseded_meta_draft_after_real_key_rotation_and_preserves_successor()
 {
    for disposition in ["pending", "legacy_pending", "verified"] {
        let f = fixture("meta_draft_superseded_origin").await;
        let server = setup_ornn(&f).await;
        let (bot, chat) = meta_turn(&f).await;
        let first = draft_in_chat(&f, &chat).await.unwrap();
        let db = &f.state.db;
        let proposals = db.collection::<AssistantAgentLearningProposal>(PROPOSALS_COLLECTION_NAME);
        let id = first["proposal_id"].as_str().unwrap();
        let original = proposals.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        let mut skill = None;
        if disposition == "verified" {
            let card = db
                .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
                .find_one(doc! {"_id":first["acknowledgement_id"].as_str().unwrap()})
                .await
                .unwrap()
                .unwrap();
            let published = Uuid::new_v4().to_string();
            mount_publication(&f, &server, &original, &published, "POST").await;
            assert_eq!(finish(&f, card, true).await.unwrap().status, "used");
            skill = Some(published);
        } else if disposition == "legacy_pending" {
            db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                .update_one(doc! {"_id":id}, doc! {"$unset":{"authoring_origin":""}})
                .await
                .unwrap();
        }
        settle(
            &f,
            &engine::get(db, &f.owner, &chat.conversation_id)
                .await
                .unwrap(),
        )
        .await;
        crate::services::key_service::rotate_api_key(
            db,
            &f.state.encryption_keys,
            &f.owner,
            &chat.api_key_id,
        )
        .await
        .unwrap();
        let caller = another_authoring_chat(&f).await;
        let second = draft_in_chat(&f, &caller).await.unwrap();
        let successor = second["proposal_id"].as_str().unwrap();
        assert_ne!(successor, id);
        assert_eq!(
            db.collection::<bson::Document>(CLAIMS)
                .count_documents(doc! {"claim.proposal_id":id})
                .await
                .unwrap(),
            0
        );
        let saved = proposals.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_eq!(
            saved.authoring_origin.as_ref().unwrap().meta_contexts[0].meta_grant,
            Some(meta::scopes::reference(&bot).unwrap())
        );
        let successor_before = db
            .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
            .find_one(doc! {"_id":successor})
            .await
            .unwrap()
            .unwrap();
        assert!(erase(&f, &bot).await);
        if let Some(skill) = skill {
            let retained = proposals.find_one(doc! {"_id":id}).await.unwrap().unwrap();
            assert_eq!(retained.status, "pinned");
            assert!(retained.authoring_origin.is_none());
            assert_eq!(
                retained.publication.as_ref().unwrap().skill_id.as_deref(),
                Some(skill.as_str())
            );
        } else {
            assert!(proposals.find_one(doc! {"_id":id}).await.unwrap().is_none());
        }
        assert_eq!(
            db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
                .find_one(doc! {"_id":successor})
                .await
                .unwrap()
                .unwrap(),
            successor_before
        );
        let successor_card = db
            .collection::<AssistantAcknowledgement>(ACKNOWLEDGEMENTS_COLLECTION_NAME)
            .find_one(
                doc! {"_id":second["acknowledgement_id"].as_str().unwrap(),"status":"pending"},
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            acks::decide(
                db,
                &f.owner,
                &caller.conversation_id,
                &successor_card.id,
                true
            )
            .await
            .unwrap()
            .status,
            "allowed"
        );
        server.verify().await;
        db.drop().await.unwrap();
    }
}
