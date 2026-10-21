//! Transactional acknowledgement evidence, admission, retry and transport controls.
use super::*;
use crate::{
    models::audit_outbox::{
        AcknowledgementApproverKind, AcknowledgementCardKind, AcknowledgementDecision, AuditDetail,
        AuditOwner, AuditPrincipal, PENDING_COLLECTION_NAME, PendingAuditEvent,
    },
    services::{
        assistant_authority_tests::{Fixture, connected, fixture, orchestrator_fixture},
        audit_runtime::{AuditRuntime, policy::AdmissionLimits},
    },
    test_utils::fail_command::{FailCommandController, Guard},
};
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

async fn action(f: &Fixture) -> AssistantAcknowledgement {
    request(
        &f.state.db,
        &f.chat,
        Request {
            kind: "action",
            service: None,
            tool: Some("w8_private_tool"),
            arguments: Some(&json!({"private_argument":"never audit"})),
            summary: "w8_private_summary",
            platform: false,
        },
    )
    .await
    .unwrap()
}

fn person(f: &Fixture) -> AuditPrincipal {
    crate::services::token_service::audit_principal(&crate::test_utils::test_auth_user(&f.owner))
        .unwrap()
}

async fn events(db: &Database, card: &str) -> Vec<PendingAuditEvent> {
    db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
        .find(doc! {"event.action":"assistant_acknowledgement_decided","event.target_id":card})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn stored_card(db: &Database, card: &str) -> bson::Document {
    db.collection::<bson::Document>(ACKS)
        .find_one(doc! {"_id":card})
        .await
        .unwrap()
        .unwrap()
}

async fn backlog(f: &mut Fixture, case: &str, reservations: u64) {
    let limits = AdmissionLimits::default();
    f.state.audit_runtime = Arc::new(AuditRuntime::new(limits.clone(), case != "local"));
    let state = crate::services::audit_state_service::load(&f.state.db)
        .await
        .unwrap();
    let count = match case {
        "below" => limits.standard.pending_count - reservations,
        "count" => limits.standard.pending_count,
        "bundle" => limits.standard.pending_count - 1,
        _ => 1,
    } as i64;
    let age = if case == "age" {
        limits.standard.pending_age_secs + 1
    } else {
        0
    };
    f.state.db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
        .update_one(doc! {"singleton":true},doc! {"$set":{
            "pending_count":count,"next_order":state.next_order.max(count),
            "oldest_pending_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(age as i64)),
        }}).await.unwrap();
    if case == "diverged" {
        f.state.db.collection::<bson::Document>(crate::models::audit_witness_delivery::COLLECTION_NAME)
            .insert_one(doc! {"_id":state.id,"divergence":{"seq":0_i64,"local_hash":"a","witnessed_hash":"b"}})
            .await.unwrap();
    }
}

#[tokio::test]
async fn w8_acknowledgement_allow_standard_and_deny_narrowing_are_atomic() {
    for case in ["local", "stale", "below", "count", "age", "diverged"] {
        let mut f = orchestrator_fixture(&format!("w8_ack_admission_{case}")).await;
        let card = action(&f).await;
        let before = stored_card(&f.state.db, &card.id).await;
        backlog(&mut f, case, 1).await;
        let result = decide(
            &f.state.db,
            &f.state.audit_runtime,
            &person(&f),
            &f.owner,
            &f.row.id,
            &card.id,
            true,
        )
        .await;
        if matches!(case, "local" | "stale" | "below") {
            assert_eq!(
                result
                    .unwrap_or_else(|error| panic!("{case}: {error:?}"))
                    .status,
                "allowed"
            );
        } else {
            if case == "diverged" {
                assert!(
                    matches!(result, Err(AppError::AuditWitnessDiverged)),
                    "{case}: {result:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(AppError::AuditAdmissionClosed { .. })),
                    "{case}: {result:?}"
                );
            }
            assert_eq!(stored_card(&f.state.db, &card.id).await, before, "{case}");
            assert!(events(&f.state.db, &card.id).await.is_empty(), "{case}");
            let denied = decide(
                &f.state.db,
                &f.state.audit_runtime,
                &person(&f),
                &f.owner,
                &f.row.id,
                &card.id,
                false,
            )
            .await
            .unwrap();
            assert_eq!(denied.status, "denied", "{case}");
        }
        let rows = events(&f.state.db, &card.id).await;
        assert_eq!(rows.len(), 1, "{case}");
        assert_eq!(rows[0].event.principal, person(&f));
        assert_eq!(
            rows[0].event.owner,
            AuditOwner::Person {
                id: f.owner.clone()
            }
        );
        assert_eq!(
            rows[0].event.detail,
            Some(AuditDetail::AcknowledgementDecision {
                decision: if matches!(case, "local" | "stale" | "below") {
                    AcknowledgementDecision::Allow
                } else {
                    AcknowledgementDecision::Deny
                },
                card_kind: AcknowledgementCardKind::Action,
                approver_kind: AcknowledgementApproverKind::Owner,
            })
        );
        let projection = serde_json::to_string(&rows[0].event).unwrap();
        for secret in [
            "w8_private_tool",
            "private_argument",
            "never audit",
            "w8_private_summary",
        ] {
            assert!(!projection.contains(secret), "{case}: {secret}");
        }
        crate::test_utils::teardown_test_database(&f.state.db).await;
    }
}

#[tokio::test]
async fn w8_acknowledgement_typed_insert_failure_rolls_back_every_business_write() {
    for kind in ["action", "service", "account", "skills"] {
        let mut f = fixture(&format!("w8_ack_rollback_{kind}")).await;
        let service = connected(
            &f.state.db,
            &f.owner,
            "w8-service",
            "https://example.invalid",
        )
        .await;
        let card = request(
            &f.state.db,
            &f.chat,
            Request {
                kind,
                service: (kind == "service").then_some((
                    service.as_str(),
                    "w8-service",
                    "Private service",
                )),
                tool: None,
                arguments: None,
                summary: "private refusal reason",
                platform: false,
            },
        )
        .await
        .unwrap();
        let work = Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<bson::Document>(ACKS)
            .update_one(
                doc! {"_id":&card.id},
                doc! {"$set":{"trigger_run_id":&work}},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(crate::services::trigger_schedule::WORK)
            .insert_one(doc! {"_id":&work,"at":bson::DateTime::now(),"fence":"held","deferrals":7})
            .await
            .unwrap();
        let snapshot = |db: Database, card: String, key: String, agent: String, work: String| async move {
            let mut result = Vec::new();
            for (collection, id) in [
                (ACKS, card),
                (KEYS, key),
                (crate::models::assistant_agent::COLLECTION_NAME, agent),
                (crate::services::trigger_schedule::WORK, work),
            ] {
                result.push(
                    db.collection::<bson::Document>(collection)
                        .find_one(doc! {"_id":id})
                        .await
                        .unwrap()
                        .unwrap(),
                );
            }
            result
        };
        let before = snapshot(
            f.state.db.clone(),
            card.id.clone(),
            f.chat.api_key_id.clone(),
            f.chat.agent_id.clone(),
            work.clone(),
        )
        .await;
        let fixture_db = f.state.db.clone();
        let controller = Arc::new(OnceLock::<FailCommandController>::new());
        let control = controller.clone();
        let injected = Arc::new(AtomicBool::new(false));
        let injection = injected.clone();
        let handler = EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command.get_str("insert") == Ok(PENDING_COLLECTION_NAME)
                && !injection.swap(true, Ordering::SeqCst)
            {
                control
                    .get()
                    .unwrap()
                    .arm(
                        doc! {"times":1},
                        doc! {"failCommands":["insert"],"errorCode":121},
                    )
                    .unwrap();
            }
        });
        let guard = Guard::acquire(&fixture_db, Some(handler)).await;
        assert!(controller.set(guard.controller()).is_ok());
        f.state.db = guard.database().clone();
        let result = decide(
            &f.state.db,
            &f.state.audit_runtime,
            &person(&f),
            &f.owner,
            &f.row.id,
            &card.id,
            true,
        )
        .await;
        assert!(
            matches!(result, Err(AppError::DatabaseError(_))),
            "{kind}: {result:?}"
        );
        assert!(
            injected.load(Ordering::SeqCst),
            "{kind}: typed enqueue was reached"
        );
        assert_eq!(
            snapshot(
                f.state.db.clone(),
                card.id.clone(),
                f.chat.api_key_id.clone(),
                f.chat.agent_id.clone(),
                work
            )
            .await,
            before,
            "{kind}"
        );
        assert!(events(&f.state.db, &card.id).await.is_empty(), "{kind}");
        drop(guard);
        crate::test_utils::teardown_test_database(&fixture_db).await;
    }
}

#[derive(Default)]
struct Trace {
    started: Vec<(i32, String, bson::Document)>,
    finished: Vec<(i32, bool, bool)>,
}

#[tokio::test]
async fn w8_acknowledgement_labeled_body_and_unknown_commit_keep_one_stable_event() {
    let mut f = orchestrator_fixture("w8_ack_retry").await;
    let card = action(&f).await;
    let fixture_db = f.state.db.clone();
    let controller = Arc::new(OnceLock::<FailCommandController>::new());
    let control = controller.clone();
    let trace = Arc::new(Mutex::new(Trace::default()));
    let capture = trace.clone();
    let transient = AtomicBool::new(false);
    let unknown = AtomicBool::new(false);
    let handler = EventHandler::callback(move |event| match event {
        CommandEvent::Started(event) => {
            let business_commit = event.command_name == "commitTransaction"
                && capture
                    .lock()
                    .unwrap()
                    .started
                    .iter()
                    .any(|(_, _, command)| {
                        command.get_str("insert") == Ok(PENDING_COLLECTION_NAME)
                            && command.get("lsid") == event.command.get("lsid")
                            && command.get("txnNumber") == event.command.get("txnNumber")
                    });
            if event.command.get_str("insert") == Ok(PENDING_COLLECTION_NAME)
                && !transient.swap(true, Ordering::SeqCst)
            {
                control.get().unwrap().arm(doc! {"times":1},doc! {"failCommands":["insert"],"errorCode":112,"errorLabels":["TransientTransactionError"]}).unwrap();
            } else if business_commit && !unknown.swap(true, Ordering::SeqCst) {
                control.get().unwrap().arm(doc! {"times":1},doc! {"failCommands":["commitTransaction"],"writeConcernError":{"code":64,"errmsg":"injected ack uncertainty"},"errorLabels":["UnknownTransactionCommitResult"]}).unwrap();
            }
            capture.lock().unwrap().started.push((
                event.request_id,
                event.command_name,
                event.command,
            ));
        }
        CommandEvent::Failed(event) => capture.lock().unwrap().finished.push((
            event.request_id,
            event.failure.contains_label("TransientTransactionError"),
            event
                .failure
                .contains_label("UnknownTransactionCommitResult"),
        )),
        CommandEvent::Succeeded(event) => {
            let labels = event.reply.get_array("errorLabels").ok();
            let has = |label: &str| {
                labels
                    .is_some_and(|labels| labels.iter().any(|value| value.as_str() == Some(label)))
            };
            capture.lock().unwrap().finished.push((
                event.request_id,
                has("TransientTransactionError"),
                has("UnknownTransactionCommitResult"),
            ));
        }
        _ => {}
    });
    let guard = Guard::acquire(&fixture_db, Some(handler)).await;
    assert!(controller.set(guard.controller()).is_ok());
    f.state.db = guard.database().clone();
    let decided = decide(
        &f.state.db,
        &f.state.audit_runtime,
        &person(&f),
        &f.owner,
        &f.row.id,
        &card.id,
        true,
    )
    .await
    .unwrap();
    assert_eq!(decided.status, "allowed");
    let rows = events(&f.state.db, &card.id).await;
    assert_eq!(rows.len(), 1);
    {
        let trace = trace.lock().unwrap();
        let inserts: Vec<_> = trace
            .started
            .iter()
            .filter(|(_, _, command)| command.get_str("insert") == Ok(PENDING_COLLECTION_NAME))
            .collect();
        assert_eq!(inserts.len(), 2, "one whole-body transient retry");
        let documents: Vec<_> = inserts
            .iter()
            .map(|(_, _, command)| {
                command.get_array("documents").unwrap()[0]
                    .as_document()
                    .unwrap()
            })
            .collect();
        assert_eq!(documents[0].get("_id"), documents[1].get("_id"));
        assert_eq!(
            documents[0]
                .get_document("event")
                .unwrap()
                .get("operation_id"),
            documents[1]
                .get_document("event")
                .unwrap()
                .get("operation_id")
        );
        assert_eq!(documents[1].get_str("_id").unwrap(), rows[0].id);
        assert!(
            trace
                .finished
                .iter()
                .any(|(id, transient, _)| *id == inserts[0].0 && *transient)
        );
        let last = &inserts[1].2;
        let commits: Vec<_> = trace
            .started
            .iter()
            .filter(|(_, name, command)| {
                name == "commitTransaction"
                    && command.get("lsid") == last.get("lsid")
                    && command.get("txnNumber") == last.get("txnNumber")
            })
            .collect();
        assert_eq!(
            commits.len(),
            2,
            "unknown result retries commit on same transaction"
        );
        for (index, commit) in commits.iter().enumerate() {
            assert!(
                trace
                    .finished
                    .iter()
                    .any(|(id, transient, unknown)| *id == commit.0
                        && !transient
                        && *unknown == (index == 0))
            );
        }
        let first_commit = trace
            .started
            .iter()
            .position(|(id, _, _)| *id == commits[0].0)
            .unwrap();
        assert!(
            !trace.started[first_commit..]
                .iter()
                .any(|(_, name, _)| matches!(
                    name.as_str(),
                    "insert" | "update" | "abortTransaction"
                )),
            "no body replay after unknown"
        );
    }
    drop(guard);
    crate::test_utils::teardown_test_database(&fixture_db).await;
}

#[tokio::test]
async fn w8_acknowledgement_person_and_key_stay_distinct_from_org_owner() {
    use crate::{
        models::{org_membership::OrgRole, user::UserType},
        test_utils::{test_membership, test_user},
    };
    let f = crate::services::org_agent_tests::Fixture::new("w8_ack_two_orgs").await;
    let (first_agent, first_thread) = f.create().await;
    let second_org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&second_org, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .insert_one(test_membership(
            &second_org,
            &f.member,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let (second_agent, second_thread) =
        crate::services::assistant_team_service::create_specialist_for(
            &f.state.db,
            &f.state.encryption_keys,
            &f.member,
            &second_org,
            f.request("other-team"),
        )
        .await
        .unwrap()
        .unwrap();
    for (actor, agent, thread) in [
        (&f.admin, &first_agent, &first_thread),
        (&f.member, &second_agent, &second_thread),
    ] {
        let chat = for_key(&f.state.db, actor, Some(&thread.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        for route in ["person", "key"] {
            let card = request(
                &f.state.db,
                &chat,
                Request {
                    kind: "action",
                    service: None,
                    tool: Some(route),
                    arguments: None,
                    summary: "private org summary",
                    platform: false,
                },
            )
            .await
            .unwrap();
            let wrong = AuditPrincipal::Person {
                id: f.outsider.clone(),
            };
            let refused = decide(
                &f.state.db,
                &f.state.audit_runtime,
                &wrong,
                actor,
                &thread.id,
                &card.id,
                true,
            )
            .await;
            assert!(
                matches!(refused, Err(AppError::Forbidden(_))),
                "{refused:?}"
            );
            assert!(events(&f.state.db, &card.id).await.is_empty());
            assert_eq!(
                stored_card(&f.state.db, &card.id)
                    .await
                    .get_str("status")
                    .unwrap(),
                "pending"
            );
            let principal = if route == "person" {
                AuditPrincipal::Person { id: actor.clone() }
            } else {
                AuditPrincipal::ApiKey {
                    id: thread.credential_api_key_id.clone(),
                    owner_id: actor.clone(),
                }
            };
            decide(
                &f.state.db,
                &f.state.audit_runtime,
                &principal,
                actor,
                &thread.id,
                &card.id,
                true,
            )
            .await
            .unwrap();
            let rows = events(&f.state.db, &card.id).await;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].event.principal, principal);
            assert_eq!(
                rows[0].event.owner,
                AuditOwner::Org {
                    id: agent.user_id.clone()
                }
            );
            assert_ne!(actor, &agent.user_id);
            assert_eq!(
                rows[0].event.detail,
                Some(AuditDetail::AcknowledgementDecision {
                    decision: AcknowledgementDecision::Allow,
                    card_kind: AcknowledgementCardKind::Action,
                    approver_kind: if route == "person" {
                        AcknowledgementApproverKind::Owner
                    } else {
                        AcknowledgementApproverKind::ApiKey
                    },
                })
            );
        }
    }
    assert_ne!(first_agent.user_id, second_agent.user_id);
    crate::test_utils::teardown_test_database(&f.state.db).await;
}

#[tokio::test]
async fn w8_acknowledgement_channel_rechecks_exact_human_sender_route_and_chat() {
    use crate::{
        models::{
            channel_thread::ThreadSenderKind,
            nyxbot_channel::{NyxbotChannel, NyxbotThread},
        },
        services::assistant_platform_reply_decision::{OwnerReplyAuthority, decide_owner_reply},
    };
    for case in [
        "valid",
        "removed_sender",
        "rebound",
        "wrong_chat",
        "wrong_bot",
        "revoked_key",
        "nonhuman",
        "ineligible",
        "unverified_sender",
    ] {
        let f = orchestrator_fixture(&format!("w8_ack_channel_{case}")).await;
        let card = action(&f).await;
        let sender = "private_channel_sender";
        let channel:NyxbotChannel=bson::from_document(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"channel_bot_id":Uuid::new_v4().to_string(),
            "platform":"telegram","bot_label":"private bot","transport":"direct","status":"active","route_api_key_id":&f.chat.api_key_id,
            "owner_sender_ids":[sender],"follow_binding_generation":0_i64,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        let chat:NyxbotThread=bson::from_document(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"channel_id":&channel.id,"partition":"private-partition","conversation_id":&f.row.id,
            "kind":"private","owner_seen":true,"owner_chat":true,"allow_posts":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        f.state
            .db
            .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_one(&channel)
            .await
            .unwrap();
        f.state
            .db
            .collection::<NyxbotThread>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
            .insert_one(&chat)
            .await
            .unwrap();
        f.state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).insert_one(doc! {"_id":&channel.channel_bot_id,"user_id":&f.owner,"platform":"telegram","platform_bot_id":"private_bot","is_active":true}).await.unwrap();
        let proof = OwnerReplyAuthority::verified_sender(
            &channel,
            &chat,
            if case == "nonhuman" {
                ThreadSenderKind::Bot
            } else {
                ThreadSenderKind::Human
            },
            case != "ineligible",
            if case == "unverified_sender" {
                "different_sender"
            } else {
                sender
            },
        );
        if matches!(case, "nonhuman" | "ineligible" | "unverified_sender") {
            assert!(proof.is_err(), "{case}");
        } else {
            let proof = proof.unwrap();
            match case {
                "removed_sender" => {
                    f.state
                        .db
                        .collection::<bson::Document>(
                            crate::models::nyxbot_channel::COLLECTION_NAME,
                        )
                        .update_one(
                            doc! {"_id":&channel.id},
                            doc! {"$set":{"owner_sender_ids":[]}},
                        )
                        .await
                        .unwrap();
                }
                "rebound" => {
                    f.state
                        .db
                        .collection::<bson::Document>(
                            crate::models::nyxbot_channel::COLLECTION_NAME,
                        )
                        .update_one(
                            doc! {"_id":&channel.id},
                            doc! {"$set":{"binding_id":"replacement"}},
                        )
                        .await
                        .unwrap();
                }
                "wrong_chat" => {
                    f.state
                        .db
                        .collection::<bson::Document>(
                            crate::models::nyxbot_channel::THREADS_COLLECTION_NAME,
                        )
                        .update_one(doc! {"_id":&chat.id}, doc! {"$set":{"channel_id":"other"}})
                        .await
                        .unwrap();
                }
                "wrong_bot" => {
                    f.state
                        .db
                        .collection::<bson::Document>(
                            crate::models::nyxbot_channel::COLLECTION_NAME,
                        )
                        .update_one(
                            doc! {"_id":&channel.id},
                            doc! {"$set":{"channel_bot_id":"replacement"}},
                        )
                        .await
                        .unwrap();
                }
                "revoked_key" => {
                    f.state
                        .db
                        .collection::<bson::Document>(KEYS)
                        .update_one(
                            doc! {"_id":&f.chat.api_key_id},
                            doc! {"$set":{"is_active":false}},
                        )
                        .await
                        .unwrap();
                }
                _ => {}
            }
            let result = decide_owner_reply(
                &f.state.db,
                &f.state.audit_runtime,
                &proof,
                &f.row.id,
                "yes",
            )
            .await;
            if case == "valid" {
                assert_eq!(result.unwrap().unwrap().status, "allowed");
                let rows = events(&f.state.db, &card.id).await;
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].event.principal, person(&f));
                assert_eq!(
                    rows[0].event.detail,
                    Some(AuditDetail::AcknowledgementDecision {
                        decision: AcknowledgementDecision::Allow,
                        card_kind: AcknowledgementCardKind::Action,
                        approver_kind: AcknowledgementApproverKind::OwnerChannel
                    })
                );
                assert!(
                    !serde_json::to_string(&rows[0].event)
                        .unwrap()
                        .contains(sender)
                );
            } else {
                assert!(result.is_err(), "{case}: {result:?}");
            }
        }
        if case != "valid" {
            assert_eq!(
                stored_card(&f.state.db, &card.id)
                    .await
                    .get_str("status")
                    .unwrap(),
                "pending",
                "{case}"
            );
            assert!(events(&f.state.db, &card.id).await.is_empty(), "{case}");
        }
        crate::test_utils::teardown_test_database(&f.state.db).await;
    }
}
