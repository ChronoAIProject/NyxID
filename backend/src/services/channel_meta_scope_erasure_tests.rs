use super::*;
use crate::{
    models::assistant_conversation::{ActiveTurn, AssistantConversation},
    services::transaction_service,
};

#[path = "channel_meta_card_source_tests.rs"]
mod card_sources;

#[path = "channel_meta_async_lifecycle_tests.rs"]
mod async_lifecycle;

struct Fixture {
    state: crate::AppState,
    bot: ChannelBot,
    scope: Document,
    adapter: crate::services::channel_adapters::meta::MetaAdapter,
    server: wiremock::MockServer,
}

async fn fixture() -> Fixture {
    let (state, adapter, server, bot) =
        crate::services::channel_adapters::meta::tests::fixture("instagram").await;
    scopes::ensure_indexes(&state.db).await.unwrap();
    let scope =
        transaction_service::run_db_only(&state.db, &(&state.db, &bot), |tx, &(db, bot)| {
            Box::pin(scopes::ensure_current(db, tx, bot, true))
        })
        .await
        .unwrap();
    Fixture {
        state,
        adapter,
        server,
        bot,
        scope: bson::to_document(&scope).unwrap(),
    }
}

fn conversation(owner: &str, guest: bool, proof: &GrantRef) -> Document {
    doc! {"_id":format!("nyxa-{}",uuid::Uuid::new_v4().simple()),"user_id":owner,"title":"Local history","model":"test",
    "credential_api_key_id":"","message_count":4_i64,"guest_turn":guest,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    "meta_grant":if guest {bson::to_bson(proof).unwrap()} else {Bson::Null}}
}

fn message(
    owner: &str,
    conversation: &str,
    turn: &str,
    seq: i64,
    role: &str,
    proof: &GrantRef,
) -> Document {
    doc! {"_id":uuid::Uuid::new_v4().to_string(),"conversation_id":conversation,"user_id":owner,"turn_id":turn,"seq":seq,
    "role":role,"text":format!("Local {turn} {role} text"),"status":"completed","created_at":bson::DateTime::now(),
    "meta_grants":[bson::to_bson(proof).unwrap()]}
}

#[tokio::test]
async fn guest_cascade_requires_positive_proof_for_untagged_mixed_and_ttl_history() {
    for evidence in [
        "same",
        "untagged",
        "foreign",
        "malformed",
        "orphan-child",
        "child-same",
        "child-foreign",
        "child-range",
        "child-malformed",
        "voice-child",
        "ttl-child",
        "org-turn-child",
    ] {
        let f = fixture().await;
        let db = &f.state.db;
        let grant = scopes::reference(&f.bot).unwrap();
        let parent = conversation(&f.bot.user_id, true, &grant);
        let id = parent.get_str("_id").unwrap();
        db.collection::<Document>("assistant_conversations")
            .insert_one(&parent)
            .await
            .unwrap();
        let mut first = message(&f.bot.user_id, id, "a-turn", 1, "user", &grant);
        match evidence {
            "ttl-child" => {
                first.insert("attachments", vec![doc! {"id":"expired-tool-image"}]);
            }
            "untagged" => {
                first.remove("meta_grants");
            }
            "foreign" => {
                let mut foreign = grant.clone();
                foreign.grant_id = uuid::Uuid::new_v4().to_string();
                first.insert("meta_grants", bson::to_bson(&vec![foreign]).unwrap());
            }
            "malformed" => {
                first.insert("meta_grants", "not typed proof");
            }
            _ => {}
        }
        db.collection::<Document>("assistant_messages")
            .insert_one(&first)
            .await
            .unwrap();
        if evidence == "orphan-child" {
            db.collection::<Document>(crate::models::assistant_tool_result::COLLECTION_NAME).insert_one(doc! {
                "_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,"conversation_id":id,"turn_id":"unproven-old-turn"}).await.unwrap();
        }
        if evidence == "voice-child" {
            db.collection::<Document>(crate::models::assistant_voice_session::COLLECTION_NAME).insert_one(doc! {
                "_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,"conversation_id":id,"turn_id":"a-turn","live_slot":true,
                "meta_grant":bson::to_bson(&grant).unwrap()}).await.unwrap();
        }
        if evidence == "ttl-child" {
            db.collection::<Document>(crate::models::assistant_upload_retention::TOMBSTONES).insert_one(doc! {
                "_id":"expired-tool-image","user_id":&f.bot.user_id,"conversation_id":id,"expired_at":bson::DateTime::now()}).await.unwrap();
        }
        if evidence == "org-turn-child" {
            db.collection::<Document>(crate::models::org_channel::MESSAGES).insert_one(doc! {
                "_id":uuid::Uuid::new_v4().to_string(),"user_id":uuid::Uuid::new_v4().to_string(),"conversation_id":id,
                "request_id":"a-turn","text":"Organization guest transcript"}).await.unwrap();
        }
        let child = if evidence.starts_with("child-") {
            let mut proof = grant.clone();
            if evidence == "child-foreign" {
                proof.grant_id = uuid::Uuid::new_v4().to_string();
            } else if evidence == "child-range" {
                proof.generation = -1;
            }
            let row = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,
            "conversation_id":id,"turn_id":"a-turn","meta_grant":if evidence=="child-malformed" {
                Bson::String("contradictory proof".into())
            } else {bson::to_bson(&proof).unwrap()}};
            db.collection::<Document>(crate::models::assistant_tool_result::COLLECTION_NAME)
                .insert_one(&row)
                .await
                .unwrap();
            Some(row)
        } else {
            None
        };
        // Neither a missing finite-lived source nor absence of an explicit B
        // tag certifies that a legacy turn belongs to A.
        db.collection::<Document>(MESSAGES)
            .delete_many(doc! {})
            .await
            .unwrap();
        let mut progress = doc! {"phase":5};
        let mut refused = false;
        for _ in 0..128 {
            let result = transaction_service::run_db_only(
                db,
                &(db, &f.scope, &progress),
                |tx, &(db, scope, progress)| Box::pin(erase_guest(db, tx, scope, progress, 5)),
            )
            .await;
            match result {
                Ok(next) => progress = next.progress,
                Err(_) => {
                    refused = true;
                    break;
                }
            }
            if db
                .collection::<Document>("assistant_conversations")
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .is_none()
            {
                break;
            }
        }
        let completes = matches!(
            evidence,
            "same" | "child-same" | "ttl-child" | "org-turn-child"
        );
        assert_eq!(refused, !completes);
        assert_eq!(
            db.collection::<Document>("assistant_conversations")
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .is_none(),
            completes
        );
        if refused {
            if let Some(child) = child {
                assert_eq!(
                    db.collection::<Document>(
                        crate::models::assistant_tool_result::COLLECTION_NAME
                    )
                    .find_one(doc! {"_id":child.get("_id").cloned()})
                    .await
                    .unwrap()
                    .unwrap(),
                    child
                );
            }
            assert_eq!(
                db.collection::<Document>("assistant_messages")
                    .find_one(doc! {"_id":first.get("_id").cloned()})
                    .await
                    .unwrap()
                    .unwrap(),
                first
            );
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn newer_checkpoint_keeps_independent_b_contents_after_source_expiry_and_redacts_only_inherited_a_goal()
 {
    use crate::models::{
        assistant_checkpoint::COLLECTION_NAME as CHECKPOINTS, assistant_message::AssistantMessage,
    };
    for mode in [
        "independent",
        "inherited",
        "unknown",
        "malformed",
        "completed-malformed",
        "failed-malformed",
    ] {
        let f = fixture().await;
        let db = &f.state.db;
        let a = scopes::reference(&f.bot).unwrap();
        let mut b = a.clone();
        b.grant_id = uuid::Uuid::new_v4().to_string();
        b.generation += 1;
        let parent = conversation(&f.bot.user_id, false, &a);
        let id = parent.get_str("_id").unwrap();
        db.collection::<Document>("assistant_conversations")
            .insert_one(&parent)
            .await
            .unwrap();
        let input_a = message(&f.bot.user_id, id, "a-turn", 1, "user", &a);
        let reply_a = message(&f.bot.user_id, id, "a-turn", 2, "assistant", &a);
        let input_b = message(&f.bot.user_id, id, "b-turn", 3, "user", &b);
        let reply_b = message(&f.bot.user_id, id, "b-turn", 4, "assistant", &b);
        db.collection::<Document>("assistant_messages")
            .insert_many([
                input_a.clone(),
                reply_a.clone(),
                input_b.clone(),
                reply_b.clone(),
            ])
            .await
            .unwrap();
        let row: AssistantConversation = bson::from_document(parent.clone()).unwrap();
        for (turn, seq, reply) in [
            ("a-turn", Some(1_i64), reply_a),
            (
                "b-turn",
                if mode == "inherited" {
                    None
                } else {
                    Some(3_i64)
                },
                reply_b,
            ),
        ] {
            let turn: ActiveTurn =
                bson::from_document(doc! {"turn_id":turn,"started_at":bson::DateTime::now(),
                "origin":if seq.is_some() {"channel"} else {"event"},"initiating_message_seq":seq})
                .unwrap();
            let reply: AssistantMessage = bson::from_document(reply).unwrap();
            transaction_service::run_db_only(
                db,
                &(db, &row, &turn, &reply),
                |tx, &(db, row, turn, reply)| {
                    Box::pin(
                        crate::services::assistant_nyxagent::write_checkpoint_in_session(
                            db,
                            tx,
                            row,
                            turn,
                            reply,
                            chrono::Utc::now(),
                        ),
                    )
                },
            )
            .await
            .unwrap();
        }
        let checkpoints = db.collection::<Document>(CHECKPOINTS);
        if mode == "unknown" {
            checkpoints
                .update_one(
                    doc! {"_id":id},
                    doc! {"$unset":{"goal_turn_id":"","source_turn_ids":""}},
                )
                .await
                .unwrap();
        }
        if mode == "malformed" {
            checkpoints
                .update_one(doc! {"_id":id}, doc! {"$set":{"source_turn_ids":"broken"}})
                .await
                .unwrap();
        }
        if matches!(mode, "completed-malformed" | "failed-malformed") {
            let field = if mode == "completed-malformed" {
                "completed_actions"
            } else {
                "failed_actions"
            };
            checkpoints
                .update_one(doc! {"_id":id}, doc! {"$set":{field:"not an action array"}})
                .await
                .unwrap();
        }
        let before = checkpoints
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        // The checkpoint writer persisted derivation on the original record;
        // cleanup does not need B's message/source to remain available.
        db.collection::<Document>("assistant_messages")
            .delete_many(doc! {"conversation_id":id,"turn_id":"b-turn"})
            .await
            .unwrap();
        db.collection::<Document>(MESSAGES)
            .delete_many(doc! {})
            .await
            .unwrap();
        let result = transaction_service::run_db_only(db, &(db, &f.scope), |tx, &(db, scope)| {
            Box::pin(async move { erase_turn(db, tx, scope, &Document::new(), 6).await })
        })
        .await;
        let after = checkpoints
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        if matches!(
            mode,
            "unknown" | "malformed" | "completed-malformed" | "failed-malformed"
        ) {
            assert!(result.is_err());
            assert_eq!(before, after);
        } else {
            result.unwrap();
            assert_eq!(after.get_str("turn_id"), Ok("b-turn"));
            assert_eq!(after.get_str("output"), before.get_str("output"));
            if mode == "independent" {
                assert_eq!(after, before);
            } else {
                assert!(!after.contains_key("goal") && !after.contains_key("goal_turn_id"));
                assert_eq!(
                    after.get_array("source_turn_ids").unwrap(),
                    &vec![Bson::String("b-turn".into())]
                );
            }
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn malformed_and_cross_grant_tags_remain_pending_until_exact_surviving_source_repair() {
    for malformed in [true, false] {
        let f = fixture().await;
        let db = &f.state.db;
        let original =
            crate::services::channel_adapters::meta::tests::original(&f.state, &f.bot).await;
        crate::services::channel_admission_service::store(db, original.clone())
            .await
            .unwrap()
            .unwrap();
        let a_plan = crate::services::channel_text_delivery::begin_bound(
            db,
            "source-proof-a",
            &f.bot,
            &original.id,
            "a-hash",
            1,
        )
        .await
        .unwrap();
        let mut b = f.bot.clone();
        let binding = b.meta_binding.as_mut().unwrap();
        binding.grant_id = uuid::Uuid::new_v4().to_string();
        binding.grant_subject_id = "901".into();
        binding.generation += 1;
        binding.grant_start_generation = binding.generation;
        transaction_service::run_db_only(db, &(db, &f.bot, &b), |tx, &(db, a, b)| {
            Box::pin(async move {
                scopes::cutover(db, tx, a, b.meta_binding.as_ref().unwrap()).await?;
                db.collection::<ChannelBot>(BOTS)
                    .replace_one(doc! {"_id":&b.id}, b)
                    .session(tx)
                    .await?;
                Ok(())
            })
        })
        .await
        .unwrap();
        f.adapter
            .prove_control(db, &f.state.http_client, &b, "333", "fixture-token")
            .await
            .unwrap();
        let mut b_message = original.clone();
        b_message.id = uuid::Uuid::new_v4().to_string();
        b_message.platform_message_id = Some("b-witness".into());
        let authority = b_message.meta_authority.as_mut().unwrap();
        authority.grant_id = Some(b.meta_binding.as_ref().unwrap().grant_id.clone());
        authority.generation = b.meta_binding.as_ref().unwrap().generation;
        authority.binding = super::super::meta::binding(&b);
        crate::services::channel_admission_service::store(db, b_message.clone())
            .await
            .unwrap()
            .unwrap();
        let b_plan = crate::services::channel_text_delivery::begin_bound(
            db,
            "source-proof-b",
            &b,
            &b_message.id,
            "b-hash",
            1,
        )
        .await
        .unwrap();
        let plans = db.collection::<Document>(PLANS);
        let poison = if malformed {
            Bson::String("malformed non-null proof".into())
        } else {
            bson::to_bson(&scopes::reference(&b).unwrap()).unwrap()
        };
        plans
            .update_one(doc! {"_id":&a_plan.id}, doc! {"$set":{"meta_grant":poison}})
            .await
            .unwrap();
        let before = plans
            .find_one(doc! {"_id":&a_plan.id})
            .await
            .unwrap()
            .unwrap();
        let service = MetaService::from(&f.state);
        let cutoff = bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::seconds(1));
        let mut refused = false;
        for _ in 0..8 {
            match pass(
                &f.state,
                &service,
                &f.bot.id,
                &f.bot.meta_binding.as_ref().unwrap().grant_id,
                cutoff,
            )
            .await
            {
                Ok(done) => assert!(!done, "a contradictory tag cannot certify deletion"),
                Err(_) => {
                    refused = true;
                    break;
                }
            }
        }
        assert!(refused);
        assert_eq!(
            plans
                .find_one(doc! {"_id":&a_plan.id})
                .await
                .unwrap()
                .unwrap(),
            before
        );
        assert!(
            plans
                .find_one(doc! {"_id":&b_plan.id})
                .await
                .unwrap()
                .is_some()
        );
        let stored = db
            .collection::<Document>(GRANT_SCOPES)
            .find_one(doc! {"_id":f.scope.get("_id").cloned()})
            .await
            .unwrap()
            .unwrap();
        assert!(
            !grants::completed(&stored, &f.bot.meta_binding.as_ref().unwrap().grant_id).unwrap()
        );
        plans.update_one(doc! {"_id":&a_plan.id},doc! {"$set":{"meta_grant":bson::to_bson(a_plan.meta_grant.as_ref().unwrap()).unwrap()}}).await.unwrap();
        let mut done = false;
        for _ in 0..80 {
            if pass(
                &f.state,
                &service,
                &f.bot.id,
                &f.bot.meta_binding.as_ref().unwrap().grant_id,
                cutoff,
            )
            .await
            .unwrap()
            {
                done = true;
                break;
            }
        }
        assert!(done);
        assert!(
            plans
                .find_one(doc! {"_id":&a_plan.id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            plans
                .find_one(doc! {"_id":&b_plan.id})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.collection::<Document>(MESSAGES)
                .find_one(doc! {"_id":&b_message.id})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn an_a_tag_cannot_authorize_erasing_a_siblings_original_parent() {
    let f = fixture().await;
    let db = &f.state.db;
    let id = uuid::Uuid::new_v4().to_string();
    let sibling = uuid::Uuid::new_v4().to_string();
    let row = doc! {"_id":&id,"channel_bot_id":&sibling,"user_id":&f.bot.user_id,"source_id":"expired-sibling-source",
    "meta_grant":bson::to_bson(&scopes::reference(&f.bot).unwrap()).unwrap()};
    db.collection::<Document>(PLANS)
        .insert_one(&row)
        .await
        .unwrap();
    let result = transaction_service::run_db_only(db, &(db, &f.scope), |tx, &(db, scope)| {
        Box::pin(async move { erase_page(db, tx, scope, PLANS, exact(scope)?, 9).await })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(
        db.collection::<Document>(PLANS)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap(),
        row
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn invalid_scope_completion_proof_prevents_every_provider_or_local_effect() {
    for proof in [
        doc! {"local_data_erased_at":"malformed","completed_grant_id":"wrong"},
        doc! {"completed_grant_id":42},
    ] {
        let f = fixture().await;
        let db = &f.state.db;
        db.collection::<Document>(GRANT_SCOPES)
            .update_one(
                doc! {"_id":f.scope.get("_id").cloned()},
                doc! {"$set":proof},
            )
            .await
            .unwrap();
        let before = db
            .collection::<Document>(GRANT_SCOPES)
            .find_one(doc! {"_id":f.scope.get("_id").cloned()})
            .await
            .unwrap()
            .unwrap();
        let result = pass(
            &f.state,
            &MetaService::from(&f.state),
            &f.bot.id,
            &f.bot.meta_binding.as_ref().unwrap().grant_id,
            bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::seconds(1)),
        )
        .await;
        assert!(result.is_err());
        assert!(f.server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            db.collection::<Document>(GRANT_SCOPES)
                .find_one(doc! {"_id":f.scope.get("_id").cloned()})
                .await
                .unwrap()
                .unwrap(),
            before
        );
        assert!(
            db.collection::<ChannelBot>(BOTS)
                .find_one(doc! {"_id":&f.bot.id,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn original_parent_provenance_audit_pages_include_non_null_tags_without_hints() {
    let f = fixture().await;
    let db = &f.state.db;
    ensure_indexes(db).await.unwrap();
    let sibling = uuid::Uuid::new_v4().to_string();
    let rows:Vec<_>=(0..12_000).map(|number|doc! {
        "_id":format!("00000000-0000-4000-8000-{number:012x}"),"channel_bot_id":if number<1001 {&f.bot.id} else {&sibling},
        "logical_key":format!("meta-audit-{number}"),"user_id":&f.bot.user_id,"source_id":"expired-source",
        "meta_grant":if number%2==0 {Bson::String("malformed non-null tag".into())} else {bson::to_bson(&scopes::reference(&f.bot).unwrap()).unwrap()},
    }).collect();
    db.collection::<Document>(PLANS)
        .insert_many(rows)
        .await
        .unwrap();
    for after in [None, Some("00000000-0000-4000-8000-00000000012c")] {
        let mut filter = doc! {"channel_bot_id":&f.bot.id};
        if let Some(after) = after {
            filter.insert("_id", doc! {"$gt":after});
        }
        let explain=db.run_command(doc! {"explain":{"find":PLANS,"filter":filter,"sort":{"_id":1},"limit":100,
            "projection":{"_id":1,"meta_grant":1,"channel_bot_id":1,"user_id":1,"source_id":1}},"verbosity":"executionStats"}).await.unwrap();
        let stats = explain.get_document("executionStats").unwrap();
        let count = |field| {
            stats
                .get_i64(field)
                .unwrap_or_else(|_| i64::from(stats.get_i32(field).unwrap()))
        };
        assert_eq!(count("nReturned"), 100);
        assert!(
            count("totalDocsExamined") <= 101 && count("totalKeysExamined") <= 102,
            "{stats:?}"
        );
        eprintln!(
            "unhinted Meta original-parent audit: returned {}, documents {}, keys {}",
            count("nReturned"),
            count("totalDocsExamined"),
            count("totalKeysExamined")
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn expired_source_receipts_require_the_retained_scope_range_before_audit_or_deletion() {
    for generation in [2_i64, 4, 6] {
        let f = fixture().await;
        let db = &f.state.db;
        let mut scope = f.scope.clone();
        scope.insert("from_generation", 3_i64);
        scope.insert("to_generation", 5_i64);
        scope.insert("status", "sealed");
        db.collection::<Document>(GRANT_SCOPES)
            .replace_one(doc! {"_id":scope.get("_id").cloned()}, &scope)
            .await
            .unwrap();
        let mut proof = scopes::reference(&f.bot).unwrap();
        proof.generation = generation;
        let row = doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_bot_id":&f.bot.id,
        "user_id":&f.bot.user_id,"source_id":"expired-original-inbound",
        "meta_grant":bson::to_bson(&proof).unwrap()};
        let mut b = proof.clone();
        b.grant_id = uuid::Uuid::new_v4().to_string();
        b.generation = 6;
        let mut b_scope = scope.clone();
        b_scope.insert("_id", uuid::Uuid::new_v4().to_string());
        b_scope.insert("grant_id", &b.grant_id);
        b_scope.insert("from_generation", 6_i64);
        b_scope.insert("to_generation", Bson::Null);
        b_scope.insert("status", "current");
        db.collection::<Document>(GRANT_SCOPES)
            .insert_one(b_scope)
            .await
            .unwrap();
        let mut sibling = row.clone();
        sibling.insert("_id", uuid::Uuid::new_v4().to_string());
        sibling.insert("meta_grant", bson::to_bson(&b).unwrap());
        let plans = db.collection::<Document>(PLANS);
        plans
            .insert_many([row.clone(), sibling.clone()])
            .await
            .unwrap();
        let audited = transaction_service::run_db_only(db, &(db, &scope), |tx, &(db, scope)| {
            Box::pin(async move { attribute(db, tx, scope, &doc! {"phase":2}, 2).await })
        })
        .await
        .unwrap();
        assert_eq!(audited.repair, generation != 4);
        let result = transaction_service::run_db_only(db, &(db, &scope), |tx, &(db, scope)| {
            Box::pin(async move { erase_page(db, tx, scope, PLANS, exact(scope)?, 9).await })
        })
        .await;
        assert_eq!(result.is_ok(), generation == 4);
        assert_eq!(
            plans
                .find_one(doc! {"_id":row.get("_id").cloned()})
                .await
                .unwrap()
                .is_none(),
            generation == 4
        );
        assert_eq!(
            plans
                .find_one(doc! {"_id":sibling.get("_id").cloned()})
                .await
                .unwrap()
                .unwrap(),
            sibling
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn final_positive_inventory_rewinds_behind_the_scope_fence_and_finishes_bounded_stragglers() {
    let f = fixture().await;
    let db = &f.state.db;
    let proof = scopes::reference(&f.bot).unwrap();
    db.collection::<Document>(BOTS)
        .update_one(
            doc! {"_id":&f.bot.id},
            doc! {"$set":{"is_active":false,"meta_binding.subscription":"removed"}},
        )
        .await
        .unwrap();
    db.collection::<Document>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":f.scope.get("_id").cloned()},
            doc! {"$set":{"erasure_requested":true,"purge":{"phase":19}}},
        )
        .await
        .unwrap();
    let rows: Vec<_> = (0..101).map(|_| doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_bot_id":&f.bot.id,
        "user_id":&f.bot.user_id,"source_id":"expired-inbound","meta_grant":bson::to_bson(&proof).unwrap()}).collect();
    db.collection::<Document>(PLANS)
        .insert_many(rows)
        .await
        .unwrap();
    let service = MetaService::from(&f.state);
    let cutoff = bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::seconds(1));
    assert!(
        !pass(&f.state, &service, &f.bot.id, &proof.grant_id, cutoff)
            .await
            .unwrap()
    );
    let rewound = db
        .collection::<Document>(GRANT_SCOPES)
        .find_one(doc! {"_id":f.scope.get("_id").cloned()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rewound.get_document("purge").unwrap().get_i32("phase"),
        Ok(9)
    );
    assert!(!grants::completed(&rewound, &proof.grant_id).unwrap());
    assert!(
        db.collection::<Document>(BOTS)
            .find_one(doc! {"_id":&f.bot.id})
            .await
            .unwrap()
            .is_some()
    );
    // The next transaction deletes at most one page, and keeps the parent and
    // exact authority available for a restart before the remaining row.
    assert!(
        !pass(&f.state, &service, &f.bot.id, &proof.grant_id, cutoff)
            .await
            .unwrap()
    );
    assert_eq!(
        db.collection::<Document>(PLANS)
            .count_documents(doc! {"channel_bot_id":&f.bot.id})
            .await
            .unwrap(),
        1
    );
    let mut done = false;
    for _ in 0..32 {
        if pass(&f.state, &service, &f.bot.id, &proof.grant_id, cutoff)
            .await
            .unwrap()
        {
            done = true;
            break;
        }
    }
    assert!(done);
    assert!(
        grants::completion_for(db, &f.bot.id, &proof.grant_id)
            .await
            .unwrap()
    );
    assert!(
        db.collection::<Document>(BOTS)
            .find_one(doc! {"_id":&f.bot.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn final_inventory_revisits_nested_queued_and_active_provenance_before_completion() {
    for location in [
        "pending",
        "active-direct",
        "active-events",
        "reply-preference",
    ] {
        let f = fixture().await;
        let db = &f.state.db;
        let proof = scopes::reference(&f.bot).unwrap();
        let mut parent = conversation(&f.bot.user_id, false, &proof);
        let context = doc! {"meta_grant":bson::to_bson(&proof).unwrap()};
        let event = doc! {"channel_contexts":[&context],"text":"retained inbox input"};
        match location {
            "reply-preference" => {
                parent.insert("reply_channel", doc! {"nyxbot_channel_id":"fixture-channel","platform":"messenger","partition":"chat"});
                parent.insert("reply_channel_meta_grant", bson::to_bson(&proof).unwrap());
            }
            "pending" => {
                parent.insert("pending_events", vec![event]);
            }
            "active-direct" => {
                parent.insert(
                    "active_turn",
                    doc! {"turn_id":"a-turn","channel_context":context},
                );
            }
            _ => {
                parent.insert("active_turn", doc! {"turn_id":"a-turn","events":[event]});
            }
        }
        db.collection::<Document>("assistant_conversations")
            .insert_one(&parent)
            .await
            .unwrap();
        db.collection::<Document>(BOTS)
            .update_one(
                doc! {"_id":&f.bot.id},
                doc! {"$set":{"is_active":false,"meta_binding.subscription":"removed"}},
            )
            .await
            .unwrap();
        let result = transaction_service::run_db_only(db, &(db, &f.scope), |tx, &(db, scope)| {
            Box::pin(finish(db, tx, scope))
        })
        .await
        .unwrap();
        assert_eq!(result.progress.get_i32("phase"), Ok(4));
        assert!(!result.done);
        assert!(
            !grants::completion_for(db, &f.bot.id, &proof.grant_id)
                .await
                .unwrap()
        );
        assert!(
            db.collection::<Document>(BOTS)
                .find_one(doc! {"_id":&f.bot.id})
                .await
                .unwrap()
                .is_some()
        );
        let next = transaction_service::run_db_only(db, &(db, &f.scope), |tx, &(db, scope)| {
            Box::pin(erase_queued(db, tx, scope, 4))
        })
        .await
        .unwrap();
        assert!(!next.done);
        assert!(
            db.collection::<Document>("assistant_conversations")
                .find_one(queued_filter(&f.bot.id, &proof.grant_id))
                .await
                .unwrap()
                .is_none()
        );
        if location == "reply-preference" {
            let homes = db.collection::<Document>("assistant_conversations");
            let id = parent.get_str("_id").unwrap();
            let cleared = homes.find_one(doc! {"_id": id}).await.unwrap().unwrap();
            assert_eq!(cleared.get("reply_channel"), Some(&Bson::Null));
            let mut successor = proof.clone();
            successor.grant_id = uuid::Uuid::new_v4().to_string();
            let preference = doc! {"reply_channel":parent.get("reply_channel").unwrap(), "reply_channel_meta_grant":bson::to_bson(&successor).unwrap()};
            homes
                .update_one(doc! {"_id":id}, doc! {"$set":&preference})
                .await
                .unwrap();
            transaction_service::run_db_only(db, &(db, &f.scope), |tx, &(db, scope)| {
                Box::pin(erase_queued(db, tx, scope, 4))
            })
            .await
            .unwrap();
            assert_eq!(homes.count_documents(preference).await.unwrap(), 1);
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn exact_turn_feedback_and_artifact_deliveries_drain_before_source_and_keep_successor_head() {
    use crate::services::assistant_feedback_service::{self as feedback, FeedbackWrite};
    for successor_head in [false, true] {
        let f = fixture().await;
        let db = &f.state.db;
        let proof = scopes::reference(&f.bot).unwrap();
        let mut parent = conversation(&f.bot.user_id, false, &proof);
        let conversation = parent.get_str("_id").unwrap().to_owned();
        let mut web = message(
            &f.bot.user_id,
            &conversation,
            "web-turn",
            3,
            "assistant",
            &proof,
        );
        web.remove("meta_grants");
        let a = message(
            &f.bot.user_id,
            &conversation,
            "a-turn",
            2,
            "assistant",
            &proof,
        );
        let delivery_a = uuid::Uuid::new_v4().to_string();
        let delivery_b = uuid::Uuid::new_v4().to_string();
        parent.insert(
            "artifact_head",
            doc! {"message_id":if successor_head {&delivery_b}else{&delivery_a},"seq":3_i64},
        );
        db.collection::<Document>("assistant_conversations")
            .insert_one(parent)
            .await
            .unwrap();
        db.collection::<Document>("assistant_messages")
            .insert_many([a.clone(), web.clone()])
            .await
            .unwrap();
        let write = |id: &str, turn: &str| FeedbackWrite {
            user_id: f.bot.user_id.clone(),
            message_id: id.into(),
            check_source_grant: true,
            conversation_id: Some(conversation.clone()),
            group_id: None,
            turn_id: Some(turn.into()),
            vote: "up".into(),
            comment: Some(format!("Private {turn} feedback")),
            comment_present: true,
            context: None,
        };
        feedback::upsert_feedback(db, write(a.get_str("_id").unwrap(), "a-turn"))
            .await
            .unwrap();
        feedback::upsert_feedback(db, write(web.get_str("_id").unwrap(), "web-turn"))
            .await
            .unwrap();
        let deliveries = db.collection::<Document>(crate::models::generated_artifact::DELIVERIES);
        for (id, origin, event) in [
            (&delivery_a, "a-turn", "artifact-event-a"),
            (&delivery_b, "web-turn", "artifact-event-b"),
        ] {
            deliveries
                .insert_one(
                    doc! {"_id":id,"user_id":&f.bot.user_id,"conversation_id":&conversation,
                    "originating_turn_id":origin,"event_turn_id":event,"attachment_ids":[]},
                )
                .await
                .unwrap();
        }
        db.collection::<Document>(BOTS)
            .update_one(
                doc! {"_id":&f.bot.id},
                doc! {"$set":{"is_active":false,"meta_binding.subscription":"removed"}},
            )
            .await
            .unwrap();
        crate::services::durable_wake_tests::resume_after(
            "meta_feedback_before_write",
            async {
                assert!(
                    feedback::upsert_feedback(db, write(a.get_str("_id").unwrap(), "a-turn"))
                        .await
                        .is_err()
                );
            },
            async {
                let service = MetaService::from(&f.state);
                let cutoff =
                    bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::seconds(1));
                let mut done = false;
                for _ in 0..64 {
                    if pass(&f.state, &service, &f.bot.id, &proof.grant_id, cutoff)
                        .await
                        .unwrap()
                    {
                        done = true;
                        break;
                    }
                }
                assert!(done);
            },
        )
        .await;
        let feedback_rows =
            db.collection::<Document>(crate::models::assistant_message_feedback::COLLECTION_NAME);
        assert_eq!(
            feedback_rows
                .count_documents(doc! {"conversation_id":&conversation,"turn_id":"a-turn"})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            feedback_rows
                .count_documents(doc! {"conversation_id":&conversation,"turn_id":"web-turn"})
                .await
                .unwrap(),
            1
        );
        assert!(
            deliveries
                .find_one(doc! {"_id":&delivery_a})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            deliveries
                .find_one(doc! {"_id":&delivery_b})
                .await
                .unwrap()
                .is_some()
        );
        let retained = db
            .collection::<Document>("assistant_conversations")
            .find_one(doc! {"_id":&conversation})
            .await
            .unwrap()
            .unwrap();
        if successor_head {
            assert_eq!(
                retained
                    .get_document("artifact_head")
                    .unwrap()
                    .get_str("message_id"),
                Ok(delivery_b.as_str())
            );
        } else {
            assert!(!retained.contains_key("artifact_head"));
        }
        assert_eq!(
            db.collection::<Document>("assistant_messages")
                .find_one(doc! {"_id":web.get("_id").cloned()})
                .await
                .unwrap()
                .unwrap(),
            web
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn large_guest_history_audits_and_destroys_in_restartable_pages_before_parent_retirement() {
    let f = fixture().await;
    let db = &f.state.db;
    ensure_indexes(db).await.unwrap();
    let grant = scopes::reference(&f.bot).unwrap();
    let mut guest = conversation(&f.bot.user_id, true, &grant);
    guest.insert("message_count", 1001_i64);
    let unrelated = conversation(&f.bot.user_id, false, &grant);
    let unrelated_id = unrelated.get_str("_id").unwrap().to_owned();
    let conversation = guest.get_str("_id").unwrap().to_owned();
    db.collection::<Document>("assistant_conversations")
        .insert_many([guest, unrelated])
        .await
        .unwrap();
    let messages: Vec<_> = (0..1001)
        .map(|number| {
            message(
                &f.bot.user_id,
                &conversation,
                &format!("a-turn-{number}"),
                number + 1,
                "user",
                &grant,
            )
        })
        .collect();
    db.collection::<Document>("assistant_messages")
        .insert_many(messages)
        .await
        .unwrap();
    let tools:Vec<_>=(0..1001).map(|number|doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,
        "conversation_id":&conversation,"turn_id":format!("a-turn-{number}"),"output":"private guest result"}).collect();
    db.collection::<Document>(crate::models::assistant_tool_result::COLLECTION_NAME)
        .insert_many(tools)
        .await
        .unwrap();
    db.collection::<Document>(BOTS)
        .update_one(
            doc! {"_id":&f.bot.id},
            doc! {"$set":{"is_active":false,"meta_binding.subscription":"removed"}},
        )
        .await
        .unwrap();
    let cutoff = bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::seconds(1));
    let mut last_count = 2002_u64;
    let mut partial = false;
    let mut done = false;
    for _ in 0..192 {
        // Every pass constructs a new worker and reloads the only durable
        // cursor from the scope. No local audit/proof survives a restart.
        done = pass(
            &f.state,
            &MetaService::from(&f.state),
            &f.bot.id,
            &grant.grant_id,
            cutoff,
        )
        .await
        .unwrap();
        let messages = db
            .collection::<Document>("assistant_messages")
            .count_documents(doc! {"conversation_id":&conversation})
            .await
            .unwrap();
        let tools = db
            .collection::<Document>(crate::models::assistant_tool_result::COLLECTION_NAME)
            .count_documents(doc! {"conversation_id":&conversation})
            .await
            .unwrap();
        let count = messages + tools;
        assert!(
            last_count - count <= 100,
            "one destructive transaction exceeded its page"
        );
        if count > 0 {
            assert!(
                db.collection::<Document>("assistant_conversations")
                    .find_one(doc! {"_id":&conversation})
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                !grants::completion_for(db, &f.bot.id, &grant.grant_id)
                    .await
                    .unwrap()
            );
        }
        if count < 2002 && count > 0 {
            partial = true;
            let stored = db
                .collection::<Document>(GRANT_SCOPES)
                .find_one(doc! {"_id":f.scope.get("_id").cloned()})
                .await
                .unwrap()
                .unwrap();
            assert!(
                stored
                    .get_document("purge")
                    .unwrap()
                    .get_i32("guest_delete_stage")
                    .is_ok()
            );
        }
        last_count = count;
        if done {
            break;
        }
    }
    assert!(done && partial);
    assert_eq!(last_count, 0);
    assert!(
        db.collection::<Document>("assistant_conversations")
            .find_one(doc! {"_id":&conversation})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>("assistant_conversations")
            .find_one(doc! {"_id":&unrelated_id})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        grants::completion_for(db, &f.bot.id, &grant.grant_id)
            .await
            .unwrap()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn manual_worker_losing_its_parent_requires_exact_typed_completion_proof() {
    let f = fixture().await;
    let db = &f.state.db;
    let mut captured = f.bot.clone();
    captured.is_active = false;
    db.collection::<Document>(BOTS)
        .delete_one(doc! {"_id":&captured.id})
        .await
        .unwrap();
    assert!(manual_pass(&f.state, &captured).await.is_err());
    let scopes_rows = db.collection::<Document>(GRANT_SCOPES);
    scopes_rows.update_one(doc! {"_id":f.scope.get("_id").cloned()},doc! {"$set":{"status":"completed",
        "local_data_erased_at":"not a date","completed_grant_id":&captured.meta_binding.as_ref().unwrap().grant_id}}).await.unwrap();
    let corrupt = scopes_rows
        .find_one(doc! {"_id":f.scope.get("_id").cloned()})
        .await
        .unwrap()
        .unwrap();
    assert!(manual_pass(&f.state, &captured).await.is_err());
    assert_eq!(
        scopes_rows
            .find_one(doc! {"_id":f.scope.get("_id").cloned()})
            .await
            .unwrap()
            .unwrap(),
        corrupt
    );
    scopes_rows
        .replace_one(doc! {"_id":f.scope.get("_id").cloned()}, &f.scope)
        .await
        .unwrap();
    let grant = &captured.meta_binding.as_ref().unwrap().grant_id;
    transaction_service::run_db_only(
        db,
        &(db, captured.id.as_str(), grant.as_str()),
        |tx, &(db, bot, grant)| Box::pin(scopes::mark_complete(db, tx, bot, grant)),
    )
    .await
    .unwrap();
    assert!(manual_pass(&f.state, &captured).await.unwrap());
    captured.meta_binding.as_mut().unwrap().grant_id = uuid::Uuid::new_v4().to_string();
    assert!(manual_pass(&f.state, &captured).await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_turn_audits_every_message_before_descendants_and_preserves_contradictory_proof() {
    for kind in [
        "mixed",
        "foreign",
        "out-of-range",
        "malformed",
        "untagged",
        "after-page",
    ] {
        let f = fixture().await;
        let db = &f.state.db;
        let a = scopes::reference(&f.bot).unwrap();
        let parent = conversation(&f.bot.user_id, false, &a);
        let conversation = parent.get_str("_id").unwrap();
        db.collection::<Document>("assistant_conversations")
            .insert_one(&parent)
            .await
            .unwrap();
        let mut input = message(&f.bot.user_id, conversation, "a-turn", 1, "user", &a);
        input.insert("_id", "00000000-0000-4000-8000-000000000000");
        let mut reply = message(&f.bot.user_id, conversation, "a-turn", 2, "assistant", &a);
        reply.insert("_id", "ffffffff-0000-4000-8000-000000000000");
        let mut b = a.clone();
        b.grant_id = uuid::Uuid::new_v4().to_string();
        match kind {
            "mixed" => {
                reply.insert("meta_grants", bson::to_bson(&vec![a.clone(), b]).unwrap());
            }
            "foreign" => {
                reply.insert("meta_grants", bson::to_bson(&vec![b]).unwrap());
            }
            "out-of-range" => {
                b = a.clone();
                b.generation = -1;
                reply.insert("meta_grants", bson::to_bson(&vec![b]).unwrap());
            }
            "malformed" => {
                reply.insert("meta_grants", "invalid proof");
            }
            _ => {
                reply.remove("meta_grants");
            }
        }
        db.collection::<Document>("assistant_messages")
            .insert_many([input.clone(), reply.clone()])
            .await
            .unwrap();
        if kind == "after-page" {
            let filler: Vec<_> = (1..100)
                .map(|i| {
                    let mut row = message(
                        &f.bot.user_id,
                        conversation,
                        "a-turn",
                        i + 2,
                        "assistant",
                        &a,
                    );
                    row.insert("_id", format!("{i:08x}-0000-4000-8000-000000000000"));
                    row
                })
                .collect();
            db.collection::<Document>("assistant_messages")
                .insert_many(filler)
                .await
                .unwrap();
        }
        let tool = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,"conversation_id":conversation,
        "turn_id":"a-turn","output":"Private original result"};
        db.collection::<Document>(crate::models::assistant_tool_result::COLLECTION_NAME)
            .insert_one(&tool)
            .await
            .unwrap();
        let mut progress = Document::new();
        if kind == "after-page" {
            let step = transaction_service::run_db_only(db, &(db, &f.scope), |tx, &(db, scope)| {
                Box::pin(async move { erase_turn(db, tx, scope, &Document::new(), 6).await })
            })
            .await
            .unwrap();
            assert_eq!(step.progress.get_bool("turn_audited").ok(), None);
            assert!(step.progress.get_str("after").is_ok());
            progress = step.progress;
        }
        let result = transaction_service::run_db_only(
            db,
            &(db, &f.scope, &progress),
            |tx, &(db, scope, progress)| Box::pin(erase_turn(db, tx, scope, progress, 6)),
        )
        .await;
        assert!(result.is_err());
        for row in [&input, &reply] {
            assert_eq!(
                db.collection::<Document>("assistant_messages")
                    .find_one(doc! {"_id":row.get("_id").cloned()})
                    .await
                    .unwrap()
                    .unwrap(),
                *row
            );
        }
        assert_eq!(
            db.collection::<Document>(crate::models::assistant_tool_result::COLLECTION_NAME)
                .find_one(doc! {"_id":tool.get("_id").cloned()})
                .await
                .unwrap()
                .unwrap(),
            tool
        );
        assert_eq!(
            db.collection::<Document>("assistant_conversations")
                .find_one(doc! {"_id":conversation})
                .await
                .unwrap()
                .unwrap(),
            parent
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn shared_turn_erasure_removes_only_notifications_whose_current_source_still_names_a() {
    let f = fixture().await;
    let db = &f.state.db;
    let a = scopes::reference(&f.bot).unwrap();
    let parent = conversation(&f.bot.user_id, false, &a);
    let conversation = parent.get_str("_id").unwrap();
    db.collection::<Document>("assistant_conversations")
        .insert_one(&parent)
        .await
        .unwrap();
    let input = message(&f.bot.user_id, conversation, "a-turn", 1, "user", &a);
    db.collection::<Document>("assistant_messages")
        .insert_one(input)
        .await
        .unwrap();
    let notifications =
        db.collection::<Document>(crate::models::assistant_notification::COLLECTION_NAME);
    let old = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,"source":{
    "conversation_id":conversation,"turn_id":"a-turn","message_id":"a-reply"}};
    let mut coalesced = old.clone();
    coalesced.insert("_id", uuid::Uuid::new_v4().to_string());
    coalesced
        .get_document_mut("source")
        .unwrap()
        .insert("turn_id", "b-turn");
    coalesced
        .get_document_mut("source")
        .unwrap()
        .insert("message_id", "b-reply");
    notifications
        .insert_many([old.clone(), coalesced.clone()])
        .await
        .unwrap();
    let mut progress = Document::new();
    for _ in 0..8 {
        let step = transaction_service::run_db_only(
            db,
            &(db, &f.scope, &progress),
            |tx, &(db, scope, progress)| Box::pin(erase_turn(db, tx, scope, progress, 6)),
        )
        .await
        .unwrap();
        progress = step.progress;
        if progress.get_i32("phase") == Ok(7) {
            break;
        }
    }
    assert!(
        notifications
            .find_one(doc! {"_id":old.get("_id").cloned()})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        notifications
            .find_one(doc! {"_id":coalesced.get("_id").cloned()})
            .await
            .unwrap()
            .unwrap(),
        coalesced
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_turn_erases_real_cards_and_bindings_and_refuses_late_publication() {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_service_execution as execution,
    };
    let f = fixture().await;
    let db = &f.state.db;
    crate::services::assistant_nyxagent::ensure_indexes(db)
        .await
        .unwrap();
    let row = crate::services::assistant_authority_tests::new_orchestrator(
        &f.state,
        &f.bot.user_id,
        "Private Meta question",
    )
    .await;
    let turn = &row.active_turn.as_ref().unwrap().turn_id;
    let proof = scopes::reference(&f.bot).unwrap();
    db.collection::<Document>("assistant_messages")
        .update_many(
            doc! {"conversation_id":&row.id,"turn_id":turn},
            doc! {"$set":{"meta_grants":[bson::to_bson(&proof).unwrap()],"meta_contexts":[bson::to_bson(&card_sources::context(&f.bot)).unwrap()]}},
        )
        .await
        .unwrap();
    let chat =
        crate::services::assistant_authority_tests::chat_for_row(&f.state, &f.bot.user_id, &row)
            .await;
    let args = serde_json::json!({"private":"operation"});
    let request = || acks::Request {
        kind: "action",
        service: None,
        tool: Some("private_operation"),
        arguments: Some(&args),
        summary: "Private Meta approval summary",
        platform: false,
    };
    let card = acks::request(db, &chat, request()).await.unwrap();
    assert_eq!(card.creation_turn_id.as_deref(), Some(turn.as_str()));
    let notification = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,"kind":"needs_decision",
    "source":{"card_id":&card.id,"conversation_id":&row.id,"turn_id":"different-creation-turn"}};
    db.collection::<Document>(crate::models::assistant_notification::COLLECTION_NAME)
        .insert_one(&notification)
        .await
        .unwrap();
    let binding = doc! {"kind":"card_binding","user_id":&f.bot.user_id,"conversation_id":&row.id,
    "api_key_id":&chat.api_key_id,"turn_id":turn,"operation":{"private":"binding"}};
    execution::publish_card_binding(db, &chat, &card.id, binding.clone())
        .await
        .unwrap();
    let independent = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.bot.user_id,"conversation_id":&row.id,
    "creation_turn_id":"independent-web-turn","requested_turn_id":"independent-web-turn","summary":"Web card"};
    db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .insert_one(&independent)
        .await
        .unwrap();
    db.collection::<Document>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":f.scope.get("_id").cloned()},
            doc! {"$set":{"erasure_requested":true}},
        )
        .await
        .unwrap();
    assert!(acks::request(db, &chat, request()).await.is_err());
    let mut progress = Document::new();
    for _ in 0..12 {
        let step = transaction_service::run_db_only(
            db,
            &(db, &f.scope, &progress),
            |tx, &(db, scope, progress)| Box::pin(erase_turn(db, tx, scope, progress, 6)),
        )
        .await
        .unwrap();
        progress = step.progress;
        if progress.get_i32("phase") == Ok(7) {
            break;
        }
    }
    assert_eq!(progress.get_i32("phase"), Ok(7));
    assert!(
        db.collection::<Document>(crate::models::assistant_notification::COLLECTION_NAME)
            .find_one(doc! {"_id":notification.get("_id").cloned()})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .find_one(doc! {"_id":&card.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>(execution::COLLECTION)
            .find_one(doc! {"_id":&card.id})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .find_one(doc! {"_id":independent.get("_id").cloned()})
            .await
            .unwrap()
            .unwrap(),
        independent
    );
    assert!(
        execution::publish_card_binding(db, &chat, &card.id, binding)
            .await
            .is_err()
    );
    assert!(acks::request(db, &chat, request()).await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn contradictory_setup_link_proofs_remain_pending_in_both_deletion_orders() {
    let f = fixture().await;
    let db = &f.state.db;
    let a = scopes::reference(&f.bot).unwrap();
    let mut b = a.clone();
    b.grant_id = uuid::Uuid::new_v4().to_string();
    let mut b_scope = f.scope.clone();
    b_scope.insert("grant_id", &b.grant_id);
    let rows = db.collection::<Document>(crate::models::channel_connect_link::COLLECTION_NAME);
    for proof in [&a, &b] {
        let connection = if proof == &a {
            &b.grant_id
        } else {
            &a.grant_id
        };
        let link = doc! {"_id":uuid::Uuid::new_v4().to_string(),"bot_id":&f.bot.id,"flow":"meta_oauth",
        "connection_id":connection,"meta_grant":bson::to_bson(proof).unwrap()};
        rows.insert_one(&link).await.unwrap();
        for scope in [&f.scope, &b_scope] {
            let grant = scope.get_str("grant_id").unwrap();
            let filter = setup_links(&f.bot.id, grant);
            assert!(
                transaction_service::run_db_only(
                    db,
                    &(db, scope, &filter),
                    |tx, &(db, scope, filter)| {
                        Box::pin(erase_page(
                            db,
                            tx,
                            scope,
                            crate::models::channel_connect_link::COLLECTION_NAME,
                            filter.clone(),
                            17,
                        ))
                    }
                )
                .await
                .is_err()
            );
            assert_eq!(
                rows.find_one(doc! {"_id":link.get("_id").cloned()})
                    .await
                    .unwrap()
                    .unwrap(),
                link
            );
        }
        rows.delete_one(doc! {"_id":link.get("_id").cloned()})
            .await
            .unwrap();
    }
    db.drop().await.unwrap();
}

fn async_catalog() -> crate::services::mcp_service::McpToolService {
    use crate::services::mcp_service::{McpToolEndpoint, McpToolService, McpToolSource};
    let contract=bson::from_document(doc! {"status_operation":"status","result_operation":"result",
        "id_field":"/id","id_parameter":"id","status_field":"/status","success_states":["done"],"failure_states":["failed"]}).unwrap();
    let submit = McpToolEndpoint {
        endpoint_id: "submit".into(),
        name: "submit".into(),
        method: "POST".into(),
        path: "/jobs".into(),
        async_operation: Some(contract),
        ..Default::default()
    };
    let endpoints = vec![
        submit,
        McpToolEndpoint {
            name: "status".into(),
            endpoint_id: "status".into(),
            method: "GET".into(),
            path: "/jobs/{id}".into(),
            parameters: Some(serde_json::json!([{"name":"id","in":"path","required":true}])),
            ..Default::default()
        },
        McpToolEndpoint {
            name: "result".into(),
            endpoint_id: "result".into(),
            method: "GET".into(),
            path: "/jobs/{id}/result".into(),
            parameters: Some(serde_json::json!([{"name":"id","in":"path","required":true}])),
            ..Default::default()
        },
    ];
    McpToolService {
        workspace_destinations_pending: false,
        service_id: "test-service".into(),
        service_name: "Test service".into(),
        service_slug: "test-service".into(),
        description: None,
        service_category: "other".into(),
        endpoints,
        durable_endpoint_metadata: Default::default(),
        source: McpToolSource::Internal,
        executable: true,
        is_generic_proxy: false,
        invalid_openapi_contract: false,
        recommended_skills: Vec::new(),
        recommended_skill_refs: None,
        skills_revision: None,
        proxy_operation_policy: None,
    }
}

async fn finish_source_turn(state: &crate::AppState, row: &AssistantConversation, text: &str) {
    use crate::services::assistant_nyxagent as engine;
    let next = engine::finish_turn(
        &state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: text.into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    assert!(next.is_none());
}

#[tokio::test]
async fn async_result_inherits_original_grant_through_reserve_queue_input_reply_and_erasure() {
    use crate::models::assistant_conversation::{ChannelTurnContext, TurnOrigin};
    use crate::services::{assistant_nyxagent as engine, async_service_operation as watches};
    let f = fixture().await;
    let db = &f.state.db;
    engine::ensure_indexes(db).await.unwrap();
    let catalog = async_catalog();
    let proof = scopes::reference(&f.bot).unwrap();
    let context:ChannelTurnContext=bson::from_document(doc! {"meta_grant":bson::to_bson(&proof).unwrap(),
        "source_event_id":uuid::Uuid::new_v4().to_string(),"channel_id":uuid::Uuid::new_v4().to_string(),"channel_bot_id":&f.bot.id,
        "conversation_kind":"private","sender_kind":"human","guest":false,"original_message":true,
        "display_name":"Private display label","conversation_label":"Private chat label"}).unwrap();
    let mut home = None;
    let mut meta_turns = Vec::new();
    let mut web_turns = Vec::new();
    let mut meta_ready = None;
    for meta in [false, true] {
        let request:engine::TurnRequest=serde_json::from_value(serde_json::json!({"conversation_id":home,"text":if meta {"Private Meta task"} else {"Independent web task"}})).unwrap();
        let mut start = engine::TurnStart::from(&request);
        if meta {
            start.origin = TurnOrigin::Channel;
            start.channel_context = Some(context.clone());
        }
        let original = engine::begin_turn(db, &f.bot.user_id, start, &f.state.encryption_keys)
            .await
            .unwrap();
        home = Some(original.id.clone());
        let chat = crate::services::assistant_authority_tests::chat_for_row(
            &f.state,
            &f.bot.user_id,
            &original,
        )
        .await;
        let watch = watches::reserve(db, &chat, &catalog, &catalog.endpoints[0])
            .await
            .unwrap();
        assert_eq!(watch.meta_contexts.len(), usize::from(meta));
        if meta {
            assert_eq!(watch.meta_contexts[0].meta_grant.as_ref(), Some(&proof));
            assert!(watch.meta_contexts[0].display_name.is_none());
            assert!(watch.meta_contexts[0].conversation_label.is_none());
        }
        let result = if meta {
            "Private asynchronous result"
        } else {
            "Independent web result"
        };
        watches::ready(
            db,
            &watch,
            Some(&f.state.encryption_keys),
            "completed",
            Some(result),
        )
        .await
        .unwrap();
        let ready = db
            .collection::<crate::models::async_service_operation::AsyncServiceOperation>(
                crate::models::async_service_operation::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap();
        assert!(watches::enqueue(db, &ready).await.unwrap());
        finish_source_turn(&f.state, &original, "Submitted operation").await;
        let request: engine::TurnRequest = serde_json::from_value(
            serde_json::json!({"conversation_id":&original.id,"text":"Read completed operation"}),
        )
        .unwrap();
        let mut start = engine::TurnStart::from(&request);
        start.origin = TurnOrigin::Event;
        let delivered = engine::begin_turn(db, &f.bot.user_id, start, &f.state.encryption_keys)
            .await
            .unwrap();
        assert!(
            watches::input_context(&f.state, &delivered)
                .await
                .unwrap()
                .contains(result)
        );
        let turn = &delivered.active_turn.as_ref().unwrap().turn_id;
        let input = db
            .collection::<Document>("assistant_messages")
            .find_one(doc! {"conversation_id":&original.id,"turn_id":turn,"role":"event"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(input.contains_key("meta_grants"), meta);
        finish_source_turn(&f.state, &delivered, result).await;
        if meta {
            meta_turns.extend([
                original.active_turn.as_ref().unwrap().turn_id.clone(),
                turn.clone(),
            ]);
            meta_ready = Some(ready);
        } else {
            web_turns.extend([
                original.active_turn.as_ref().unwrap().turn_id.clone(),
                turn.clone(),
            ]);
        }
    }
    let home = home.unwrap();
    db.collection::<Document>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":f.scope.get("_id").cloned()},
            doc! {"$set":{"erasure_requested":true}},
        )
        .await
        .unwrap();
    assert!(!watches::enqueue(db, &meta_ready.unwrap()).await.unwrap());
    let mut progress = Document::new();
    for _ in 0..32 {
        let step = transaction_service::run_db_only(
            db,
            &(db, &f.scope, &progress),
            |tx, &(db, scope, progress)| Box::pin(erase_turn(db, tx, scope, progress, 6)),
        )
        .await
        .unwrap();
        progress = step.progress;
        if progress.get_i32("phase") == Ok(7) {
            break;
        }
    }
    assert_eq!(progress.get_i32("phase"), Ok(7));
    let messages = db.collection::<Document>("assistant_messages");
    for turn in meta_turns {
        assert_eq!(
            messages
                .count_documents(doc! {"conversation_id":&home,"turn_id":turn})
                .await
                .unwrap(),
            0
        );
    }
    for turn in web_turns {
        assert_eq!(
            messages
                .count_documents(doc! {"conversation_id":&home,"turn_id":turn})
                .await
                .unwrap(),
            2
        );
    }
    if let Some(checkpoint) = db
        .collection::<Document>(crate::models::assistant_checkpoint::COLLECTION_NAME)
        .find_one(doc! {"_id":&home})
        .await
        .unwrap()
    {
        assert!(!format!("{checkpoint:?}").contains("Private asynchronous result"));
    }
    db.drop().await.unwrap();
}
