//! Round-three scheduler, genuine legacy web-resume and installed-index controls.
use super::*;

async fn due(f: &Fixture, requests: &[&ApprovalRequest]) {
    for request in requests {
        let version = proposals::v3::AssistantVersion::of(request).unwrap();
        f.state.db.collection::<Document>("approval_requests").update_one(doc! {"_id":&request.id},
            version.update(doc! {"$set":{"execution_proposal.v1.wake_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1)),
                "execution_proposal.v1.maintenance_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(1))}}))
            .await.unwrap();
    }
}

async fn reply_transport(
    f: &Fixture,
    row: &AssistantConversation,
) -> (wiremock::MockServer, String) {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(json!({"ok":true,
            "result":{"message_id":501,"chat":{"id":200}}})),
        )
        .mount(&server)
        .await;
    let source = row
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .as_ref()
        .unwrap()
        .channel_source
        .as_ref()
        .unwrap();
    let channel = f
        .state
        .db
        .collection::<Document>("nyxbot_channels")
        .find_one(doc! {"_id":&source.channel_id})
        .await
        .unwrap()
        .unwrap();
    let bot = channel.get_str("channel_bot_id").unwrap().to_owned();
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            bot.clone(),
            Arc::new(
                crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                    &server.uri(),
                ),
            ),
        );
    (server, bot)
}

// A's original [A] proof survives. A recovered wake and newly approved B really
// coalesce through admission in both orders; settlement releases their union once.
#[tokio::test]
async fn owner_permission_resume_recovered_and_fresh_decisions_preserve_independent_proofs() {
    for a_first in [true, false] {
        for crash_joined in [false, true] {
            let (f, row, _) =
                channel_turn_transport("owner_recovered_fresh_coalesced", "direct").await;
            set_task(&f, &row).await;
            let cells = sheet(&f, 200).await;
            let (relay, bot) = reply_transport(&f, &row).await;
            let model = model_with_initial_reviews(&f, true, true).await;
            run(&f, &row).await;
            let requests: Vec<_> = f
                .state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find(doc! {"status":"pending"})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(requests.len(), 2);
            let a = requests
                .iter()
                .find(|r| {
                    proposals::proposal(r)
                        .unwrap()
                        .review
                        .operation
                        .starts_with("PUT ")
                })
                .unwrap();
            let b = requests
                .iter()
                .find(|r| {
                    proposals::proposal(r)
                        .unwrap()
                        .review
                        .operation
                        .starts_with("GET ")
                })
                .unwrap();
            assert!(f.server.received_requests().await.unwrap().is_empty());
            proposals::decide(&f.state, &f.owner, &a.id, &decision(a, "persistent"))
                .await
                .unwrap();
            let first = event_turn(&f, &row).await;
            assert_eq!(first.active_turn.as_ref().unwrap().events.len(), 1);
            proposals::continuation::FAIL_AFTER_RECEIPT
                .scope(true, run(&f, &first))
                .await;
            let settled = engine::messages(&f.state.db, &f.owner, &row.id, 20, None)
                .await
                .unwrap();
            assert_eq!(settled.last().unwrap().status, "failed");
            assert_eq!(
                settled.last().unwrap().error_detail.as_ref().unwrap().stage,
                "permission_resume"
            );
            if !a_first && crash_joined {
                // A complete historical v2 proof is adopted under the new
                // fenced protocol; its real provider receipt is preserved.
                let version = proposals::v3::AssistantVersion::of(a).unwrap();
                f.state
                    .db
                    .collection::<Document>("approval_requests")
                    .update_one(
                        doc! {"_id":&a.id},
                        version.update(
                            doc! {"$set":{"execution_proposal.v1.owner_continuation.version":2},
                            "$unset":{"execution_proposal.v1.owner_continuation.handoff":""}},
                        ),
                    )
                    .await
                    .unwrap();
            }
            let completed_a = stored(&f, &a.id).await;
            let original_a = proposals::proposal(&completed_a)
                .unwrap()
                .owner_continuation
                .as_ref()
                .unwrap();
            assert_eq!(original_a.batch.as_ref().unwrap(), &vec![a.id.clone()]);
            assert!(original_a.released_turn_id.is_none());
            assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
            assert_eq!(model.calls.lock().unwrap().len(), 1);
            assert!(
                engine::get(&f.state.db, &f.owner, &row.id)
                    .await
                    .unwrap()
                    .active_turn
                    .is_none()
            );
            if a_first {
                due(&f, &[a]).await;
                proposals::sweep(&f.state).await.unwrap();
            }
            proposals::decide(&f.state, &f.owner, &b.id, &decision(b, "persistent"))
                .await
                .unwrap();
            if !a_first {
                due(&f, &[a]).await;
                proposals::sweep(&f.state).await.unwrap();
            }
            let expected = if a_first {
                [a.id.clone(), b.id.clone()]
            } else {
                [b.id.clone(), a.id.clone()]
            };
            let queued = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert_eq!(
                queued
                    .pending_events
                    .iter()
                    .filter_map(|event| event
                        .id
                        .strip_prefix("execution-decision:")
                        .map(str::to_owned))
                    .collect::<Vec<_>>(),
                expected
            );
            // Genuine independently admitted owner task, plus a stale source
            // snapshot. Admission must defer the former and prune the latter.
            let other_request = engine::TurnRequest {
                conversation_id: None,
                text: "Independent task must remain deferred".into(),
                acknowledgement_id: None,
                retry_of: None,
                attachment_ids: vec![],
                agent_id: None,
                model: None,
                access_mode: None,
            };
            let mut other_start: engine::TurnStart = (&other_request).into();
            other_start.execution_enrollment = true;
            let other =
                engine::begin_turn(&f.state.db, &f.owner, other_start, &f.state.encryption_keys)
                    .await
                    .unwrap();
            assert_ne!(other.id, row.id);
            let mut deferred = crate::services::assistant_team_service::event(
                "followup",
                "Independent task must remain deferred".into(),
                None,
            );
            deferred.execution_task = other.active_turn.as_ref().unwrap().execution_task.clone();
            let deferred_id = deferred.id.clone();
            let mut stale = crate::services::assistant_team_service::event(
                "followup",
                "Stale task must never enter continuation".into(),
                None,
            );
            stale.execution_task = first.active_turn.as_ref().unwrap().execution_task.clone();
            stale.execution_task.as_mut().unwrap().epoch += 1;
            let stale_id = stale.id.clone();
            engine::push_events(
                &f.state.db,
                &f.owner,
                &row.id,
                if crash_joined {
                    vec![stale]
                } else {
                    vec![deferred.clone(), stale]
                },
            )
            .await
            .unwrap()
            .unwrap();
            let joined = event_turn(&f, &row).await;
            assert_eq!(
                joined
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .events
                    .iter()
                    .map(|event| event.id.clone())
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|id| format!("execution-decision:{id}"))
                    .collect::<Vec<_>>()
            );
            let pending = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert_eq!(pending.pending_events.len(), usize::from(!crash_joined));
            if !crash_joined {
                assert_eq!(pending.pending_events[0].id, deferred_id);
            }
            assert!(
                !pending
                    .pending_events
                    .iter()
                    .any(|event| event.id == stale_id)
            );
            let joined_id = joined.active_turn.as_ref().unwrap().turn_id.clone();
            // The actual admission commits the independent originals and their
            // common handoff before any optional resume dependency or B's GET.
            let mut admitted_union = vec![a.id.clone(), b.id.clone()];
            admitted_union.sort();
            for request in [a, b] {
                let receipt = stored(&f, &request.id).await;
                let proof = proposals::proposal(&receipt)
                    .unwrap()
                    .owner_continuation
                    .as_ref()
                    .unwrap();
                assert!(!proof.blocked);
                assert!(proof.released_turn_id.is_none());
                assert_eq!(proof.batch.as_ref().unwrap(), &vec![request.id.clone()]);
                let handoff = proof.handoff.as_ref().unwrap();
                assert_eq!(handoff.request_ids, admitted_union);
                assert_eq!(handoff.turn_id, joined_id);
            }
            let final_turn = if crash_joined {
                proposals::continuation::FAIL_AFTER_RECEIPT
                    .scope(true, run(&f, &joined))
                    .await;
                let settled = engine::messages(&f.state.db, &f.owner, &row.id, 20, None)
                    .await
                    .unwrap();
                assert_eq!(settled.last().unwrap().status, "failed");
                assert_eq!(
                    settled.last().unwrap().error_detail.as_ref().unwrap().stage,
                    "permission_resume"
                );
                assert_eq!(model.calls.lock().unwrap().len(), 1);
                assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
                due(&f, &[a, b]).await;
                proposals::sweep(&f.state).await.unwrap();
                let missing = if a_first { &b.id } else { &a.id };
                // Simulate one lost recovery entry; its proven receipt is read,
                // never expanded into another provider operation.
                f.state.db.collection::<Document>("assistant_conversations").update_one(doc! {"_id":&row.id},
                    doc! {"$pull":{"pending_events":{"id":format!("execution-decision:{missing}")}}}).await.unwrap();
                engine::push_events(&f.state.db, &f.owner, &row.id, vec![deferred.clone()])
                    .await
                    .unwrap()
                    .unwrap();
                let recovery = event_turn(&f, &row).await;
                assert_eq!(recovery.active_turn.as_ref().unwrap().events.len(), 1);
                run(&f, &recovery).await;
                recovery
            } else {
                run(&f, &joined).await;
                joined
            };
            final_reply(&f, &row, 2, &model).await;
            assert_eq!(*cells.lock().unwrap(), Some(values()));
            assert_eq!(*model.reads.lock().unwrap(), [values()]);
            assert_eq!(model.calls.lock().unwrap().len(), 2);
            assert_eq!(
                f.server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .map(|r| r.method.as_str())
                    .collect::<Vec<_>>(),
                ["PUT", "GET"]
            );
            assert_eq!(count(&f, "approval_requests").await, 2);
            assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
            let mut union = vec![a.id.clone(), b.id.clone()];
            union.sort();
            for request in [a, b] {
                let receipt = stored(&f, &request.id).await;
                let p = proposals::proposal(&receipt).unwrap();
                let proof = p.owner_continuation.as_ref().unwrap();
                assert_eq!(proof.batch.as_ref().unwrap(), &vec![request.id.clone()]);
                assert!(!proof.blocked);
                assert_eq!(proof.handoff.as_ref().unwrap().request_ids, union);
                assert_eq!(
                    proof.released_turn_id.as_deref(),
                    Some(final_turn.active_turn.as_ref().unwrap().turn_id.as_str())
                );
                assert_eq!(p.delivered_turn_id, proof.released_turn_id);
                if request.id == a.id {
                    assert_eq!(proof.batch_turn_id, original_a.batch_turn_id);
                } else {
                    assert_eq!(proof.batch_turn_id.as_deref(), Some(joined_id.as_str()));
                }
            }
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    let replies: Vec<Value> = relay
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .filter_map(|r| r.body_json::<Value>().ok())
                        .filter(|v| v["text"] == ANSWER)
                        .collect();
                    if !replies.is_empty() {
                        assert_eq!(replies.len(), 1);
                        assert_eq!(replies[0]["chat_id"], "200");
                        assert_eq!(replies[0]["message_thread_id"], 77);
                        assert_eq!(replies[0]["reply_to_message_id"], 500);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            due(&f, &[a, b]).await;
            proposals::sweep(&f.state).await.unwrap();
            let remaining = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert!(
                !remaining
                    .pending_events
                    .iter()
                    .any(|event| event.id.starts_with("execution-decision:"))
            );
            assert_eq!(remaining.pending_events.len(), 1);
            assert_eq!(remaining.pending_events[0].id, deferred_id);
            assert_eq!(
                remaining.pending_events[0].execution_task,
                other.active_turn.as_ref().unwrap().execution_task
            );
            assert!(
                execute(&f, &final_turn)
                    .await
                    .unwrap()
                    .text
                    .contains("HTTP 200")
            );
            assert_eq!(model.calls.lock().unwrap().len(), 2);
            assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
            crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
                .lock()
                .unwrap()
                .remove(&bot);
            drop(model);
            f.cleanup().await;
        }
    }
}

// Actual first-party web approval owns the event wake and runner. The optional
// continuation capability alone is broken; legacy authorization still sends once.
#[tokio::test]
async fn owner_permission_resume_unmarked_web_approval_ignores_optional_readiness_failure() {
    let (f, row) = assistant("owner_legacy_web_optional_readiness").await;
    set_task(&f, &row).await;
    sheet(&f, 200).await;
    let model = model(&f, false).await;
    run(&f, &row).await;
    let request = pending(&f, None).await;
    let task = row
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .as_ref()
        .unwrap();
    f.state
        .db
        .collection::<Document>("assistant_messages")
        .update_one(
            doc! {"_id":&task.task.root_id},
            doc! {"$unset":{"execution_dispatch_evidence_version":"","execution_dispatches":""}},
        )
        .await
        .unwrap();
    readiness_dependency(&f, true).await;
    // Leave enough time to capture the genuinely admitted event turn for the
    // later runtime replay control; the provider response remains real HTTP.
    f.server.reset().await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .and(wiremock::matchers::path(RANGE))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"updatedCells":2}))
                .set_delay(std::time::Duration::from_secs(1)),
        )
        .mount(&f.server)
        .await;
    approve(&f, &request, "persistent").await;
    let admitted = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            if current.active_turn.as_ref().is_some_and(|turn| {
                turn.events
                    .iter()
                    .any(|event| event.id == format!("execution-decision:{}", request.id))
            }) {
                break current;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        admitted.active_turn.as_ref().unwrap().origin,
        TurnOrigin::Event
    );
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            let receipt = stored(&f, &request.id).await;
            if current.active_turn.is_none()
                && proposals::proposal(&receipt)
                    .unwrap()
                    .delivered_turn_id
                    .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let receipt = stored(&f, &request.id).await;
    let p = proposals::proposal(&receipt).unwrap();
    assert_eq!(p.state, "completed");
    assert!(p.owner_continuation.is_none());
    assert_eq!(receipt.activity_outcome.as_deref(), Some("completed"));
    let messages = engine::messages(&f.state.db, &f.owner, &row.id, 20, None)
        .await
        .unwrap();
    assert!(messages.last().unwrap().text.contains("HTTP 200"));
    assert!(messages.last().unwrap().error_code.is_none());
    assert!(
        execute(&f, &admitted)
            .await
            .unwrap()
            .text
            .contains("HTTP 200")
    );
    due(&f, &[&request]).await;
    proposals::sweep(&f.state).await.unwrap();
    assert!(
        engine::get(&f.state.db, &f.owner, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    assert_eq!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        ["PUT"]
    );
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    f.cleanup().await;
}

// Synthetic rows measure the real production predicate/installer, independently
// of the genuine runtime/provider journeys above. Keep competing old indexes.
#[tokio::test]
async fn owner_permission_resume_release_lookup_uses_installed_indexes_across_tenants() {
    use mongodb::{IndexModel, options::IndexOptions};
    let f = Fixture::new("owner_release_indexed_lookup").await;
    let collection = f.state.db.collection::<Document>("approval_requests");
    proposals::ensure_owner_continuation_indexes(&f.state.db)
        .await
        .unwrap();
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id":1,"status":1})
                .build(),
        )
        .await
        .unwrap();
    let versions = proposals::v3::AssistantVersion::ALL;
    for version in versions {
        let prefix = format!("execution_proposal.v{}", version.number());
        collection.create_index(IndexModel::builder().keys(doc! {"execution_proposal.version":1,
            format!("{prefix}.delivered_turn_id"):1,format!("{prefix}.maintenance_at"):1,"created_at":1}).build()).await.unwrap();
        collection
            .create_index(
                IndexModel::builder()
                    .keys(doc! {format!("{prefix}.gap_key"):1,"created_at":-1})
                    .options(
                        IndexOptions::builder()
                            .name(format!("competing_gap_v{}", version.number()))
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
    }
    let noise: Vec<_> = (0..3072).map(|i| {
        let version = versions[i % versions.len()].number() as i32;
        doc! {"_id":format!("noise-{i}"),"user_id":format!("owner-{i}"),"status":"approved","created_at":bson::DateTime::now(),
            "execution_proposal":{"version":version,format!("v{version}"):{"conversation_id":format!("conversation-{i}"),
                "gap_key":format!("gap-{i}"),"owner_continuation":{"version":1,"released_turn_id":format!("turn-{i}")}}}}
    }).collect();
    collection.insert_many(noise).await.unwrap();
    for version in versions {
        let n = version.number() as i32;
        collection.insert_one(doc! {"_id":format!("positive-v{n}"),"user_id":"foreign-owner","status":"approved",
            "created_at":bson::DateTime::now(),"execution_proposal":{"version":n,format!("v{n}"):{
                "conversation_id":"target-conversation","owner_continuation":{"version":if n==1 {1} else {3},"released_turn_id":format!("target-turn-v{n}")}}}})
            .await.unwrap();
    }
    let installed: Vec<_> = collection
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for version in versions {
        let name = format!("execution_owner_release_v{}", version.number());
        let index = installed
            .iter()
            .find(|index| index.options.as_ref().and_then(|o| o.name.as_ref()) == Some(&name))
            .unwrap();
        let prefix = format!("execution_proposal.v{}", version.number());
        assert_eq!(
            index.keys,
            doc! {format!("{prefix}.conversation_id"):1,format!("{prefix}.owner_continuation.released_turn_id"):1}
        );
        assert_eq!(
            index.options.as_ref().unwrap().partial_filter_expression,
            Some(doc! {"execution_proposal.version":version.number() as i32,
            format!("{prefix}.owner_continuation.released_turn_id"):{"$type":"string"}})
        );
    }
    let mut cases = vec![
        (
            "ordinary-no-match".to_owned(),
            "absent-conversation".to_owned(),
            "absent-turn".to_owned(),
            0,
        ),
        (
            "same-conversation-new-turn".to_owned(),
            "target-conversation".to_owned(),
            "absent-turn".to_owned(),
            0,
        ),
    ];
    for version in versions {
        cases.push((
            format!("historical-exact-v{}", version.number()),
            "target-conversation".into(),
            format!("target-turn-v{}", version.number()),
            1,
        ));
    }
    for (name, conversation, turn, expected) in cases {
        let predicate = proposals::effects::release_lookup(&conversation, &turn);
        let explain = f
            .state
            .db
            .run_command(
                doc! {"explain":{"find":"approval_requests","filter":predicate,
                "projection":{"_id":1},"limit":1},"verbosity":"executionStats"},
            )
            .await
            .unwrap();
        let value = serde_json::to_value(explain).unwrap();
        let stats = &value["executionStats"];
        assert_eq!(stats["nReturned"], expected);
        assert_eq!(stats["totalDocsExamined"], expected);
        assert_eq!(stats["totalKeysExamined"], expected);
        let winning = value["queryPlanner"]["winningPlan"].to_string();
        assert!(!winning.contains("COLLSCAN"));
        assert!(winning.contains("execution_owner_release_v"));
        println!(
            "OWNER_RELEASE_EXECUTION_STATS {name}: returned={} docs={} keys={}",
            stats["nReturned"], stats["totalDocsExamined"], stats["totalKeysExamined"]
        );
    }
    f.cleanup().await;
}
