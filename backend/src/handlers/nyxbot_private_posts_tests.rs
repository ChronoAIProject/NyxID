use super::*;
use crate::services::nyxbot_chat_settings_service::{APPLY_LIMIT, AccessSettings};

fn settings(value: bool, apply: bool) -> AccessSettings {
    AccessSettings {
        private_chat_posts: Some(value),
        apply_to_existing: apply,
        ..Default::default()
    }
}
async fn record(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
    kind: &'static str,
    topic: Option<&str>,
) -> NyxbotThread {
    chats::record_chat(
        state,
        row,
        partition,
        &chats::ChatFacts {
            kind,
            chat_id: partition.into(),
            thread_id: topic.map(str::to_owned),
            owner: false,
            title: Some("$literal-title".into()),
        },
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn private_posts_2124_creation_and_placeholders_preserve_explicit_and_legacy_choices() {
    let (state, _, server) = setup("posts2124_creation").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (mut row, agent_key) = channel(&state, "gateway").await;
    for setting in [None, Some(false), Some(true)] {
        row.private_chat_posts = setting;
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        for kind in ["private", "group", "channel"] {
            let id = Uuid::new_v4().to_string();
            let chat = record(&state, &row, &id, kind, None).await;
            assert_eq!(chat.allow_posts, setting == Some(true) && kind == "private");
            assert_eq!(chat.title.as_deref(), Some("$literal-title"));
        }
        assert!(
            !record(
                &state,
                &row,
                &Uuid::new_v4().to_string(),
                "private",
                Some("topic")
            )
            .await
            .allow_posts
        );
        // The conversation fallback and real gateway PUT both reserve placeholders.
        let partition = format!("direct_{}", Uuid::new_v4().simple());
        let (probe, _) = thread_conversation(&state, &row, &partition).await.unwrap();
        assert!(!probe.allow_posts);
        let chat = record(&state, &row, &partition, "private", None).await;
        assert_eq!(chat.allow_posts, setting == Some(true));
        state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"binding_id":"posts-binding"}},
            )
            .await
            .unwrap();
        let partition = format!("conv_{}", Uuid::new_v4().simple());
        assert_eq!(
            put_conversation(
                State(state.clone()),
                Path(("posts-binding".into(), partition.clone())),
                bearer(&agent_key)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let chat = record(&state, &row, &partition, "private", None).await;
        assert_eq!(chat.allow_posts, setting == Some(true));
        let partition = format!("conv_{}", Uuid::new_v4().simple());
        assert_eq!(
            put_conversation(
                State(state.clone()),
                Path(("posts-binding".into(), partition.clone())),
                bearer(&agent_key)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert!(
            !record(&state, &row, &partition, "group", None)
                .await
                .allow_posts
        );
    }
    let chat = record(&state, &row, "explicit-off", "private", None).await;
    chats::update_chat(
        &state,
        OWNER,
        &chat.id,
        &chats::ChatSettings {
            allow_posts: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        !record(&state, &row, "explicit-off", "private", None)
            .await
            .allow_posts
    );
    assert!(matches!(
        chats::validate_post(&state, OWNER, &chat.id, None).await,
        Err(AppError::Forbidden(_))
    ));
    let now = bson::DateTime::now();
    state.db.collection::<bson::Document>(THREADS).insert_one(doc!{"_id":Uuid::new_v4().to_string(),"channel_id":&row.id,"user_id":OWNER,"partition":"legacy","created_at":now,"updated_at":now}).await.unwrap();
    assert!(
        !record(&state, &row, "legacy", "private", None)
            .await
            .allow_posts
    );
    let custom = custom_event_thread(&state, &row, "custom:test", &chat)
        .await
        .unwrap();
    assert!(!custom.allow_posts);
    let mut legacy = bson::to_document(&row).unwrap();
    legacy.remove("private_chat_posts");
    let legacy: NyxbotChannel = bson::from_document(legacy).unwrap();
    assert_eq!(legacy.private_chat_posts, None);
    assert!(!ChannelAgentResponse::from(&legacy).private_chat_posts);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn private_posts_2124_bulk_is_scoped_opt_in_and_atomic_at_the_cap() {
    let (state, _, server) = setup("posts2124_bulk").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (row, _) = channel(&state, "direct").await;
    let (other, _) = channel(&state, "direct").await;
    let private = record(&state, &row, "private", "private", None).await;
    let group = record(&state, &row, "group", "group", None).await;
    let topic = record(&state, &row, "topic", "private", Some("topic")).await;
    let foreign = record(&state, &other, "foreign", "private", None).await;
    let custom = custom_event_thread(&state, &row, "custom:event", &group)
        .await
        .unwrap();
    let child = record(&state, &row, "child", "private", None).await;
    state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"_id":&child.id},
            doc! {"$set":{"record_scope":"platform_thread"}},
        )
        .await
        .unwrap();
    assert_eq!(
        chats::update_access(&state, OWNER, &row.id, &settings(true, false))
            .await
            .unwrap()["applied_count"],
        0
    );
    assert!(
        !state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"_id":&private.id})
            .await
            .unwrap()
            .unwrap()
            .allow_posts
    );
    assert_eq!(
        chats::update_access(&state, OWNER, &row.id, &settings(true, true))
            .await
            .unwrap()["applied_count"],
        1
    );
    assert_eq!(
        chats::update_access(&state, OWNER, &row.id, &settings(false, false))
            .await
            .unwrap()["applied_count"],
        0
    );
    assert!(
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"_id":&private.id})
            .await
            .unwrap()
            .unwrap()
            .allow_posts
    );
    assert_eq!(
        chats::update_access(&state, OWNER, &row.id, &settings(false, true))
            .await
            .unwrap()["applied_count"],
        1
    );
    for id in [&group.id, &topic.id, &foreign.id, &custom.id, &child.id] {
        assert!(
            !state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap()
                .allow_posts
        );
    }
    let now = bson::DateTime::now();
    state.db.collection::<bson::Document>(THREADS).insert_many((0..APPLY_LIMIT).map(|i|doc!{"_id":Uuid::new_v4().to_string(),"channel_id":&row.id,"user_id":OWNER,"partition":format!("bulk{i}"),"kind":"private","created_at":now,"updated_at":now})).await.unwrap();
    assert!(matches!(
        chats::update_access(&state, OWNER, &row.id, &settings(true, true)).await,
        Err(AppError::ValidationError(_))
    ));
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .private_chat_posts,
        Some(false)
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(THREADS)
            .count_documents(doc! {"allow_posts":true})
            .await
            .unwrap(),
        0
    );
    assert!(settings(true, true).validate().is_ok());
    assert!(
        AccessSettings {
            apply_to_existing: true,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn private_posts_2124_rest_requires_human_owner_and_live_org_admin() {
    use crate::models::org_membership::OrgRole;
    let (state, _, server) = setup("posts2124_authority").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (row, _) = channel(&state, "direct").await;
    let request =
        || serde_json::from_value::<LinkRequest>(json!({"private_chat_posts":true})).unwrap();
    let mut nonhuman = crate::test_utils::test_auth_user(OWNER);
    nonhuman.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    assert!(
        link_channel(
            State(state.clone()),
            nonhuman,
            Path(row.id.clone()),
            Json(request())
        )
        .await
        .is_err()
    );
    assert!(
        link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
            Path(row.id.clone()),
            Json(request())
        )
        .await
        .is_err()
    );
    assert!(
        link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(row.id.clone()),
            Json(request())
        )
        .await
        .is_ok()
    );
    let org = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    state
        .db
        .collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &org,
            OWNER,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"bot_owner_id":&org}})
        .await
        .unwrap();
    assert!(matches!(
        link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(row.id.clone()),
            Json(request())
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":OWNER},
            doc! {"$set":{"role":"admin"}},
        )
        .await
        .unwrap();
    assert!(
        link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(row.id.clone()),
            Json(request())
        )
        .await
        .is_ok()
    );
    state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":OWNER},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert!(
        link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(row.id.clone()),
            Json(request())
        )
        .await
        .is_err()
    );
    state.db.drop().await.unwrap();
}
