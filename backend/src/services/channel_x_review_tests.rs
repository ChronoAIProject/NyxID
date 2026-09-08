use super::*;
use futures::TryStreamExt;
use std::time::Duration;

fn event(id: u64) -> serde_json::Value {
    json!({"id": id.to_string(), "sender_id": "20", "dm_conversation_id": "10-20", "text": "private DM"})
}

async fn route_agent(state: &AppState, server: &MockServer, bot: &ChannelBot) {
    let agent = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
        "_id": &agent, "user_id": &bot.user_id, "name": "agent", "key_prefix": "nyxid_ag", "key_hash": "deadbeef".repeat(8),
        "scopes": "read write", "is_active": true, "callback_url": format!("{}/callback", server.uri()), "created_at": bson::DateTime::now(),
    }).await.unwrap();
    state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id, "channel_bot_id": &bot.id, "platform": "x",
        "platform_conversation_id": "10-20", "platform_conversation_type": "private", "agent_api_key_id": agent,
        "default_agent": false, "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
}

#[tokio::test]
async fn reconnect_relays_dms_received_while_failed_once() {
    let (state, adapter, server, owner, key) = fixture().await;
    let bot = insert_bot(&state, &owner, &key).await;
    route_agent(&state, &server, &bot).await;
    channel_credentials::fail_bot(&state.db, &bot, "Reconnect the account")
        .await
        .unwrap();
    let failed = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(failed.status, "failed");
    identity(&server, "10").await;
    Mock::given(method("GET"))
        .and(path("/2/dm_events"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [event(102), event(101), event(100)]})),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(202))
        .expect(2)
        .mount(&server)
        .await;
    let adapters = || {
        vec![Box::new(XAdapter {
            api_base: Some(server.uri()),
        }) as Box<dyn PlatformAdapter>]
    };
    channel_poll_service::sweep_with_adapters(&state, adapters())
        .await
        .unwrap();
    assert!(server.received_requests().await.unwrap().is_empty());
    channel_bot_service::reconnect_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &failed,
        &key,
    )
    .await
    .unwrap();
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(current.poll_cursor, bot.poll_cursor);
    channel_poll_service::sweep_with_adapters(&state, adapters())
        .await
        .unwrap();
    due(&state, &bot).await;
    channel_poll_service::sweep_with_adapters(&state, adapters())
        .await
        .unwrap();
    let callbacks = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/callback")
        .collect::<Vec<_>>();
    assert_eq!(callbacks.len(), 2);
    let messages: Vec<bson::Document> = state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find(doc! {"channel_bot_id": &bot.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert!(
        messages
            .iter()
            .all(|m| m.get_str("callback_status") == Ok("delivered"))
    );
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .poll_cursor
            .as_deref(),
        Some("102")
    );
}

#[tokio::test]
async fn reconnect_initializes_only_missing_cursors() {
    let (state, adapter, server, owner, key) = fixture().await;
    let bot = insert_bot(&state, &owner, &key).await;
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(doc! {"_id": &bot.id}, doc! {"$set": {"poll_cursor": null}})
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    identity(&server, "10").await;
    Mock::given(method("GET"))
        .and(path("/2/dm_events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [event(200)]})))
        .expect(1)
        .mount(&server)
        .await;
    channel_bot_service::reconnect_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &bot,
        &key,
    )
    .await
    .unwrap();
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .poll_cursor
            .as_deref(),
        Some("200")
    );
}

#[tokio::test]
async fn oversized_backlog_advances_with_notice_and_does_not_fail() {
    let (state, adapter, server, owner, key) = fixture().await;
    let bot = insert_bot(&state, &owner, &key).await;
    route_agent(&state, &server, &bot).await;
    for page in 0..10_u64 {
        Mock::given(method("GET")).and(path("/2/dm_events"))
            .and(move |r: &wiremock::Request| {
                let token = r.url.query_pairs().find(|(k, _)| k == "pagination_token").map(|(_, v)| v.to_string());
                token == (page > 0).then(|| page.to_string())
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": (0..100).map(|offset| event(2000 - page * 100 - offset)).collect::<Vec<_>>(),
                "meta": {"next_token": (page + 1).to_string()}
            }))).expect(1).mount(&server).await;
    }
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1000)
        .mount(&server)
        .await;
    channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
        .await
        .unwrap();
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(current.poll_cursor.as_deref(), Some("2000"));
    assert_eq!(current.poll_error_count, 0);
    assert_eq!(current.status, "active");
    assert!(current.error.is_none());
    assert!(
        current
            .last_poll_notice
            .as_deref()
            .unwrap()
            .contains("Older X DMs were skipped")
    );
    let callbacks = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/callback")
        .map(|r| serde_json::from_slice::<serde_json::Value>(&r.body).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(callbacks.len(), 1000);
    // The callback envelope preserves chronological platform message IDs.
    let ids = callbacks
        .iter()
        .map(|body| {
            body["raw_platform_data"]["id"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, (1001..=2000).collect::<Vec<_>>());
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/2/dm_events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [event(2000)]})))
        .expect(1)
        .mount(&server)
        .await;
    due(&state, &bot).await;
    channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
        .await
        .unwrap();
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .last_poll_notice,
        current.last_poll_notice
    );
}

#[tokio::test]
async fn sweep_continues_after_database_and_provider_errors() {
    let (state, _, server, owner, key) = fixture().await;
    let bad = insert_bot(&state, &owner, &key).await;
    let provider_error = insert_bot(&state, &owner, &key).await;
    let healthy = insert_bot(&state, &owner, &key).await;
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(
            doc! {"_id": &provider_error.id},
            doc! {"$set": {"poll_cursor": "invalid"}},
        )
        .await
        .unwrap();
    // A real MongoDB rejection during lease claim must not abort the tick.
    state
        .db
        .run_command(doc! {"collMod": BOTS, "validator": {"$or": [
            {"_id": {"$ne": &bad.id}}, {"poll_lease_until": null}
        ]}, "validationLevel": "strict", "validationAction": "error"})
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/2/dm_events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [event(101)]})))
        .expect(1)
        .mount(&server)
        .await;
    channel_poll_service::sweep_with_adapters(
        &state,
        vec![Box::new(XAdapter {
            api_base: Some(server.uri()),
        })],
    )
    .await
    .unwrap();
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &healthy.id)
            .await
            .unwrap()
            .poll_cursor
            .as_deref(),
        Some("101")
    );
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &provider_error.id)
            .await
            .unwrap()
            .poll_error_count,
        1
    );
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &bad.id)
            .await
            .unwrap()
            .poll_cursor,
        bad.poll_cursor
    );
}

#[tokio::test]
async fn oauth_refresh_sweep_expires_only_stale_channel_pending_rows() {
    let (state, adapter, _, owner, _) = fixture().await;
    let mut ids = Vec::new();
    for _ in 0..4 {
        let start = channel_credentials::start_connection(
            &state.db,
            &state.encryption_keys,
            &state.config.base_url,
            &adapter,
            &owner,
            &owner,
            "Support",
        )
        .await
        .unwrap();
        ids.push(start.connection_id);
    }
    let keys = state.db.collection::<bson::Document>(KEYS);
    let old = bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::hours(2));
    keys.update_many(
        doc! {"_id": {"$in": &ids[..3]}},
        doc! {"$set": {"created_at": old}},
    )
    .await
    .unwrap();
    keys.update_one(doc! {"_id": &ids[1]}, doc! {"$set": {"status": "active"}})
        .await
        .unwrap();
    keys.update_one(
        doc! {"_id": &ids[2]},
        doc! {"$set": {"source": "user_created"}},
    )
    .await
    .unwrap();
    super::super::user_token_service::refresh_expiring_oauth_keys(
        &state.db,
        &state.encryption_keys,
        chrono::Duration::minutes(15),
        None,
    )
    .await
    .unwrap();
    assert!(
        keys.find_one(doc! {"_id": &ids[0]})
            .await
            .unwrap()
            .is_none()
    );
    for id in &ids[1..] {
        assert!(keys.find_one(doc! {"_id": id}).await.unwrap().is_some());
    }
    assert_eq!(keys.count_documents(doc! {}).await.unwrap(), 4);
}

#[tokio::test]
async fn x_byo_handler_returns_the_adapter_managed_only_explanation() {
    let (state, _, server, owner, _) = fixture().await;
    let result = crate::handlers::channel_bots::create_bot(
        State(state),
        test_auth_user(&owner),
        crate::telemetry::TelemetryContext::default(),
        Json(
            serde_json::from_value(
                json!({"platform": "x", "label": "Support", "bot_token": "private-token"}),
            )
            .unwrap(),
        ),
    )
    .await;
    assert!(
        matches!(result, Err(AppError::ValidationError(ref message)) if message == "X accounts are connected through Connect X account; developer credentials are not accepted")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn shared_provider_field_clear_and_delete_audits_name_the_provider() {
    use crate::handlers::admin_platform_credentials as admin;
    let (state, _, _, owner, _) = fixture().await;
    super::super::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let role = super::super::role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap()
        .admin;
    state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": &owner}, doc! {"$addToSet": {"role_ids": role}})
        .await
        .unwrap();
    let auth = test_auth_user(&owner);
    let (_, Json(updated)) = admin::update(
        State(state.clone()),
        auth.clone(),
        Path("x".into()),
        Json(serde_json::from_value(json!({"fields": {"client_secret": null}})).unwrap()),
    )
    .await
    .unwrap();
    assert!(!updated.available);
    let provider = state
        .db
        .collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"slug": "twitter"})
        .await
        .unwrap()
        .unwrap();
    assert!(provider.client_id_encrypted.is_some());
    assert!(provider.client_secret_encrypted.is_none());
    admin::delete(State(state.clone()), auth, Path("x".into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type": "admin_platform_credentials_updated", "event_data.shared_provider_slug": "twitter"}).await.unwrap();
            if count == 2 { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("both shared-provider audits");
}
