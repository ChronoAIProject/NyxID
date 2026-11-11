use super::*;
use crate::services::{
    assistant_acknowledgement_service as acks,
    assistant_authority_tests::{Fixture, orchestrator_fixture},
    assistant_nyxagent as engine,
    machine_activity_service::ACTIVITY_ID,
};
use serde_json::json;

async fn start(f: &Fixture, chat: &ChatAuthority) -> String {
    engine::activity_started_for_turn(
        &f.state.db,
        &f.owner,
        &chat.conversation_id,
        chat.turn_id.as_deref().unwrap(),
        "nyxid__create_schedule",
    )
    .await
    .unwrap()
    .unwrap()
}
async fn activities(f: &Fixture) -> Vec<TurnActivity> {
    engine::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap()
        .active_turn
        .unwrap()
        .activities
}
async fn refresh_for(f: &Fixture, a: &mut [TurnActivity]) {
    refresh(&f.state.db, &f.owner, &f.row.id, a.iter_mut().collect())
        .await
        .unwrap();
}
async fn card(
    f: &Fixture,
    activity: &str,
) -> crate::models::assistant_acknowledgement::AssistantAcknowledgement {
    ACTIVITY_ID
        .scope(
            Some(activity.into()),
            acks::request(
                &f.state.db,
                &f.chat,
                acks::Request {
                    kind: "action",
                    service: None,
                    tool: Some("nyxid__create_schedule"),
                    arguments: Some(&json!({"id":"exact"})),
                    summary: "Confirm scheduled automation",
                    platform: false,
                },
            ),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn gated_settlement_links_exact_successful_or_failed_continuation() {
    for ok in [true, false] {
        let f = Box::pin(orchestrator_fixture("decision_continuation")).await;
        let id = start(&f, &f.chat).await;
        let ack = card(&f, &id).await;
        engine::activity_finished_for_turn(
            &f.state.db,
            &f.owner,
            &f.row.id,
            f.chat.turn_id.as_deref().unwrap(),
            &id,
            false,
            None,
        )
        .await
        .unwrap();
        let mut original = activities(&f).await;
        assert_eq!(original[0].status, "error", "legacy status is retained");
        assert_eq!(original[0].decision.as_ref().unwrap().id, ack.id);
        refresh_for(&f, &mut original).await;
        assert_eq!(original[0].decision.as_ref().unwrap().state, "waiting");
        engine::finish_turn(
            &f.state.db,
            &f.row,
            &f.row.credential_api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "Waiting".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        )
        .await
        .unwrap();
        let messages: Vec<crate::models::assistant_message::AssistantMessage> = f
            .state
            .db
            .collection("assistant_messages")
            .find(doc! {"conversation_id":&f.row.id,"role":"assistant"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(
            messages[0].activities[0].decision.as_ref().unwrap().id,
            ack.id
        );
        acks::decide(&f.state.db, &f.owner, &f.row.id, &ack.id, true)
            .await
            .unwrap();
        refresh_for(&f, &mut original).await;
        assert_eq!(
            original[0].decision.as_ref().unwrap().state,
            "allowed",
            "Allow is not execution"
        );
        let next = engine::begin_turn(
            &f.state.db,
            &f.owner,
            &engine::TurnRequest {
                conversation_id: Some(f.row.id.clone()),
                acknowledgement_id: Some(ack.id.clone()),
                retry_of: None,
                attachment_ids: vec![],
                agent_id: None,
                text: "Continue".into(),
                model: None,
                access_mode: None,
            },
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
        let chat = acks::for_key(&f.state.db, &f.owner, Some(&next.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        let unrelated = start(&f, &chat).await;
        engine::activity_finished_for_turn(
            &f.state.db,
            &f.owner,
            &f.row.id,
            chat.turn_id.as_deref().unwrap(),
            &unrelated,
            true,
            None,
        )
        .await
        .unwrap();
        refresh_for(&f, &mut original).await;
        assert_eq!(
            original[0].decision.as_ref().unwrap().state,
            "allowed",
            "unrelated success proves nothing"
        );
        let resumed = start(&f, &chat).await;
        assert!(
            ACTIVITY_ID
                .scope(
                    Some(resumed.clone()),
                    acks::consume_action(
                        &f.state.db,
                        &chat,
                        &ack.id,
                        "nyxid__create_schedule",
                        &json!({"id":"exact"})
                    )
                )
                .await
                .unwrap()
        );
        // While the exact resumed occurrence is still running, a result for
        // the original turn/activity must not claim its completion slot.
        let mut stale = original[0].decision.clone().unwrap();
        stale.executing = true;
        finished(
            &f.state.db,
            &f.row.id,
            f.chat.turn_id.as_deref().unwrap(),
            &id,
            Some(&stale),
            if ok {
                Completion::Failed
            } else {
                Completion::Succeeded
            },
            None,
        )
        .await
        .unwrap();
        engine::activity_finished_for_turn(
            &f.state.db,
            &f.owner,
            &f.row.id,
            chat.turn_id.as_deref().unwrap(),
            &resumed,
            ok,
            None,
        )
        .await
        .unwrap();
        refresh_for(&f, &mut original).await;
        assert_eq!(
            original[0].decision.as_ref().unwrap().state,
            if ok { "completed" } else { "failed" }
        );
        // Nor can it overwrite the completed receipt afterward.
        finished(
            &f.state.db,
            &f.row.id,
            f.chat.turn_id.as_deref().unwrap(),
            &id,
            Some(&stale),
            if ok {
                Completion::Failed
            } else {
                Completion::Succeeded
            },
            None,
        )
        .await
        .unwrap();
        refresh_for(&f, &mut original).await;
        assert_eq!(
            original[0].decision.as_ref().unwrap().state,
            if ok { "completed" } else { "failed" }
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn decision_denial_expiry_legacy_and_missing_evidence_are_not_success() {
    let f = Box::pin(orchestrator_fixture("decision_terminals")).await;
    let id = start(&f, &f.chat).await;
    let ack = card(&f, &id).await;
    let mut activity = activities(&f).await;
    acks::decide(&f.state.db, &f.owner, &f.row.id, &ack.id, false)
        .await
        .unwrap();
    refresh_for(&f, &mut activity).await;
    assert_eq!(activity[0].decision.as_ref().unwrap().state, "denied");
    for (status, expected, expired) in [
        ("denied", "denied", false),
        ("pending", "expired", true),
        ("allowed", "expired", true),
        ("used", "allowed", true),
    ] {
        f.state.db.collection::<Document>(ACKS).update_one(doc!{"_id":&ack.id},doc!{"$set":{"status":status,"expires_at":bson::DateTime::from_millis(if expired {0}else{i64::MAX})}}).await.unwrap();
        refresh_for(&f, &mut activity).await;
        assert_eq!(activity[0].decision.as_ref().unwrap().state, expected);
    }
    f.state
        .db
        .collection::<Document>(ACKS)
        .delete_one(doc! {"_id":&ack.id})
        .await
        .unwrap();
    refresh_for(&f, &mut activity).await;
    assert_eq!(activity[0].decision.as_ref().unwrap().state, "unavailable");
    activity[0].decision = None;
    refresh_for(&f, &mut activity).await;
    assert!(activity[0].decision.is_none());
    let encoded = bson::to_document(&activity[0]).unwrap();
    assert!(!encoded.contains_key("decision"));
    f.state.db.drop().await.unwrap();
}

#[test]
fn execution_permission_v1_to_v4_need_success_evidence_not_just_receipt_completion() {
    for version in 1..=4 {
        for (state, expected) in [
            ("pending", "waiting"),
            ("queued", "allowed"),
            ("waiting", "waiting"),
            ("denied", "denied"),
            ("expired", "expired"),
            ("cancelled", "cancelled"),
            ("unknown", "unavailable"),
            ("completed", "unavailable"),
        ] {
            let mut row = doc! {"status":"approved","expires_at":bson::DateTime::from_millis(i64::MAX),"execution_proposal":{"version":version,format!("v{version}"):{"state":state}}};
            assert_eq!(approval_state(&row), expected);
            row.insert("activity_outcome", "completed");
            if state == "completed" {
                assert_eq!(approval_state(&row), "completed");
                row.insert("activity_outcome", "failed");
                assert_eq!(approval_state(&row), "failed");
            }
        }
    }
}

#[tokio::test]
async fn raw_proxy_gate_lazily_records_one_activity_and_exact_connected_result() {
    let f = Box::pin(orchestrator_fixture("decision_proxy")).await;
    let id = uuid::Uuid::new_v4().to_string();
    let scope = ProxyActivity::new(id.clone());
    let used = scope.used.clone();
    PROXY_ACTIVITY.scope(scope,async {
        assert!(activities(&f).await.is_empty());
        f.state.db.collection::<Document>(APPROVALS).insert_one(doc!{"_id":"proxy-card","user_id":&f.owner,"status":"pending","expires_at":bson::DateTime::from_millis(i64::MAX)}).await.unwrap();
        approval(&f.state.db,&f.chat,"proxy-card",false).await.unwrap();
        approval(&f.state.db,&f.chat,"proxy-card",false).await.unwrap();
        assert_eq!(activities(&f).await.len(),1);
        f.state.db.collection::<Document>(APPROVALS).update_one(doc!{"_id":"proxy-card"},doc!{"$set":{"status":"approved"}}).await.unwrap();
        approval(&f.state.db,&f.chat,"proxy-card",true).await.unwrap();
        engine::activity_finished_for_turn(&f.state.db,&f.owner,&f.row.id,f.chat.turn_id.as_deref().unwrap(),&id,true,None).await.unwrap();
        let mut a=activities(&f).await;refresh_for(&f,&mut a).await;
        assert_eq!(a[0].decision.as_ref().unwrap().state,"completed");
    }).await;
    assert!(used.load(std::sync::atomic::Ordering::Relaxed));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authored_skill_requires_exact_pinned_publication() {
    let f = Box::pin(orchestrator_fixture("decision_skill")).await;
    let id = start(&f, &f.chat).await;
    let ack = card(&f, &id).await;
    f.state.db.collection::<Document>(ACKS).update_one(doc!{"_id":&ack.id},doc!{"$set":{"status":"used","authored_skill":{"agent_id":&f.chat.agent_id,"proposal_id":"proposal","revision":2_i64,"skills_revision":1_i64}}}).await.unwrap();
    let col = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_agent_learning::PROPOSALS_COLLECTION_NAME);
    col.insert_one(doc!{"_id":"proposal","agent_id":&f.chat.agent_id,"revision":2_i64,"status":"publishing","publication":{"acknowledgement_id":&ack.id}}).await.unwrap();
    let mut a = activities(&f).await;
    for (status, expected) in [
        ("publishing", "allowed"),
        ("pinned", "completed"),
        ("published_unpinned", "failed"),
    ] {
        col.update_one(doc! {"_id":"proposal"}, doc! {"$set":{"status":status}})
            .await
            .unwrap();
        refresh_for(&f, &mut a).await;
        assert_eq!(a[0].decision.as_ref().unwrap().state, expected);
    }
    col.update_one(
        doc! {"_id":"proposal"},
        doc! {"$set":{"revision":3_i64,"status":"pinned"}},
    )
    .await
    .unwrap();
    refresh_for(&f, &mut a).await;
    assert_eq!(
        a[0].decision.as_ref().unwrap().state,
        "allowed",
        "another revision cannot prove execution"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stopped_consumed_occurrence_preserves_cancellation_and_confirmed_effects() {
    for ok in [false, true] {
        let f = Box::pin(orchestrator_fixture("decision_stop")).await;
        let id = start(&f, &f.chat).await;
        let ack = card(&f, &id).await;
        acks::decide(&f.state.db, &f.owner, &f.row.id, &ack.id, true)
            .await
            .unwrap();
        assert!(
            ACTIVITY_ID
                .scope(
                    Some(id.clone()),
                    acks::consume_action(
                        &f.state.db,
                        &f.chat,
                        &ack.id,
                        "nyxid__create_schedule",
                        &json!({"id":"exact"})
                    )
                )
                .await
                .unwrap()
        );
        let mut a = activities(&f).await;
        engine::request_stop(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap();
        refresh_for(&f, &mut a).await;
        assert_eq!(a[0].decision.as_ref().unwrap().state, "cancelled");
        engine::activity_finished_for_turn(
            &f.state.db,
            &f.owner,
            &f.row.id,
            f.chat.turn_id.as_deref().unwrap(),
            &id,
            ok,
            None,
        )
        .await
        .unwrap();
        refresh_for(&f, &mut a).await;
        assert_eq!(
            a[0].decision.as_ref().unwrap().state,
            if ok { "completed" } else { "cancelled" }
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn approved_machine_submission_waits_for_its_job_and_preserves_cancellation() {
    let f = Box::pin(orchestrator_fixture("decision_machine")).await;
    let id = start(&f, &f.chat).await;
    let ack = card(&f, &id).await;
    acks::decide(&f.state.db, &f.owner, &f.row.id, &ack.id, true)
        .await
        .unwrap();
    assert!(
        ACTIVITY_ID
            .scope(
                Some(id.clone()),
                acks::consume_action(
                    &f.state.db,
                    &f.chat,
                    &ack.id,
                    "nyxid__create_schedule",
                    &json!({"id":"exact"})
                )
            )
            .await
            .unwrap()
    );
    let receipt = crate::models::machine_receipt::MachineReceipt {
        operation_id: "operation".into(),
        node_id: "machine".into(),
        agent_id: f.chat.agent_id.clone(),
        context_mode: None,
        action: "command.exec".into(),
        status: "running".into(),
        job_id: Some("job".into()),
        exit_code: None,
        bytes: None,
        duration_ms: None,
        error_code: None,
        screenshot_id: None,
        preview_id: None,
        preview_enabled: false,
    };
    ACTIVITY_ID
        .scope(
            Some(id.clone()),
            crate::services::machine_activity_service::record(&f.state.db, &f.chat, &receipt),
        )
        .await
        .unwrap();
    engine::activity_finished_for_turn(
        &f.state.db,
        &f.owner,
        &f.row.id,
        f.chat.turn_id.as_deref().unwrap(),
        &id,
        true,
        None,
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>("machine_jobs")
        .insert_one(doc! {"_id":"job","user_id":&f.owner,"state":"running"})
        .await
        .unwrap();
    let mut a = activities(&f).await;
    for (state, expected, exit) in [
        ("running", "allowed", 0_i64),
        ("finished", "completed", 0),
        ("finished", "failed", 1),
        ("cancelled", "cancelled", 0),
    ] {
        f.state
            .db
            .collection::<Document>("machine_jobs")
            .update_one(
                doc! {"_id":"job"},
                doc! {"$set":{"state":state,"exit_code":exit}},
            )
            .await
            .unwrap();
        refresh_for(&f, &mut a).await;
        assert_eq!(a[0].decision.as_ref().unwrap().state, expected);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn connected_proxy_response_completion_never_changes_bytes_and_drop_is_unconfirmed() {
    use axum::body::{Body, to_bytes};
    for complete in [true, false] {
        let f = Box::pin(orchestrator_fixture("decision_proxy_body")).await;
        let id = start(&f, &f.chat).await;
        f.state.db.collection::<Document>(APPROVALS).insert_one(doc!{"_id":"card","user_id":&f.owner,"status":"approved","expires_at":bson::DateTime::from_millis(i64::MAX)}).await.unwrap();
        ACTIVITY_ID
            .scope(
                Some(id.clone()),
                approval(&f.state.db, &f.chat, "card", true),
            )
            .await
            .unwrap();
        let response = axum::response::Response::new(Body::from("unchanged provider bytes"));
        let response = proxy_response(
            &f.state.db,
            &f.chat,
            f.chat.turn_id.as_deref().unwrap(),
            &id,
            false,
            Ok(response),
        )
        .await
        .unwrap();
        if complete {
            assert_eq!(
                &to_bytes(response.into_body(), 1024).await.unwrap()[..],
                b"unchanged provider bytes"
            );
        } else {
            drop(response);
        }
        let mut a = activities(&f).await;
        refresh_for(&f, &mut a).await;
        assert_eq!(
            a[0].decision.as_ref().unwrap().state,
            if complete { "completed" } else { "allowed" }
        );
        if !complete {
            engine::request_stop(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap();
            refresh_for(&f, &mut a).await;
            assert_eq!(a[0].decision.as_ref().unwrap().state, "cancelled");
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn legacy_history_without_links_needs_no_database_read() {
    let state = crate::test_utils::test_app_state_no_db().await;
    let mut activity: TurnActivity = bson::from_document(
        doc! {"id":"legacy","label":"tool","status":"error","started_at":bson::DateTime::now()},
    )
    .unwrap();
    refresh(&state.db, "owner", "thread", vec![&mut activity])
        .await
        .unwrap();
    assert!(activity.decision.is_none());
}

#[tokio::test]
async fn exact_unusable_action_card_preserves_refusal_and_decision_state() {
    let f = Box::pin(orchestrator_fixture("decision_unusable")).await;
    let first = start(&f, &f.chat).await;
    let ack = card(&f, &first).await;
    for (status, expected) in [
        ("pending", "waiting"),
        ("denied", "denied"),
        ("expired", "expired"),
    ] {
        f.state
            .db
            .collection::<Document>(ACKS)
            .update_one(doc! {"_id":&ack.id}, doc! {"$set":{"status":status}})
            .await
            .unwrap();
        let activity = start(&f, &f.chat).await;
        let refusal = ACTIVITY_ID
            .scope(
                Some(activity.clone()),
                acks::confirm_action_with(
                    &f.state.db,
                    &f.chat,
                    "nyxid__create_schedule",
                    &json!({"id":"exact", "acknowledgement_id":ack.id}),
                    &acks::ConfirmationNeed {
                        org_instruction: Default::default(),
                        class: acks::OwnerActionClass::Change,
                        mode: None,
                        webhook: false,
                        policy: true,
                        delegation: false,
                    },
                    "Confirm scheduled automation",
                    acks::InvalidCard::Refuse,
                ),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refusal["error"], "acknowledgement_invalid");
        let mut activities = activities(&f).await;
        refresh_for(&f, &mut activities).await;
        let linked = activities
            .iter()
            .find(|a| a.id == activity)
            .unwrap()
            .decision
            .as_ref()
            .unwrap();
        assert_eq!(linked.id, ack.id);
        assert_eq!(linked.state, expected);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_connection_consent_is_actor_scoped_and_never_claims_completion() {
    let f = Box::pin(orchestrator_fixture("decision_consent")).await;
    let id = start(&f, &f.chat).await;
    let consents = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME);
    consents.insert_one(doc!{"_id":"consent","actor_user_id":&f.owner,"conversation_id":&f.row.id,"decision":"pending","expires_at":bson::DateTime::from_millis(i64::MAX)}).await.unwrap();
    ACTIVITY_ID
        .scope(
            Some(id),
            bind(
                &f.state.db,
                &f.chat,
                "shared_consent",
                "consent",
                false,
                None,
            ),
        )
        .await
        .unwrap();
    let mut activities = activities(&f).await;
    for (decision, expected) in [
        ("pending", "waiting"),
        ("once", "allowed"),
        ("always", "allowed"),
        ("consumed", "allowed"),
        ("skip", "denied"),
        ("revoked", "unavailable"),
    ] {
        consents
            .update_one(doc! {"_id":"consent"}, doc! {"$set":{"decision":decision}})
            .await
            .unwrap();
        refresh_for(&f, &mut activities).await;
        assert_eq!(activities[0].decision.as_ref().unwrap().state, expected);
    }
    consents
        .update_one(
            doc! {"_id":"consent"},
            doc! {"$set":{"decision":"pending","expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    refresh_for(&f, &mut activities).await;
    assert_eq!(activities[0].decision.as_ref().unwrap().state, "expired");
    consents
        .update_one(
            doc! {"_id":"consent"},
            doc! {"$set":{"actor_user_id":"another-person"}},
        )
        .await
        .unwrap();
    refresh_for(&f, &mut activities).await;
    assert_eq!(
        activities[0].decision.as_ref().unwrap().state,
        "unavailable"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn approved_proxy_handshake_never_claims_stream_completion_or_failure() {
    for (status, connect) in [(101, false), (200, true)] {
        let f = Box::pin(orchestrator_fixture("decision_tunnel")).await;
        let id = start(&f, &f.chat).await;
        f.state.db.collection::<Document>(APPROVALS).insert_one(doc!{"_id":"card","user_id":&f.owner,"status":"approved","expires_at":bson::DateTime::from_millis(i64::MAX)}).await.unwrap();
        ACTIVITY_ID
            .scope(
                Some(id.clone()),
                approval(&f.state.db, &f.chat, "card", true),
            )
            .await
            .unwrap();
        let response = axum::response::Response::builder()
            .status(status)
            .body(axum::body::Body::empty())
            .unwrap();
        let response = proxy_response(
            &f.state.db,
            &f.chat,
            f.chat.turn_id.as_deref().unwrap(),
            &id,
            connect,
            Ok(response),
        )
        .await
        .unwrap();
        assert_eq!(response.status().as_u16(), status);
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let mut activities = activities(&f).await;
        refresh_for(&f, &mut activities).await;
        assert_eq!(activities[0].decision.as_ref().unwrap().state, "allowed");
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn connected_proxy_keeps_trailers_as_well_as_data_frames() {
    use http_body_util::BodyExt;
    let f = Box::pin(orchestrator_fixture("decision_proxy_frames")).await;
    let id = start(&f, &f.chat).await;
    f.state.db.collection::<Document>(APPROVALS).insert_one(doc!{"_id":"card","user_id":&f.owner,"status":"approved","expires_at":bson::DateTime::from_millis(i64::MAX)}).await.unwrap();
    ACTIVITY_ID
        .scope(
            Some(id.clone()),
            approval(&f.state.db, &f.chat, "card", true),
        )
        .await
        .unwrap();
    let trailers = axum::http::HeaderMap::from_iter([(
        axum::http::HeaderName::from_static("x-checksum"),
        axum::http::HeaderValue::from_static("test-value"),
    )]);
    let frames = futures::stream::iter(vec![
        Ok::<_, std::io::Error>(hyper::body::Frame::data(bytes::Bytes::from_static(b"body"))),
        Ok(hyper::body::Frame::trailers(trailers.clone())),
    ]);
    let body = axum::body::Body::new(http_body_util::StreamBody::new(frames));
    let response = proxy_response(
        &f.state.db,
        &f.chat,
        f.chat.turn_id.as_deref().unwrap(),
        &id,
        false,
        Ok(axum::response::Response::new(body)),
    )
    .await
    .unwrap();
    let collected = response.into_body().collect().await.unwrap();
    assert_eq!(collected.trailers(), Some(&trailers));
    assert_eq!(collected.to_bytes(), bytes::Bytes::from_static(b"body"));
    let mut activities = activities(&f).await;
    refresh_for(&f, &mut activities).await;
    assert_eq!(activities[0].decision.as_ref().unwrap().state, "completed");
    f.state.db.drop().await.unwrap();
}
