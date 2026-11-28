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
                            "$unset":{"execution_proposal.v1.owner_continuation.handoff":"",
                                "execution_proposal.v1.owner_continuation.admission_handoff":""}},
                        ),
                    )
                    .await
                    .unwrap();
                let raw = f
                    .state
                    .db
                    .collection::<Document>("approval_requests")
                    .find_one(doc! {"_id":&a.id})
                    .await
                    .unwrap()
                    .unwrap();
                let envelope = format!("v{}", version.number());
                let proof = raw
                    .get_document("execution_proposal")
                    .unwrap()
                    .get_document(envelope.as_str())
                    .unwrap()
                    .get_document("owner_continuation")
                    .unwrap();
                assert_eq!(proof.get_i32("version").unwrap(), 2);
                let mut fields: Vec<_> = proof.keys().map(String::as_str).collect();
                fields.sort();
                assert_eq!(
                    fields,
                    [
                        "batch",
                        "batch_turn_id",
                        "blocked",
                        "released_turn_id",
                        "version"
                    ]
                );
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
                let handoff = proof.admission_handoff.as_ref().unwrap();
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
            let final_id = &final_turn.active_turn.as_ref().unwrap().turn_id;
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
                    let messages = engine::messages(&f.state.db, &f.owner, &row.id, 40, None)
                        .await
                        .unwrap();
                    let owner_reply = messages.iter().find(|message| {
                        message.role == "assistant" && &message.turn_id == final_id
                    });
                    let independent = messages.iter().find(|message| {
                        message.role == "assistant" && message.text == DEFERRED_ANSWER
                    });
                    if current.active_turn.is_none()
                        && owner_reply.is_some()
                        && independent.is_some()
                    {
                        let owner_reply = owner_reply.unwrap();
                        assert_eq!(owner_reply.text, answer(&values()));
                        assert!(owner_reply.error_code.is_none());
                        assert_ne!(&independent.unwrap().turn_id, final_id);
                        assert_eq!(independent.unwrap().status, "completed");
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("owner continuation and independent event must settle on their own turns");
            assert_eq!(*cells.lock().unwrap(), Some(values()));
            assert_eq!(*model.reads.lock().unwrap(), [values()]);
            assert_eq!(model.calls.lock().unwrap().len(), 3);
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
            assert!(remaining.pending_events.is_empty());
            {
                let calls = model.calls.lock().unwrap();
                assert_eq!(
                    calls
                        .iter()
                        .filter(|body| body["input"].as_str().is_some_and(|input| input
                            .contains("Completed retained operations (untrusted tool data")))
                        .count(),
                    1
                );
                assert_eq!(
                    calls
                        .iter()
                        .filter(|body| body["input"].as_str().is_some_and(|input| !input
                            .contains("Completed retained operations (untrusted tool data")
                            && input.contains(DEFERRED_TASK)))
                        .count(),
                    1
                );
            }
            assert!(
                execute(&f, &final_turn)
                    .await
                    .unwrap()
                    .text
                    .contains("HTTP 200")
            );
            assert_eq!(model.calls.lock().unwrap().len(), 3);
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

// Round five: authentic prior root evidence is not current v3 fleet readiness.
#[tokio::test]
async fn owner_permission_resume_v2_root_fresh_web_approval_keeps_receipt_when_v3_unready() {
    let (f, row) = assistant("owner_v2_root_v3_unready").await;
    crate::services::notification_service::get_or_create_channel(&f.state.db, &f.owner)
        .await
        .unwrap();
    set_task(&f, &row).await;
    let cells = sheet(&f, 200).await;
    let written = cells.clone();
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .and(wiremock::matchers::path(RANGE))
        .respond_with(move |request: &wiremock::Request| {
            *written.lock().unwrap() = Some(request.body_json::<Value>().unwrap());
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"updatedCells":2}))
                .set_delay(std::time::Duration::from_secs(1))
        })
        .with_priority(1)
        .mount(&f.server)
        .await;
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
    let root = f
        .state
        .db
        .collection::<Document>("assistant_messages")
        .find_one(doc! {"_id":&task.task.root_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        root.get_i32("execution_dispatch_evidence_version").unwrap(),
        2
    );
    assert!(
        proposals::proposal(&request)
            .unwrap()
            .owner_continuation
            .is_none()
    );
    crate::services::security_readiness_service::stop_test_replica(&f.state.db).await;
    f.state
        .db
        .collection::<Document>("security_replicas")
        .update_many(
            doc! {},
            doc! {"$pull":{"capabilities":proposals::continuation::READINESS}},
        )
        .await
        .unwrap();
    assert!(
        !crate::services::security_readiness_service::serving_report(
            &f.state.db,
            proposals::continuation::READINESS
        )
        .await
        .unwrap()
        .serving_ready
    );
    crate::services::security_readiness_service::require_serving(&f.state.db, proposals::READINESS)
        .await
        .unwrap();
    approve(&f, &request, "persistent").await;
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            let saved = stored(&f, &request.id).await;
            if current.active_turn.is_some()
                && let Some(intent) = &proposals::proposal(&saved).unwrap().owner_continuation
            {
                assert_eq!(intent.version, 0);
                assert!(intent.handoff.is_none());
                assert!(intent.released_turn_id.is_none());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("observe durable intent without a premature positive v3 marker");

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            let saved = stored(&f, &request.id).await;
            let p = proposals::proposal(&saved).unwrap();
            if current.active_turn.is_none() && p.delivered_turn_id.is_some() {
                assert_eq!(p.state, "completed");
                assert_eq!(saved.activity_outcome.as_deref(), Some("completed"));
                let intent = p.owner_continuation.as_ref().unwrap();
                assert_eq!(intent.version, 0);
                assert!(intent.blocked);
                assert!(intent.handoff.is_none());
                assert!(intent.admission_handoff.is_some());
                assert!(intent.released_turn_id.is_none());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let messages = engine::messages(&f.state.db, &f.owner, &row.id, 20, None)
        .await
        .unwrap();
    assert_eq!(messages.last().unwrap().status, "completed");
    assert!(messages.last().unwrap().text.contains("HTTP 200"));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert_eq!(*cells.lock().unwrap(), Some(values()));
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    due(&f, &[&request]).await;
    proposals::sweep(&f.state).await.unwrap();
    assert!(
        engine::get(&f.state.db, &f.owner, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    drop(model);
    f.cleanup().await;
}

// Rotation is real and precedes the next event claim. Invalid namespace entries
// are consumed without rendering proposal data; the ordinary event still runs.
#[tokio::test]
async fn owner_permission_resume_rotated_key_drops_stale_decision_and_settles_unrelated_event() {
    let (mut f, row) = assistant("owner_rotated_decision_queue").await;
    set_task(&f, &row).await;
    sheet(&f, 200).await;
    let model = model(&f, false).await;
    run(&f, &row).await;
    let request = pending(&f, None).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "persistent"),
    )
    .await
    .unwrap();
    let queued = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
    let original = queued
        .pending_events
        .iter()
        .find(|event| event.id == format!("execution-decision:{}", request.id))
        .unwrap()
        .clone();
    // Independent missing-reference and wrong-kind controls use the exact
    // server-only namespace. The live, correctly typed event remains eligible.
    let mut wrong_kind = original.clone();
    wrong_kind.kind = "followup".into();
    let mut missing = original.clone();
    missing.id = format!("execution-decision:{}", Uuid::new_v4());
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let filtered = proposals::filter_decisions(
        &f.state.db,
        &mut session,
        &queued,
        vec![wrong_kind, missing.clone(), original.clone()],
    )
    .await
    .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, original.id);
    session.abort_transaction().await.unwrap();
    let predecessor = f.key.clone();
    let rotated = crate::services::key_service::rotate_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &predecessor,
    )
    .await
    .unwrap();
    let credential = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(credential.api_key_id, rotated.id);
    assert_ne!(credential.api_key_id, predecessor);
    f.key = credential.api_key_id.clone();
    f.raw = credential.raw_key.to_string();
    engine::push_events(&f.state.db, &f.owner, &row.id, vec![missing])
        .await
        .unwrap()
        .unwrap();
    let event_request = engine::TurnRequest {
        conversation_id: Some(row.id.clone()),
        text: String::new(),
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&event_request).into();
    start.origin = TurnOrigin::Event;
    let drained = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys).await;
    assert!(matches!(drained, Err(AppError::Conflict(_))));
    let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
    assert!(current.active_turn.is_none());
    assert!(current.pending_events.is_empty());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    let ordinary =
        crate::services::assistant_team_service::event("followup", DEFERRED_TASK.into(), None);
    let ordinary_id = ordinary.id.clone();
    engine::push_events(&f.state.db, &f.owner, &row.id, vec![ordinary])
        .await
        .unwrap()
        .unwrap();
    let resumed = event_turn(&f, &row).await;
    let turn = resumed.active_turn.as_ref().unwrap();
    assert_eq!(turn.events.len(), 1);
    assert_eq!(turn.events[0].id, ordinary_id);
    let input = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.role == "user" && message.turn_id == turn.turn_id)
        .unwrap()
        .text;
    assert!(input.contains(DEFERRED_TASK));
    run_input(&f, &resumed, &input).await;
    let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
        .await
        .unwrap();
    let reply = messages
        .iter()
        .find(|message| message.role == "assistant" && message.turn_id == turn.turn_id)
        .unwrap();
    assert_eq!(reply.status, "completed");
    assert_eq!(reply.text, DEFERRED_ANSWER);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(model.calls.lock().unwrap().len(), 2);
    let saved = stored(&f, &request.id).await;
    assert!(
        proposals::proposal(&saved)
            .unwrap()
            .owner_continuation
            .as_ref()
            .is_none_or(|proof| proof.released_turn_id.is_none())
    );
    assert!(
        engine::get(&f.state.db, &f.owner, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    drop(model);
    f.cleanup().await;
}

type WorkerDocuments = (Document, Document);

async fn worker_documents(db: &mongodb::Database, request: &ApprovalRequest) -> WorkerDocuments {
    let proposal = db
        .collection::<Document>("approval_requests")
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    let grant = db
        .collection::<Document>("approval_grants")
        .find_one(doc! {"_id":&proposals::proposal(request).unwrap().grant_id})
        .await
        .unwrap()
        .unwrap();
    (proposal, grant)
}

async fn set_worker(db: &mongodb::Database, row: &AssistantConversation, change: &str) {
    let turn = row.active_turn.as_ref().unwrap();
    let mut events = turn.events.clone();
    if change == "events" {
        events[0].id = format!("execution-decision:{}", Uuid::new_v4());
    }
    let attempt = if turn.execution_attempt.is_none() && change != "current" {
        bson::Bson::Null
    } else {
        bson::Bson::Document(
            doc! {"epoch":if matches!(change,"epoch"|"current") {2_i64} else {1_i64},
            "claim_id":if matches!(change,"claim"|"current") {"current-claim"} else {"prior-claim"}},
        )
    };
    db.collection::<Document>("assistant_conversations").update_one(doc! {"_id":&row.id}, vec![doc! {"$set":{
        "active_turn.worker_instance":{"id":if matches!(change,"worker"|"current") {"current-worker"} else {"prior-worker"}},
        "active_turn.execution_attempt":attempt,
        "active_turn.stop_requested":change=="stop",
        "active_turn.lease_expires_at":{"$add":["$$NOW",if change=="lease" {-60_000_i64} else {120_000_i64}]},
        "active_turn.events":{"$literal":bson::to_bson(&events).unwrap()},
    }}]).await.unwrap();
}

struct TakeOverBeforeDispatch {
    db: mongodb::Database,
    inner: Arc<dyn NyxAgentEgress>,
    captured: Arc<Mutex<Option<WorkerDocuments>>>,
}

#[async_trait::async_trait]
impl NyxAgentEgress for TakeOverBeforeDispatch {
    async fn resume_execution(
        &self,
        row: &AssistantConversation,
        resume: proposals::Resume,
    ) -> AppResult<axum::response::Response> {
        // The old worker already owns a preparation claim, but no provider or
        // grant-use dispatch. The real egress must reject its captured worker.
        let request = self
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {"_id":&resume.request_id})
            .await?
            .ok_or(AppError::AssistantTurnRequired)?;
        let before = worker_documents(&self.db, &request).await;
        *self.captured.lock().unwrap() = Some(before);
        set_worker(&self.db, row, "current").await;
        self.inner.resume_execution(row, resume).await
    }
    async fn request(
        &self,
        authority: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        self.inner.request(authority, credential, request).await
    }
}

// Exact worker/event loss is never optional readiness. Cover both the initial
// batch transaction and the later preparation/dispatch gaps with real egress.
#[tokio::test]
async fn owner_permission_resume_stale_worker_preserves_proposal_and_grant_for_current_worker() {
    use crate::models::assistant_conversation::{EngineWorkerInstance, ExecutionAttempt};
    for phase in [
        "marked",
        "legacy_worker",
        "legacy",
        "readiness_off",
        "before_claim",
        "before_dispatch",
    ] {
        let (f, row) = assistant("owner_resume_worker_takeover").await;
        set_task(&f, &row).await;
        sheet(&f, 200).await;
        let model = model(&f, false).await;
        run(&f, &row).await;
        let request = pending(&f, None).await;
        if phase == "legacy" {
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
                    doc! {"$unset":{"execution_dispatch_evidence_version":""}},
                )
                .await
                .unwrap();
        }
        if phase == "readiness_off" {
            readiness_dependency(&f, true).await;
        }
        let approved = proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation"),
        )
        .await
        .unwrap();
        let mut prior = event_turn(&f, &row).await;
        let turn = prior.active_turn.as_mut().unwrap();
        turn.worker_instance = Some(EngineWorkerInstance {
            id: "prior-worker".into(),
            hostname: None,
        });
        turn.execution_attempt = if phase == "legacy_worker" {
            None
        } else {
            Some(ExecutionAttempt {
                epoch: 1,
                claim_id: "prior-claim".into(),
                foreground_delivery: None,
            })
        };
        let changes: &[&'static str] = if phase == "before_dispatch" {
            &["current"]
        } else if phase == "legacy_worker" {
            &["worker"]
        } else {
            &["worker", "epoch", "claim", "stop", "lease", "events"]
        };
        for &change in changes {
            set_worker(&f.state.db, &prior, "prior").await;
            let captured = Arc::new(Mutex::new(None));
            let initial = worker_documents(&f.state.db, &approved).await;
            let result = if phase == "before_claim" {
                let captured = captured.clone();
                let stale = prior.clone();
                let hook: proposals::ModelApprovalHook = Arc::new(move |db, request| {
                    let captured = captured.clone();
                    let stale = stale.clone();
                    Box::pin(async move {
                        let before = worker_documents(&db, &request).await;
                        *captured.lock().unwrap() = Some(before);
                        set_worker(&db, &stale, change).await;
                    })
                });
                proposals::continuation::BEFORE_EFFECT_CLAIM
                    .scope(hook, execute(&f, &prior))
                    .await
            } else if phase == "before_dispatch" {
                let egress = TakeOverBeforeDispatch {
                    db: f.state.db.clone(),
                    captured: captured.clone(),
                    inner: crate::handlers::assistant_nyxagent::http_egress(
                        f.state.clone(),
                        test_utils::test_auth_user(&f.owner),
                    ),
                };
                let mut credential = load_credential(&f, &prior).await;
                execute_using_egress(&f, &prior, &mut credential, &egress).await
            } else {
                set_worker(&f.state.db, &prior, change).await;
                execute(&f, &prior).await
            };
            let error = result.unwrap_err();
            assert_eq!(
                (error.code, error.stage),
                ("cancelled", "permission_resume"),
                "{phase}/{change}"
            );
            let expected = captured.lock().unwrap().clone().unwrap_or(initial);
            let after = worker_documents(&f.state.db, &approved).await;
            assert!(
                after == expected,
                "stale worker changed proposal or grant: {phase}/{change}"
            );
            assert!(f.server.received_requests().await.unwrap().is_empty());
            assert_eq!(model.calls.lock().unwrap().len(), 1);
        }
        set_worker(&f.state.db, &prior, "current").await;
        if phase == "before_dispatch" {
            // The old legitimate preparation claim has no provider dispatch.
            // Advance its recovery lease, then let the current worker reclaim it.
            let version = proposals::v3::AssistantVersion::of(&approved).unwrap();
            f.state
                .db
                .collection::<Document>("approval_requests")
                .update_one(
                    doc! {"_id":&approved.id},
                    version.update(doc! {"$set":{"execution_proposal.v1.claimed_at":
                    bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(3))}}),
                )
                .await
                .unwrap();
        }
        let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
        let result = execute(&f, &current).await.unwrap();
        if matches!(phase, "legacy" | "readiness_off") {
            assert!(result.text.contains("HTTP 200"));
        } else {
            assert_eq!(result.text, "Remaining task continued.");
        }
        engine::finish_turn(
            &f.state.db,
            &current,
            &f.key,
            &Uuid::new_v4().to_string(),
            &result,
        )
        .await
        .unwrap();
        let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
            .await
            .unwrap();
        let replies: Vec<_> = messages
            .iter()
            .filter(|message| {
                message.role == "assistant"
                    && message.turn_id == current.active_turn.as_ref().unwrap().turn_id
            })
            .collect();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].status, "completed");
        assert_eq!(replies[0].text, result.text);
        let receipt = stored(&f, &approved.id).await;
        let p = proposals::proposal(&receipt).unwrap();
        assert_eq!(p.state, "completed");
        assert_eq!(
            p.delivered_turn_id.as_deref(),
            Some(current.active_turn.as_ref().unwrap().turn_id.as_str())
        );
        let (_, grant) = worker_documents(&f.state.db, &approved).await;
        assert!(grant.get_bool("revoked").unwrap());
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            model.calls.lock().unwrap().len(),
            if matches!(phase, "legacy" | "readiness_off") {
                1
            } else {
                2
            }
        );
        assert!(
            execute(&f, &current)
                .await
                .unwrap()
                .text
                .contains("HTTP 200")
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        drop(model);
        f.cleanup().await;
    }
}

// A has a durable release but no successful settlement. A's recovered notice
// cannot absorb or poison the new independently approved readback's proof.
#[tokio::test]
async fn owner_permission_resume_released_receipt_preserves_fresh_success_in_both_queue_orders() {
    for released_first in [true, false] {
        let (f, row, _) = channel_turn_transport("owner_released_and_fresh", "direct").await;
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
            .find(|request| {
                proposals::proposal(request)
                    .unwrap()
                    .review
                    .operation
                    .starts_with("PUT ")
            })
            .unwrap();
        let b = requests
            .iter()
            .find(|request| {
                proposals::proposal(request)
                    .unwrap()
                    .review
                    .operation
                    .starts_with("GET ")
            })
            .unwrap();
        proposals::decide(&f.state, &f.owner, &a.id, &decision(a, "persistent"))
            .await
            .unwrap();
        let first = event_turn(&f, &row).await;
        let first_id = first.active_turn.as_ref().unwrap().turn_id.clone();
        proposals::continuation::FAIL_AFTER_RELEASE
            .scope(true, run(&f, &first))
            .await;
        let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
            .await
            .unwrap();
        let failed = messages
            .iter()
            .find(|message| message.role == "assistant" && message.turn_id == first_id)
            .unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(
            failed.error_detail.as_ref().unwrap().stage,
            "permission_release"
        );
        let released = stored(&f, &a.id).await;
        let a_proof = proposals::proposal(&released)
            .unwrap()
            .owner_continuation
            .as_ref()
            .unwrap();
        assert_eq!(a_proof.released_turn_id.as_deref(), Some(first_id.as_str()));
        assert!(
            proposals::proposal(&released)
                .unwrap()
                .delivered_turn_id
                .is_none()
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(model.calls.lock().unwrap().len(), 1);
        if released_first {
            due(&f, &[a]).await;
            proposals::sweep(&f.state).await.unwrap();
        }
        proposals::decide(&f.state, &f.owner, &b.id, &decision(b, "persistent"))
            .await
            .unwrap();
        if !released_first {
            due(&f, &[a]).await;
            proposals::sweep(&f.state).await.unwrap();
        }
        let resumed = event_turn(&f, &row).await;
        let turn = resumed.active_turn.as_ref().unwrap();
        let expected = if released_first {
            [a.id.clone(), b.id.clone()]
        } else {
            [b.id.clone(), a.id.clone()]
        };
        assert_eq!(
            turn.events
                .iter()
                .map(|event| event.id.clone())
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|id| format!("execution-decision:{id}"))
                .collect::<Vec<_>>()
        );
        let fresh = stored(&f, &b.id).await;
        let intent = proposals::proposal(&fresh)
            .unwrap()
            .owner_continuation
            .as_ref()
            .unwrap();
        assert_eq!(intent.version, 0);
        assert!(!intent.blocked);
        assert_eq!(intent.batch.as_ref().unwrap(), &vec![b.id.clone()]);
        assert_eq!(
            intent.admission_handoff.as_ref().unwrap().request_ids,
            vec![b.id.clone()]
        );
        assert!(intent.handoff.is_none());
        run(&f, &resumed).await;
        final_reply(&f, &row, 2, &model).await;
        assert_eq!(*cells.lock().unwrap(), Some(values()));
        assert_eq!(*model.reads.lock().unwrap(), [values()]);
        let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
            .await
            .unwrap();
        let reply = messages
            .iter()
            .find(|message| message.role == "assistant" && message.turn_id == turn.turn_id)
            .unwrap();
        assert_eq!(reply.status, "completed");
        assert_eq!(reply.text, answer(&values()));
        for (request, release_id) in [(a, first_id.as_str()), (b, turn.turn_id.as_str())] {
            let receipt = stored(&f, &request.id).await;
            let p = proposals::proposal(&receipt).unwrap();
            let proof = p.owner_continuation.as_ref().unwrap();
            assert!(!proof.blocked);
            assert_eq!(proof.batch.as_ref().unwrap(), &vec![request.id.clone()]);
            assert_eq!(proof.released_turn_id.as_deref(), Some(release_id));
            assert_eq!(p.delivered_turn_id.as_deref(), Some(turn.turn_id.as_str()));
            assert_eq!(
                proof.handoff.as_ref().unwrap().request_ids,
                vec![request.id.clone()]
            );
        }
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let replies: Vec<Value> = relay
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .filter_map(|request| request.body_json::<Value>().ok())
                    .filter(|body| body["text"] == ANSWER)
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
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .map(|request| request.method.as_str())
                .collect::<Vec<_>>(),
            ["PUT", "GET"]
        );
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        assert_eq!(count(&f, "approval_requests").await, 2);
        assert!(
            execute(&f, &resumed)
                .await
                .unwrap()
                .text
                .contains("HTTP 200")
        );
        due(&f, &[a, b]).await;
        proposals::sweep(&f.state).await.unwrap();
        assert!(
            engine::get(&f.state.db, &f.owner, &row.id)
                .await
                .unwrap()
                .pending_events
                .is_empty()
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&bot);
        drop(model);
        f.cleanup().await;
    }
}
