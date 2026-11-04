use super::*;
use crate::models::{assistant_conversation::AssistantConversation, audit_log::AuditLog};
use crate::services::assistant_agent_credential_service as credentials;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn start(group: &AssistantGroup, agent: &AssistantAgent) -> TurnStart {
    let mut start = TurnStart::from(&engine::TurnRequest {
        conversation_id: None,
        text: "private turn sentinel".into(),
        agent_id: Some(agent.id.clone()),
        model: None,
        access_mode: None,
        attachment_ids: vec![],
        acknowledgement_id: None,
        retry_of: None,
    });
    start.group_id = Some(group.id.clone());
    start.origin = TurnOrigin::Group;
    start
}
async fn begin(
    state: &AppState,
    group: &AssistantGroup,
    agent: &AssistantAgent,
) -> AssistantConversation {
    engine::begin_turn(
        &state.db,
        OWNER,
        start(group, agent),
        &state.encryption_keys,
    )
    .await
    .unwrap()
}
async fn finish(
    state: &AppState,
    row: &AssistantConversation,
) -> AppResult<Option<engine::TurnError>> {
    engine::finish_turn(
        &state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "late reply sentinel".into(),
            session_id: Some("old-session".into()),
            response_id: None,
            error: None,
        },
    )
    .await
}
async fn assert_old_key_refused(state: &AppState, key: &str) {
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let router = router.with_state(state.clone());
    for (method, path, body) in [
        (
            "POST",
            "/mcp",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
        ),
        ("GET", "/api/v1/proxy/s/llm-nyx/v1/models", ""),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {key}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
}

async fn assert_key_revoked(state: &AppState, key_id: &str) {
    let key = state
        .db
        .collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":key_id,"user_id":OWNER})
        .await
        .unwrap()
        .unwrap();
    assert!(!key.is_active);
    // The public lookup deliberately hides inactive keys.
    assert!(matches!(
        key_service::get_api_key(&state.db, OWNER, key_id).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn crew_removal_revokes_key_settles_turn_and_readd_is_fresh() {
    let (state, calls, server) = setup("crew_removal_revocation").await;
    let remaining = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let removed = researcher(&state).await;
    let group = group_with(&state, "Crew", &[&remaining, &removed]).await;
    let keep = begin(&state, &group, &remaining).await;
    let old = begin(&state, &group, &removed).await;
    let credential =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, OWNER, &old.id)
            .await
            .unwrap()
            .unwrap();
    let public = groups::append(
        &state.db,
        OWNER,
        &group.id,
        "agent",
        Some(&removed),
        "Previously published",
        vec![],
    )
    .await
    .unwrap();
    for row in [&keep, &old] {
        state
            .db
            .collection::<bson::Document>("assistant_voice_announcements")
            .insert_one(doc! {
                "_id": &row.id, "user_id": OWNER, "conversation_id": &row.id,
                "text": "Private voice notification",
            })
            .await
            .unwrap();
        state.db.collection::<bson::Document>("assistant_acknowledgements").insert_one(doc!{
            "_id":&row.id,"user_id":OWNER,"conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,"status":"pending"
        }).await.unwrap();
        for status in ["pending", "approved"] {
            state.db.collection::<bson::Document>("approval_requests").insert_one(doc!{
                "_id":format!("{}-{status}",row.id),"user_id":if status=="approved" {"34345678-1234-4123-8123-1234567890ab"} else {OWNER},"requester_type":"api_key","requester_id":&row.credential_api_key_id,
                "status":status
            }).await.unwrap();
        }
        state.db.collection::<bson::Document>("approval_grants").insert_one(doc!{
            "_id":&row.id,"user_id":OWNER,"requester_type":"api_key","requester_id":&row.credential_api_key_id,"revoked":false,"org_scoped":false
        }).await.unwrap();
    }
    groups::persist_last_recipients(
        &state.db,
        OWNER,
        &group.id,
        &[removed.id.clone(), remaining.id.clone()],
    )
    .await
    .unwrap();
    groups::address(
        &state.db,
        OWNER,
        &group.id,
        &[removed.id.clone(), remaining.id.clone()],
    )
    .await
    .unwrap();
    groups::update(
        &state.db,
        OWNER,
        &group.id,
        None,
        Some(std::slice::from_ref(&remaining.id)),
    )
    .await
    .unwrap();
    assert_old_key_refused(&state, &credential.raw_key).await;
    assert_key_revoked(&state, &old.credential_api_key_id).await;
    let threads = groups::member_threads(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, keep.id);
    assert_eq!(threads[0].credential_api_key_id, keep.credential_api_key_id);
    assert_eq!(
        threads[0].active_turn.as_ref().unwrap().turn_id,
        keep.active_turn.as_ref().unwrap().turn_id
    );
    assert!(!threads[0].active_turn.as_ref().unwrap().stop_requested);
    assert!(
        key_service::get_api_key(&state.db, OWNER, &keep.credential_api_key_id)
            .await
            .unwrap()
            .is_active
    );
    for collection in [
        "assistant_agent_credentials",
        "assistant_acknowledgements",
        "assistant_messages",
        "assistant_voice_announcements",
    ] {
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {"conversation_id":&old.id})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("assistant_voice_announcements")
            .count_documents(doc! {"conversation_id": &keep.id})
            .await
            .unwrap(),
        1,
        "Retiring one crew thread preserves another thread's voice notifications",
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("approval_requests")
            .count_documents(doc! {"requester_id":&old.credential_api_key_id,"status":"expired"})
            .await
            .unwrap(),
        2
    );
    assert_eq!(state.db.collection::<bson::Document>("approval_requests").count_documents(doc!{"requester_id":&keep.credential_api_key_id,"status":{"$in":["pending","approved"]}}).await.unwrap(),2);
    assert!(
        state
            .db
            .collection::<bson::Document>("approval_grants")
            .find_one(doc! {"_id":&old.id})
            .await
            .unwrap()
            .unwrap()
            .get_bool("revoked")
            .unwrap()
    );
    assert!(
        !state
            .db
            .collection::<bson::Document>("approval_grants")
            .find_one(doc! {"_id":&keep.id})
            .await
            .unwrap()
            .unwrap()
            .get_bool("revoked")
            .unwrap()
    );
    let current = groups::get(&state.db, OWNER, &group.id).await.unwrap();
    assert_eq!(current.last_recipient_ids, vec![remaining.id.clone()]);
    assert_eq!(current.pending_agent_ids, vec![remaining.id.clone()]);
    assert!(
        groups::messages(&state.db, OWNER, &group.id, 100, None)
            .await
            .unwrap()
            .iter()
            .any(|m| m.id == public.id)
    );
    let stopped=state.db.collection::<AuditLog>("audit_log").find_one(doc!{"event_type":"assistant_turn_failed","event_data.conversation_id":&old.id,"event_data.code":"cancelled"}).await.unwrap().unwrap();
    assert_eq!(stopped.event_data.unwrap()["stage"], "stop");
    let audit=state.db.collection::<AuditLog>("audit_log").find_one(doc!{"event_type":"assistant_crew_member_thread_retired","event_data.conversation_id":&old.id}).await.unwrap().unwrap();
    let data = audit.event_data.unwrap();
    assert_eq!(data.as_object().unwrap().len(), 3);
    assert_eq!(data["group_id"], group.id);
    assert_eq!(data["agent_id"], removed.id);
    assert!(calls.lock().await.is_empty());
    assert!(
        credentials::load_or_provision(&state.db, &state.encryption_keys, OWNER, &old.id)
            .await
            .is_err()
    );
    assert!(
        engine::begin_turn(
            &state.db,
            OWNER,
            start(&group, &removed),
            &state.encryption_keys
        )
        .await
        .is_err()
    );
    let readded = groups::update(
        &state.db,
        OWNER,
        &group.id,
        None,
        Some(&[remaining.id.clone(), removed.id.clone()]),
    )
    .await
    .unwrap();
    let fresh = begin(&state, &readded, &removed).await;
    assert_ne!(fresh.id, old.id);
    assert_ne!(fresh.credential_api_key_id, old.credential_api_key_id);
    assert!(fresh.nyxagent_session_id.is_none());
    assert_old_key_refused(&state, &credential.raw_key).await;
    assert!(matches!(
        finish(&state, &old).await,
        Err(AppError::NotFound(_))
    ));
    member_settled(&state, &old, "late reply sentinel", None).await;
    assert!(
        !groups::messages(&state.db, OWNER, &group.id, 100, None)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "late reply sentinel")
    );
    assert!(
        groups::append_member_reply(
            &state.db,
            &old,
            "agent",
            &removed,
            "late reply sentinel",
            vec![]
        )
        .await
        .is_err()
    );
    // Existing crew deletion still removes every remaining thread and key.
    groups::delete(&state.db, OWNER, &group.id).await.unwrap();
    assert!(
        groups::member_threads(&state.db, OWNER, &group.id)
            .await
            .unwrap()
            .is_empty()
    );
    for row in [&keep, &fresh] {
        assert_key_revoked(&state, &row.credential_api_key_id).await;
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("assistant_voice_announcements")
            .count_documents(doc! {"user_id": OWNER})
            .await
            .unwrap(),
        0,
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crew_removal_stale_edit_cannot_restore_removed_member() {
    let (state, _, server) = setup("crew_removal_stale_edit").await;
    let a = researcher(&state).await;
    let b = specialist(&state, "remaining").await;
    let c = specialist(&state, "new-member").await;
    let before = group_with(&state, "Crew", &[&a, &b]).await;
    let row = begin(&state, &before, &a).await;
    finish(&state, &row).await.unwrap();
    groups::update_prepared(&state.db, OWNER, &before, None, Some(vec![b.clone()]))
        .await
        .unwrap();
    let stale = groups::update_from_snapshot(
        &state.db,
        OWNER,
        &before,
        None,
        Some(&[a.id.clone(), b.id.clone(), c.id.clone()]),
    )
    .await;
    assert!(matches!(stale, Err(AppError::Conflict(_))));
    assert_eq!(
        groups::get(&state.db, OWNER, &before.id)
            .await
            .unwrap()
            .member_agent_ids,
        vec![b.id]
    );
    assert_key_revoked(&state, &row.credential_api_key_id).await;
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crew_removal_concurrent_settlement_and_admission_cannot_leave_authority() {
    let (state, _, server) = setup("crew_removal_races").await;
    let a = researcher(&state).await;
    let b = specialist(&state, "remaining").await;
    for n in 0..3 {
        let group = group_with(&state, &format!("Crew{n}"), &[&a, &b]).await;
        let row = begin(&state, &group, &a).await;
        let (removal, settlement) = tokio::join!(
            groups::update(
                &state.db,
                OWNER,
                &group.id,
                None,
                Some(std::slice::from_ref(&b.id))
            ),
            finish(&state, &row),
        );
        removal.unwrap();
        assert!(settlement.is_ok() || matches!(settlement, Err(AppError::NotFound(_))));
        assert!(
            groups::member_thread(&state.db, OWNER, &group.id, &a.id)
                .await
                .unwrap()
                .is_none()
        );
        assert_key_revoked(&state, &row.credential_api_key_id).await;
        let group = groups::update(
            &state.db,
            OWNER,
            &group.id,
            None,
            Some(&[a.id.clone(), b.id.clone()]),
        )
        .await
        .unwrap();
        let (removal, admission) = tokio::join!(
            groups::update(
                &state.db,
                OWNER,
                &group.id,
                None,
                Some(std::slice::from_ref(&b.id))
            ),
            engine::begin_turn(&state.db, OWNER, start(&group, &a), &state.encryption_keys),
        );
        match removal {
            Ok(_) => {
                assert!(
                    groups::member_thread(&state.db, OWNER, &group.id, &a.id)
                        .await
                        .unwrap()
                        .is_none()
                );
                if let Ok(row) = admission {
                    assert_key_revoked(&state, &row.credential_api_key_id).await;
                }
            }
            Err(AppError::AssistantTurnActive) => {
                // A fresh turn that wins preparation preserves the entire old
                // roster and authority; a retry settles it before removal.
                assert!(
                    groups::get(&state.db, OWNER, &group.id)
                        .await
                        .unwrap()
                        .member_agent_ids
                        .contains(&a.id)
                );
                groups::update(
                    &state.db,
                    OWNER,
                    &group.id,
                    None,
                    Some(std::slice::from_ref(&b.id)),
                )
                .await
                .unwrap();
            }
            Err(error) => panic!("unexpected removal error: {error}"),
        }
    }
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn crew_removal_retires_shared_and_org_members_without_changing_their_agents() {
    let (state, _, server) = setup("crew_removal_foreign_members").await;
    let keep = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let shared = shared_researcher(&state).await;
    let org = org_writer(&state).await;
    for agent in [&shared, &org] {
        let group = group_with(&state, "Crew", &[&keep, agent]).await;
        let row = begin(&state, &group, agent).await;
        let key =
            credentials::load_for_conversation(&state.db, &state.encryption_keys, OWNER, &row.id)
                .await
                .unwrap()
                .unwrap();
        // Retirement is authorized by the crew owner even after agent use
        // rights disappear; it must not depend on private-thread read ACLs.
        if agent.id == org.id {
            state
                .db
                .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
                .update_many(
                    doc! {"org_user_id":&org.user_id,"member_user_id":OWNER},
                    doc! {"$set":{"revoked_at":bson::DateTime::now()}},
                )
                .await
                .unwrap();
        }
        groups::update(
            &state.db,
            OWNER,
            &group.id,
            None,
            Some(std::slice::from_ref(&keep.id)),
        )
        .await
        .unwrap();
        assert_old_key_refused(&state, &key.raw_key).await;
        assert!(
            groups::member_threads(&state.db, OWNER, &group.id)
                .await
                .unwrap()
                .is_empty()
        );
        if agent.id == org.id {
            state
                .db
                .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
                .update_many(
                    doc! {"org_user_id":&org.user_id,"member_user_id":OWNER},
                    doc! {"$set":{"revoked_at":bson::Bson::Null}},
                )
                .await
                .unwrap();
        }
        assert!(
            team::agent(&state.db, OWNER, &agent.id)
                .await
                .unwrap()
                .destroyed_at
                .is_none()
        );
        let group = groups::update(
            &state.db,
            OWNER,
            &group.id,
            None,
            Some(&[keep.id.clone(), agent.id.clone()]),
        )
        .await
        .unwrap();
        let fresh = begin(&state, &group, agent).await;
        assert_ne!(fresh.id, row.id);
        assert_ne!(fresh.credential_api_key_id, row.credential_api_key_id);
    }
    server.abort();
    state.db.drop().await.unwrap();
}
