use super::*;
use crate::{
    models::assistant_conversation::{ActiveTurn, AssistantConversation},
    services::transaction_service,
};

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
        for _ in 0..16 {
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
        let completes = matches!(evidence, "same" | "child-same");
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
            Box::pin(erase_turn(db, tx, scope, 6))
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
    for location in ["pending", "active-direct", "active-events"] {
        let f = fixture().await;
        let db = &f.state.db;
        let proof = scopes::reference(&f.bot).unwrap();
        let mut parent = conversation(&f.bot.user_id, false, &proof);
        let context = doc! {"meta_grant":bson::to_bson(&proof).unwrap()};
        let event = doc! {"channel_contexts":[&context],"text":"retained inbox input"};
        match location {
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
