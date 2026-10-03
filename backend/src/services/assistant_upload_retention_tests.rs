use super::*;
use crate::{
    models::{
        assistant_upload::AssistantUpload,
        user::{COLLECTION_NAME as USERS, UserType},
    },
    services::{
        assistant_nyxagent as engine, assistant_team_service as team,
        assistant_upload_service as uploads,
    },
    test_utils::{connect_transaction_test_database, test_app_state, test_auth_user, test_user},
};
use uuid::Uuid;

async fn fixture() -> (AppState, String, String) {
    let db = connect_transaction_test_database("upload_retention").await;
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db);
    ensure_settings(&state.db).await.unwrap();
    let agent = team::ensure_nyxbot(&state.db, &owner).await.unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let thread = team::create_thread_for(
        &state.db,
        &state.encryption_keys,
        &owner,
        &agent,
        "retention",
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    (state, owner, thread.id)
}
async fn policy(state: &AppState, owner: &str, p: Policy) {
    Box::pin(update(
        &state.db,
        &AuditActor::from_auth_user(&test_auth_user(owner)),
        state.audit_chain_hmac_key.as_slice(),
        Some(p),
    ))
    .await
    .unwrap();
}
async fn root(
    state: &AppState,
    owner: &str,
    conversation: &str,
    mime: &str,
    bound: bool,
    age: Duration,
) -> String {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now() - age;
    let extracted = super::super::attachment_extraction::Extracted {
        content_type: mime.into(),
        text: "private extracted text".into(),
        sections: Vec::new(),
    };
    let row = AssistantUpload {
        id: id.clone(),
        origin: "user_upload".into(),
        user_id: owner.into(),
        conversation_id: conversation.into(),
        group_id: None,
        message_id: bound.then(|| Uuid::new_v4().to_string()),
        label: "private.txt".into(),
        content_type: mime.into(),
        size: 8,
        pages: 0,
        chunks: 2,
        text_encrypted: state
            .encryption_keys
            .encrypt(&serde_json::to_vec(&extracted).unwrap())
            .await
            .unwrap(),
        created_at: now,
        expires_at: None,
        bound_at: bound.then_some(now),
    };
    state
        .db
        .collection(ATTACHMENTS)
        .insert_one(row)
        .await
        .unwrap();
    for (i, bytes) in [b"abcd", b"efgh"].iter().enumerate() {
        state.db.collection::<Document>(ATTACHMENTS).insert_one(doc! {"_id":Uuid::new_v4().to_string(),"parent_attachment_id":&id,"user_id":owner,"conversation_id":conversation,
            "chunk_index":i as i64,"data_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:state.encryption_keys.encrypt(*bytes).await.unwrap()}}).await.unwrap();
    }
    id
}
async fn read(
    state: &AppState,
    owner: &str,
    conversation: &str,
    id: &str,
) -> AppResult<(String, Vec<u8>)> {
    Box::pin(uploads::owner_read(
        &state.db,
        &state.encryption_keys,
        owner,
        conversation,
        id,
    ))
    .await
}
#[test]
fn defaults_bounds_and_legacy_binding_clock() {
    assert_eq!(Policy::default().pending_hours, 24);
    assert_eq!(Policy::default().tool_image_days, None);
    for hours in [0, 8761] {
        assert!(
            validate(&Policy {
                pending_hours: hours,
                ..Policy::default()
            })
            .is_err()
        );
    }
    for days in [0, 366] {
        assert!(
            validate(&Policy {
                image_days: days,
                ..Policy::default()
            })
            .is_err()
        );
        assert!(
            validate(&Policy {
                document_days: days,
                ..Policy::default()
            })
            .is_err()
        );
        assert!(
            validate(&Policy {
                tool_image_days: Some(days),
                ..Policy::default()
            })
            .is_err()
        );
    }
    validate(&Policy {
        pending_hours: 8760,
        image_days: 365,
        document_days: 1,
        tool_image_days: Some(365),
        ..Policy::default()
    })
    .unwrap();
    let now = Utc::now();
    let row = doc! {"origin":"user_upload","message_id":"m","content_type":"text/plain","created_at":bson::DateTime::from_chrono(now-Duration::days(40)),"expires_at":bson::DateTime::from_chrono(now+Duration::days(29))};
    assert!(!age_expired(&row, Policy::default(), now));
    assert!(age_expired(
        &row,
        Policy {
            document_days: 1,
            ..Policy::default()
        },
        now
    ));
}
#[tokio::test]
async fn current_policy_shortening_refuses_reads_before_sweep_and_refresh() {
    let (state, owner, thread) = fixture().await;
    let id = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        true,
        Duration::days(2),
    )
    .await;
    assert_eq!(
        read(&state, &owner, &thread, &id).await.unwrap().1,
        b"abcdefgh"
    );
    refresh(&state.db, &state.upload_retention).await.unwrap();
    policy(
        &state,
        &owner,
        Policy {
            document_days: 1,
            ..Policy::default()
        },
    )
    .await;
    assert_eq!(state.upload_retention.read().unwrap().settings.revision, 0);
    assert!(matches!(
        read(&state, &owner, &thread, &id).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    assert!(
        expired_ids(&state.db, &owner, std::slice::from_ref(&id))
            .await
            .unwrap()
            .contains(&id)
    );
    assert!(
        state
            .db
            .collection::<Document>(ATTACHMENTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .is_some()
    );
    // Extending a policy can preserve bytes while they still exist.
    policy(&state, &owner, Policy::default()).await;
    assert!(read(&state, &owner, &thread, &id).await.is_ok());
}
#[tokio::test]
async fn pending_image_and_tool_retention_use_distinct_clocks() {
    let (state, owner, thread) = fixture().await;
    let pending = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        false,
        Duration::hours(2),
    )
    .await;
    let image = root(
        &state,
        &owner,
        &thread,
        "image/png",
        true,
        Duration::days(2),
    )
    .await;
    let tool = Uuid::new_v4().to_string();
    state.db.collection::<Document>(ATTACHMENTS).insert_one(doc! {"_id":&tool,"user_id":&owner,"conversation_id":&thread,"content_type":"image/png","created_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(400))}).await.unwrap();
    let filter = doc! {"_id":&tool,"user_id":&owner,"conversation_id":&thread};
    require_available(&state.db, filter.clone()).await.unwrap();
    policy(
        &state,
        &owner,
        Policy {
            pending_hours: 1,
            image_days: 1,
            tool_image_days: Some(365),
            ..Policy::default()
        },
    )
    .await;
    for id in [&pending, &image] {
        assert!(matches!(
            read(&state, &owner, &thread, id).await,
            Err(AppError::AssistantAttachmentExpired)
        ));
    }
    assert!(matches!(
        require_available(&state.db, filter).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
}
#[tokio::test]
async fn sweep_deletes_root_chunks_and_text_preserves_scoped_placeholder_and_is_idempotent() {
    let (state, owner, thread) = fixture().await;
    let id = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        true,
        Duration::days(2),
    )
    .await;
    policy(
        &state,
        &owner,
        Policy {
            document_days: 1,
            ..Policy::default()
        },
    )
    .await;
    assert_eq!(sweep(&state.db).await.unwrap(), 1);
    assert_eq!(
        state
            .db
            .collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {"$or":[{"_id":&id},{"parent_attachment_id":&id}]})
            .await
            .unwrap(),
        0
    );
    let tomb = state
        .db
        .collection::<Document>(TOMBSTONES)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !tomb.contains_key("text_encrypted")
            && !tomb.contains_key("label")
            && !tomb.contains_key("data_encrypted")
    );
    assert!(matches!(
        read(&state, &owner, &thread, &id).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    assert!(matches!(
        engine::read_attachment(&state.db, &state.encryption_keys, &owner, &thread, &id).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    assert!(matches!(
        require_available(
            &state.db,
            doc! {"_id":&id,"user_id":"other","conversation_id":&thread}
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        require_available(
            &state.db,
            doc! {"_id":&id,"user_id":&owner,"conversation_id":"other"}
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(sweep(&state.db).await.unwrap(), 0);
    assert!(
        expired_ids(&state.db, &owner, std::slice::from_ref(&id))
            .await
            .unwrap()
            .contains(&id)
    );
    engine::delete(&state.db, &owner, &thread).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<Document>(TOMBSTONES)
            .count_documents(doc! {"user_id":&owner})
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn sweep_stale_lease_and_stale_policy_cannot_delete() {
    let (state, owner, thread) = fixture().await;
    let id = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        true,
        Duration::days(2),
    )
    .await;
    policy(
        &state,
        &owner,
        Policy {
            document_days: 1,
            ..Policy::default()
        },
    )
    .await;
    let runtime = cluster_lease_runtime();
    let old = runtime.acquire(&state.db, LEASE).await.unwrap().unwrap();
    assert_eq!(sweep(&state.db).await.unwrap(), 0);
    LeaseStore::release(&state.db, &old).await.unwrap();
    let fresh = runtime.acquire(&state.db, LEASE).await.unwrap().unwrap();
    assert!(!delete_fenced(&state.db, &old, &id).await.unwrap());
    policy(&state, &owner, Policy::default()).await;
    assert!(!delete_fenced(&state.db, &fresh, &id).await.unwrap());
    assert!(read(&state, &owner, &thread, &id).await.is_ok());
    LeaseStore::release(&state.db, &fresh).await.unwrap();
}
#[tokio::test]
async fn replicas_refresh_settings_and_reset_monotonically() {
    let (state, owner, _) = fixture().await;
    let other: Cache = Default::default();
    refresh(&state.db, &other).await.unwrap();
    policy(
        &state,
        &owner,
        Policy {
            pending_hours: 2,
            ..Policy::default()
        },
    )
    .await;
    assert_eq!(other.read().unwrap().settings.policy, None);
    refresh(&state.db, &other).await.unwrap();
    assert_eq!(
        other.read().unwrap().settings.policy.unwrap().pending_hours,
        2
    );
    update(
        &state.db,
        &AuditActor::from_auth_user(&test_auth_user(&owner)),
        state.audit_chain_hmac_key.as_slice(),
        None,
    )
    .await
    .unwrap();
    refresh(&state.db, &other).await.unwrap();
    let snapshot = other.read().unwrap();
    assert_eq!(snapshot.settings.policy, None);
    assert_eq!(snapshot.settings.revision, 2);
    assert!(snapshot.refreshed_at.is_some());
}
#[tokio::test]
async fn first_turn_deletion_is_atomic_with_real_settlement_and_leaves_documents() {
    let (state, owner, thread) = fixture().await;
    policy(
        &state,
        &owner,
        Policy {
            images_delete_after_turn: true,
            ..Policy::default()
        },
    )
    .await;
    let image = root(
        &state,
        &owner,
        &thread,
        "image/png",
        false,
        Duration::zero(),
    )
    .await;
    let document = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        false,
        Duration::zero(),
    )
    .await;
    let row = Box::pin(engine::begin_turn(
        &state.db,
        &owner,
        &engine::TurnRequest {
            conversation_id: Some(thread.clone()),
            attachment_ids: vec![image.clone(), document.clone()],
            text: "Read files".into(),
            agent_id: None,
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    assert!(read(&state, &owner, &thread, &image).await.is_ok());
    Box::pin(engine::finish_turn(
        &state.db,
        &row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "done".into(),
            session_id: None,
            response_id: None,
            error: Some(engine::TurnError::new("cancelled")),
        },
    ))
    .await
    .unwrap();
    assert!(matches!(
        read(&state, &owner, &thread, &image).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    assert!(
        state
            .db
            .collection::<Document>(ATTACHMENTS)
            .find_one(doc! {"_id":image})
            .await
            .unwrap()
            .is_none()
    );
    assert!(read(&state, &owner, &thread, &document).await.is_ok());
}
#[tokio::test]
async fn legacy_settled_images_expire_when_option_is_enabled_later() {
    let (state, owner, thread) = fixture().await;
    let image = root(
        &state,
        &owner,
        &thread,
        "image/png",
        true,
        Duration::hours(1),
    )
    .await;
    state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME).insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&owner,"conversation_id":&thread,"turn_id":"previous","role":"user","created_at":bson::DateTime::now(),"attachments":[{"id":&image}]}).await.unwrap();
    policy(
        &state,
        &owner,
        Policy {
            images_delete_after_turn: true,
            ..Policy::default()
        },
    )
    .await;
    assert!(matches!(
        read(&state, &owner, &thread, &image).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    assert_eq!(sweep(&state.db).await.unwrap(), 1);
}
#[tokio::test]
async fn ttl_migration_removes_only_attachment_ttl_and_is_repeatable() {
    let (state, _, _) = fixture().await;
    for name in [ATTACHMENTS, "assistant_upload_limits"] {
        state
            .db
            .collection::<Document>(name)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"expires_at":1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .expire_after(StdDuration::ZERO)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
    }
    ensure_indexes(&state.db).await.unwrap();
    ensure_indexes(&state.db).await.unwrap();
    for (name, ttl_expected) in [(ATTACHMENTS, false), ("assistant_upload_limits", true)] {
        let indexes: Vec<_> = state
            .db
            .collection::<Document>(name)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(
            indexes
                .iter()
                .any(|i| i.options.as_ref().and_then(|o| o.expire_after).is_some()),
            ttl_expected
        );
    }
}
#[tokio::test]
async fn pending_expiry_is_not_retained_and_owner_purge_removes_tombstones() {
    let (state, owner, thread) = fixture().await;
    let pending = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        false,
        Duration::hours(25),
    )
    .await;
    root(
        &state,
        &owner,
        &thread,
        "text/plain",
        true,
        Duration::days(31),
    )
    .await;
    assert_eq!(sweep(&state.db).await.unwrap(), 2);
    assert!(
        state
            .db
            .collection::<Document>(TOMBSTONES)
            .find_one(doc! {"_id":pending})
            .await
            .unwrap()
            .is_none()
    );
    crate::services::admin_user_service::delete_current_user_cascade(&state.db, &owner)
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<Document>(TOMBSTONES)
            .count_documents(doc! {"user_id":owner})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn upload_retention_tool_and_transcript_report_expiry_before_and_after_cleanup() {
    use crate::handlers::assistant_nyxagent as handlers;
    use axum::extract::{Path, Query, State};
    let (state, owner, thread) = fixture().await;
    let id = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        false,
        Duration::zero(),
    )
    .await;
    let row = Box::pin(engine::begin_turn(
        &state.db,
        &owner,
        &engine::TurnRequest {
            conversation_id: Some(thread.clone()),
            attachment_ids: vec![id.clone()],
            text: "Read".into(),
            agent_id: None,
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    let chat = super::super::assistant_acknowledgement_service::for_key(
        &state.db,
        &owner,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        uploads::read(
            &state.db,
            &state.encryption_keys,
            &chat,
            &serde_json::json!({"attachment_id":&id})
        )
        .await
        .is_ok()
    );
    state
        .db
        .collection::<Document>(ATTACHMENTS)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"bound_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(2))}},
        )
        .await
        .unwrap();
    policy(
        &state,
        &owner,
        Policy {
            document_days: 1,
            ..Policy::default()
        },
    )
    .await;
    for swept in [false, true] {
        if swept {
            assert_eq!(sweep(&state.db).await.unwrap(), 1);
        }
        let error = Box::pin(uploads::read(
            &state.db,
            &state.encryption_keys,
            &chat,
            &serde_json::json!({"attachment_id":&id}),
        ))
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::AssistantAttachmentExpired));
        assert!(error.to_string().contains("expired per retention policy"));
        let tool = super::super::assistant_account_tools::error_result(
            AppError::AssistantAttachmentExpired,
        );
        assert_eq!(tool.value["error_code"], 12101);
        assert!(tool.is_error);
        assert!(
            tool.value["message"]
                .as_str()
                .unwrap()
                .contains("expired per retention policy")
        );
        use axum::response::IntoResponse;
        assert_eq!(error.into_response().status(), axum::http::StatusCode::GONE);
        let result = Box::pin(handlers::history(
            State(state.clone()),
            test_auth_user(&owner),
            Path(thread.clone()),
            Query(Default::default()),
        ))
        .await
        .unwrap();
        let json = serde_json::to_value(result.0).unwrap();
        assert_eq!(json["messages"][0]["attachments"][0]["expired"], true);
    }
}

#[tokio::test]
async fn upload_retention_sweep_is_bounded_and_collects_legacy_orphan_chunks() {
    let (state, owner, thread) = fixture().await;
    let rows: Vec<_> = (0..BATCH + 1)
        .map(|i| {
            doc! {
                "_id":format!("root-{i:04}"), "origin":"user_upload", "user_id":&owner,
                "conversation_id":&thread, "content_type":"text/plain",
                "created_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(25)),
            }
        })
        .collect();
    state
        .db
        .collection::<Document>(ATTACHMENTS)
        .insert_many(rows)
        .await
        .unwrap();
    state.db.collection::<Document>(ATTACHMENTS).insert_one(doc! {"_id":"orphan","parent_attachment_id":"gone","user_id":&owner,"data_encrypted":"old TTL orphan"}).await.unwrap();
    assert_eq!(sweep(&state.db).await.unwrap(), BATCH as usize);
    assert_eq!(
        state
            .db
            .collection::<Document>(ATTACHMENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(sweep(&state.db).await.unwrap(), 1);
    assert_eq!(sweep(&state.db).await.unwrap(), 0);
}

#[tokio::test]
async fn upload_retention_audit_failure_rolls_back_policy() {
    let (state, owner, _) = fixture().await;
    let audit = crate::models::audit_log::COLLECTION_NAME;
    state
        .db
        .collection::<Document>(audit)
        .insert_one(doc! {"_id":"sentinel"})
        .await
        .unwrap();
    state.db.run_command(doc! {"collMod":audit,"validator":{"event_type":{"$ne":"admin_upload_retention_updated"}}}).await.unwrap();
    assert!(
        Box::pin(update(
            &state.db,
            &AuditActor::from_auth_user(&test_auth_user(&owner)),
            state.audit_chain_hmac_key.as_slice(),
            Some(Policy {
                pending_hours: 2,
                ..Policy::default()
            })
        ))
        .await
        .is_err()
    );
    assert_eq!(load(&state.db).await.unwrap().revision, 0);
    assert_eq!(load(&state.db).await.unwrap().policy, None);
}

#[tokio::test]
async fn upload_retention_group_placeholder_and_deletion_keep_scope() {
    use super::super::assistant_group_service as groups;
    use axum::extract::{Path, Query, State};
    let (state, owner, thread) = fixture().await;
    let agent = team::ensure_nyxbot(&state.db, &owner).await.unwrap();
    let group = groups::create(&state.db, &owner, "Files", &[agent.id], "user")
        .await
        .unwrap();
    let id = root(
        &state,
        &owner,
        &thread,
        "text/plain",
        false,
        Duration::zero(),
    )
    .await;
    state
        .db
        .collection::<Document>(ATTACHMENTS)
        .update_many(
            doc! {"$or":[{"_id":&id},{"parent_attachment_id":&id}]},
            doc! {"$set":{"conversation_id":"","group_id":&group.id}},
        )
        .await
        .unwrap();
    Box::pin(groups::append_with_uploads(
        &state.db,
        &owner,
        &group.id,
        "user",
        None,
        "Read",
        std::slice::from_ref(&id),
    ))
    .await
    .unwrap();
    state
        .db
        .collection::<Document>(ATTACHMENTS)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"bound_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(31))}},
        )
        .await
        .unwrap();
    assert_eq!(sweep(&state.db).await.unwrap(), 1);
    assert!(matches!(
        read(&state, &owner, &group.id, &id).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    assert!(matches!(
        read(&state, &owner, &thread, &id).await,
        Err(AppError::NotFound(_))
    ));
    let history = Box::pin(crate::handlers::assistant_group::list_messages(
        State(state.clone()),
        test_auth_user(&owner),
        Path(group.id.clone()),
        Query(Default::default()),
    ))
    .await
    .unwrap();
    let attachment = history.0["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["attachments"].as_array().unwrap())
        .find(|attachment| attachment["id"] == id)
        .unwrap();
    assert_eq!(attachment["expired"], true);
    Box::pin(groups::delete(&state.db, &owner, &group.id))
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<Document>(TOMBSTONES)
            .count_documents(doc! {"group_id":group.id})
            .await
            .unwrap(),
        0
    );
}
