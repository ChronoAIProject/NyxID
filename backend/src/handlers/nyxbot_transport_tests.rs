use super::*;
use crate::models::{
    channel_conversation::{COLLECTION_NAME as ROUTES, ChannelConversation},
    org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
};
use crate::services::feature_flag_service::{FlagTarget, set_platform_override};
use std::sync::atomic::AtomicBool;

async fn add_org(state: &AppState) -> String {
    let org = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            OWNER,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    org
}

async fn linked(state: &AppState, org: Option<&str>, platform: &str) -> NyxbotChannel {
    let mut bot = bot_doc(platform, "Transport test");
    bot.insert("user_id", org.unwrap_or(OWNER));
    let id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(bot)
        .await
        .unwrap();
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .insert(id.clone(), "ou_bot".into());
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let (row, _) = Box::pin(connect(state, OWNER, None, &id, &agent))
        .await
        .unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"owner_sender_ids": ["ou_owner"], "private_chats": "everyone"}},
        )
        .await
        .unwrap();
    load_channel(state, OWNER, &row.id).await.unwrap()
}

async fn set_flag(state: &AppState, target: FlagTarget, on: bool) {
    set_platform_override(&state.db, gateway_flag("lark").unwrap(), &target, on, OWNER)
        .await
        .unwrap();
}

async fn set_org_flag(state: &AppState, org: &str, on: bool) {
    feature_flag_service::set_platform_org_override(
        &state.db,
        org,
        gateway_flag("lark").unwrap(),
        on,
        OWNER,
    )
    .await
    .unwrap();
}

async fn route(state: &AppState, row: &NyxbotChannel) -> ChannelConversation {
    state
        .db
        .collection::<ChannelConversation>(ROUTES)
        .find_one(doc! {"_id": row.route_id.as_deref().unwrap(), "is_active": true})
        .await
        .unwrap()
        .unwrap()
}

async fn inactive_key(state: &AppState, owner: &str, id: &str) {
    assert!(
        !key_service::get_api_key(&state.db, owner, id)
            .await
            .is_ok_and(|k| k.is_active)
    );
}

async fn due(state: &AppState, id: &str) {
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": id},
            doc! {"$unset": {"gateway_attempted_at": "", "relay_attempted_at": ""}},
        )
        .await
        .unwrap();
}

async fn native_discord_identity(
    state: &AppState,
    row: &NyxbotChannel,
) -> (
    wiremock::MockServer,
    native_tests::NativeDiscordFixtureGuard,
) {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, path},
    };
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let token = state
        .encryption_keys
        .encrypt(b"test-discord-token")
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform_bot_id":"100","bot_token_encrypted":bson::Binary {
                subtype:bson::spec::BinarySubtype::Generic,bytes:token,
            }}},
        )
        .await
        .unwrap();
    let http = MockServer::start().await;
    crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), http.uri());
    Mock::given(path("/users/@me"))
        .and(header("authorization", "Bot test-discord-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"100","bot":true})))
        .mount(&http)
        .await;
    (
        http,
        native_tests::NativeDiscordFixtureGuard(row.channel_bot_id.clone()),
    )
}

async fn complete_native_drain(state: &AppState, row: &NyxbotChannel) {
    use crate::services::{
        discord_gateway::transport::relay_allowed,
        nyxbot_gateway_release_service::settle_native_drains,
    };
    assert!(!relay_allowed(&state.db, &row.channel_bot_id).await.unwrap());
    settle_native_drains(&state.db).await.unwrap();
    assert!(!relay_allowed(&state.db, &row.channel_bot_id).await.unwrap());
    let changed = state.db.collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .update_many(doc! {"channel_id":&row.id,"native_drain_for":{"$exists":true},"native_not_before":{"$type":"date"}},
            doc! {"$set":{"native_not_before":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}}).await.unwrap();
    assert_eq!(changed.modified_count, 1);
    settle_native_drains(&state.db).await.unwrap();
    assert!(relay_allowed(&state.db, &row.channel_bot_id).await.unwrap());
}

#[tokio::test]
async fn org_gateway_flags_resolve_for_linking_admin_and_live_org_membership() {
    for org_override in [false, true] {
        let (mut state, _, agent) = setup("nyxbot_org_gateway_flags").await;
        let org = add_org(&state).await;
        let before = linked(&state, Some(&org), "lark").await;
        let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
        switch_to_gateway(&state).await.unwrap();
        assert!(calls.lock().await.is_empty());
        let off = load_channel(&state, OWNER, &before.id).await.unwrap();
        assert_eq!(off.transport, "direct");
        assert!(off.gateway_attempted_at.is_none());
        if org_override {
            set_org_flag(&state, &org, true).await;
        } else {
            set_flag(&state, FlagTarget::User(OWNER.into()), true).await;
        }
        switch_to_gateway(&state).await.unwrap();
        let after = load_channel(&state, OWNER, &before.id).await.unwrap();
        assert_eq!(after.transport, "gateway");
        assert_eq!(after.route_id, before.route_id);
        assert_eq!(after.owner_sender_ids, before.owner_sender_ids);
        assert_eq!(after.private_chats, before.private_chats);
        let route = route(&state, &after).await;
        assert_eq!(route.user_id, org);
        assert_eq!(route.agent_api_key_id, after.route_api_key_id);
        assert_eq!(
            key_service::get_api_key(&state.db, &org, &after.route_api_key_id)
                .await
                .unwrap()
                .user_id,
            org
        );
        let provider_key =
            key_service::get_api_key(&state.db, OWNER, after.agent_api_key_id.as_deref().unwrap())
                .await
                .unwrap();
        assert_eq!(provider_key.user_id, OWNER);
        assert!(provider_key.allowed_service_ids.is_empty());
        let created = calls
            .lock()
            .await
            .iter()
            .find(|c| c.0 == "POST")
            .unwrap()
            .2
            .clone();
        assert_eq!(created["profile"]["metadata"]["owner"]["subject"], OWNER);
        inactive_key(&state, &org, &before.route_api_key_id).await;
        gateway.abort();
        agent.abort();
    }
}

#[tokio::test]
async fn org_gateway_refusal_leaves_relay_route_keys_and_settings_unchanged() {
    let (mut state, _, agent) = setup("nyxbot_org_gateway_refusal").await;
    let org = add_org(&state).await;
    let before = linked(&state, Some(&org), "lark").await;
    set_flag(&state, FlagTarget::User(OWNER.into()), true).await;
    let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(true))).await;
    let keys_before = key_service::list_api_keys(&state.db, &org)
        .await
        .unwrap()
        .len();
    let personal_before = key_service::list_api_keys(&state.db, OWNER)
        .await
        .unwrap()
        .len();
    switch_to_gateway(&state).await.unwrap();
    let after = load_channel(&state, OWNER, &before.id).await.unwrap();
    assert_eq!(after.status, "active");
    assert_eq!(after.transport, "direct");
    assert_eq!(after.route_api_key_id, before.route_api_key_id);
    assert_eq!(after.route_id, before.route_id);
    assert_eq!(after.owner_sender_ids, before.owner_sender_ids);
    assert_eq!(after.link_code_hash, before.link_code_hash);
    assert_eq!(after.link_code_expires_at, before.link_code_expires_at);
    assert!(after.pending_agent_api_key_id.is_none() && after.pending_route_api_key_id.is_none());
    assert!(after.gateway_fallback_at.is_some());
    assert_eq!(
        route(&state, &after).await.agent_api_key_id,
        before.route_api_key_id
    );
    assert_eq!(
        key_service::list_api_keys(&state.db, &org)
            .await
            .unwrap()
            .len(),
        keys_before
    );
    assert_eq!(
        key_service::list_api_keys(&state.db, OWNER)
            .await
            .unwrap()
            .len(),
        personal_before
    );
    assert!(calls.lock().await.iter().any(|c| c.0 == "DELETE"));
    calls.lock().await.clear();
    switch_to_gateway(&state).await.unwrap();
    assert!(
        calls.lock().await.is_empty(),
        "refusals retain the daily retry window"
    );
    gateway.abort();
    agent.abort();
}

#[tokio::test]
async fn flag_off_returns_personal_and_org_bots_after_their_active_answer() {
    for is_org in [false, true] {
        let (mut state, _, agent) = setup("nyxbot_gateway_return").await;
        let org = if is_org {
            Some(add_org(&state).await)
        } else {
            None
        };
        let before = linked(&state, org.as_deref(), "lark").await;
        let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
        set_flag(&state, FlagTarget::User(OWNER.into()), true).await;
        switch_to_gateway(&state).await.unwrap();
        let on = load_channel(&state, OWNER, &before.id).await.unwrap();
        assert_eq!(on.transport, "gateway");
        calls.lock().await.clear();
        set_flag(&state, FlagTarget::User(OWNER.into()), false).await;
        let conversations = state.db.collection::<bson::Document>(CONVERSATIONS);
        conversations
            .insert_one(doc! {"_id": "answering", "user_id": OWNER,
            "reply_channel": {"nyxbot_channel_id": &on.id, "partition": "p", "platform": "lark"},
            "active_turn": {"turn_id": "t", "started_at": bson::DateTime::now()}})
            .await
            .unwrap();
        switch_to_gateway(&state).await.unwrap();
        let busy = load_channel(&state, OWNER, &on.id).await.unwrap();
        assert_eq!(busy.transport, "gateway");
        assert_eq!(busy.route_api_key_id, on.route_api_key_id);
        assert!(busy.pending_route_api_key_id.is_none());
        assert!(calls.lock().await.is_empty());
        conversations
            .delete_one(doc! {"_id": "answering"})
            .await
            .unwrap();
        // Retry remains bounded even after the answer completes.
        switch_to_gateway(&state).await.unwrap();
        assert_eq!(
            load_channel(&state, OWNER, &on.id).await.unwrap().transport,
            "gateway"
        );
        due(&state, &on.id).await;
        switch_to_gateway(&state).await.unwrap();
        let off = load_channel(&state, OWNER, &on.id).await.unwrap();
        assert_eq!(off.transport, "direct");
        assert_eq!(off.status, "active");
        assert_eq!(off.route_id, before.route_id);
        assert_eq!(off.owner_sender_ids, before.owner_sender_ids);
        assert_eq!(off.private_chats, before.private_chats);
        assert!(off.agent_api_key_id.is_none() && off.agent_key_ciphertext.is_none());
        assert!(off.gateway_channel_id.is_none() && off.binding_id.is_none());
        let owner = org.as_deref().unwrap_or(OWNER);
        let route = route(&state, &off).await;
        assert_eq!(route.user_id, owner);
        assert_eq!(route.agent_api_key_id, off.route_api_key_id);
        let key = key_service::get_api_key(&state.db, owner, &off.route_api_key_id)
            .await
            .unwrap();
        assert!(
            key.callback_url
                .unwrap()
                .ends_with(&format!("/api/v1/nyxbot/relay/{}", off.id))
        );
        inactive_key(&state, owner, &on.route_api_key_id).await;
        inactive_key(&state, OWNER, on.agent_api_key_id.as_deref().unwrap()).await;
        assert!(calls.lock().await.iter().any(|c| c.0 == "DELETE"));
        gateway.abort();
        agent.abort();
    }
}

#[tokio::test]
async fn org_gateway_provider_endpoints_reject_live_admin_loss_before_any_delivery() {
    for endpoint in 0..6 {
        let (mut state, agent_calls, agent) = setup("nyxbot_org_gateway_revoke").await;
        let org = add_org(&state).await;
        let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
        set_org_flag(&state, &org, true).await;
        // Initial connect must use the same ownership split as migration.
        let row = linked(&state, Some(&org), "lark").await;
        assert_eq!(row.transport, "gateway");
        let raw = state
            .encryption_keys
            .decrypt(row.agent_key_ciphertext.as_ref().unwrap())
            .await
            .unwrap();
        let raw = String::from_utf8(raw.to_vec()).unwrap();
        state
            .db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .update_one(
                doc! {"org_user_id": &org, "member_user_id": OWNER},
                doc! {"$set": {"role": "member"}},
            )
            .await
            .unwrap();
        let binding = row.binding_id.clone().unwrap();
        let response = match endpoint {
            0 => {
                Box::pin(respond(
                    &state,
                    &raw,
                    &event("never deliver", "ou_owner", "revoked"),
                    "revoked",
                ))
                .await
            }
            1 => {
                Box::pin(put_binding(
                    State(state.clone()),
                    Path(binding.clone()),
                    bearer(&raw),
                    Json(binding_body(&raw, OWNER)),
                ))
                .await
            }
            2 => {
                Box::pin(delete_binding(
                    State(state.clone()),
                    Path(binding.clone()),
                    bearer(&raw),
                ))
                .await
            }
            3 => {
                Box::pin(put_conversation(
                    State(state.clone()),
                    Path((binding.clone(), PARTITION.into())),
                    bearer(&raw),
                ))
                .await
            }
            4 => {
                Box::pin(delete_conversation(
                    State(state.clone()),
                    Path((binding.clone(), PARTITION.into())),
                    bearer(&raw),
                ))
                .await
            }
            _ => {
                Box::pin(get_event_context(
                    State(state.clone()),
                    Path((binding, PARTITION.into(), "event".into())),
                    bearer(&raw),
                ))
                .await
            }
        };
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "endpoint {endpoint}"
        );
        assert!(body_text(response).await.contains("org_access_lost"));
        let failed = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.last_error.as_deref(), Some("org_access_lost"));
        assert!(failed.agent_key_ciphertext.is_none());
        assert!(
            state
                .db
                .collection::<ChannelConversation>(ROUTES)
                .find_one(doc! {"_id": row.route_id.as_deref().unwrap(), "is_active": true})
                .await
                .unwrap()
                .is_none()
        );
        inactive_key(&state, &org, &row.route_api_key_id).await;
        inactive_key(&state, OWNER, row.agent_api_key_id.as_deref().unwrap()).await;
        assert!(calls.lock().await.iter().any(|c| c.0 == "DELETE"));
        assert!(agent_calls.lock().await.is_empty());
        assert_eq!(
            state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        gateway.abort();
        agent.abort();
    }
}

#[tokio::test]
async fn telegram_personal_and_org_bots_always_keep_the_gateway() {
    for is_org in [false, true] {
        let (mut state, _, agent) = setup("nyxbot_telegram_gateway_only").await;
        let org = if is_org {
            Some(add_org(&state).await)
        } else {
            None
        };
        let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
        let row = linked(&state, org.as_deref(), "telegram").await;
        assert_eq!(row.transport, "gateway");
        calls.lock().await.clear();
        switch_to_gateway(&state).await.unwrap();
        let after = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(after.transport, "gateway");
        assert_eq!(after.route_api_key_id, row.route_api_key_id);
        assert!(after.relay_attempted_at.is_none());
        assert!(calls.lock().await.is_empty());
        gateway.abort();
        agent.abort();
    }
}

#[tokio::test]
async fn return_swap_rolls_back_if_the_existing_route_changed() {
    let (mut state, _, agent) = setup("nyxbot_gateway_return_cas").await;
    let org = add_org(&state).await;
    let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    set_org_flag(&state, &org, true).await;
    let row = linked(&state, Some(&org), "lark").await;
    assert_eq!(row.transport, "gateway");
    let keys_before = key_service::list_api_keys(&state.db, &org)
        .await
        .unwrap()
        .len();
    // A route admin changed it after the channel was set up. The channel and
    // route must be compared together; neither may be partly switched.
    state
        .db
        .collection::<ChannelConversation>(ROUTES)
        .update_one(
            doc! {"_id": row.route_id.as_deref().unwrap()},
            doc! {"$set": {"agent_api_key_id": "changed-by-admin"}},
        )
        .await
        .unwrap();
    set_org_flag(&state, &org, false).await;
    calls.lock().await.clear();
    switch_to_gateway(&state).await.unwrap();
    let unchanged = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(unchanged.transport, "gateway");
    assert_eq!(unchanged.route_api_key_id, row.route_api_key_id);
    assert_eq!(unchanged.agent_api_key_id, row.agent_api_key_id);
    assert_eq!(unchanged.binding_id, row.binding_id);
    assert!(unchanged.pending_route_api_key_id.is_none());
    assert_eq!(
        route(&state, &row).await.agent_api_key_id,
        "changed-by-admin"
    );
    assert_eq!(
        key_service::list_api_keys(&state.db, &org)
            .await
            .unwrap()
            .len(),
        keys_before
    );
    assert!(
        calls.lock().await.is_empty(),
        "the working gateway must not be released"
    );
    gateway.abort();
    agent.abort();
}

#[tokio::test]
async fn legacy_channel_rows_default_new_transport_metadata() {
    let (state, _, agent) = setup("nyxbot_transport_legacy").await;
    let (row, _) = channel(&state, "direct").await;
    let mut document = bson::to_document(&row).unwrap();
    document.remove("relay_attempted_at");
    let old: NyxbotChannel = bson::from_document(document).unwrap();
    assert!(old.relay_attempted_at.is_none());
    assert_eq!(old.transport, "direct");
    let old_chat: NyxbotThread = bson::from_document(doc! {
        "_id": "legacy", "channel_id": &row.id, "user_id": OWNER,
        "partition": PARTITION, "created_at": bson::DateTime::now(),
        "updated_at": bson::DateTime::now(),
    })
    .unwrap();
    assert!(old_chat.relay_partition.is_none());
    agent.abort();
}

#[tokio::test]
async fn pending_org_return_waits_for_stale_cleanup_before_retrying() {
    let (mut state, _, agent) = setup("nyxbot_gateway_return_recovery").await;
    let org = add_org(&state).await;
    let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    set_org_flag(&state, &org, true).await;
    let row = linked(&state, Some(&org), "lark").await;
    let pending = new_route_key(
        &state,
        &org,
        "Interrupted return",
        "https://example.com/relay",
    )
    .await
    .unwrap();
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    channels.update_one(doc! {"_id": &row.id}, doc! {"$set": {
        "pending_route_api_key_id": &pending.id,
        "relay_attempted_at": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::minutes(MOVE_RECHECK_MINUTES + 1))
    }}).await.unwrap();
    set_org_flag(&state, &org, false).await;
    calls.lock().await.clear();
    // A slow or interrupted preparation keeps its key reference even when
    // the normal retry interval passes. Another replica must not overwrite it.
    switch_to_gateway(&state).await.unwrap();
    let waiting = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(waiting.transport, "gateway");
    assert_eq!(
        waiting.pending_route_api_key_id.as_deref(),
        Some(pending.id.as_str())
    );
    assert_eq!(waiting.route_api_key_id, row.route_api_key_id);
    assert!(
        key_service::get_api_key(&state.db, &org, &pending.id)
            .await
            .is_ok()
    );
    assert!(calls.lock().await.is_empty());
    // After the stale interval, cleanup deletes the org-owned prepared key
    // before a fresh return can replace the working gateway route.
    channels.update_one(doc! {"_id": &row.id}, doc! {"$set": {
        "relay_attempted_at": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::minutes(releases::SETUP_MINUTES + 1))
    }}).await.unwrap();
    switch_to_gateway(&state).await.unwrap();
    let recovered = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(recovered.transport, "direct");
    assert_eq!(recovered.route_id, row.route_id);
    assert!(recovered.pending_route_api_key_id.is_none());
    inactive_key(&state, &org, &pending.id).await;
    inactive_key(&state, &org, &row.route_api_key_id).await;
    assert!(calls.lock().await.iter().any(|c| c.0 == "DELETE"));
    gateway.abort();
    agent.abort();
}

#[tokio::test]
async fn gateway_return_preserves_exact_private_chat_mapping_without_merging_senders() {
    let (mut state, _, agent) = setup("nyxbot_gateway_chat_roundtrip").await;
    let row = linked(&state, None, "lark").await;
    let relay = chats::direct_partition("dm", "guest", None);
    let chat = chats::record_chat(
        &state,
        &row,
        &relay,
        &chats::ChatFacts {
            kind: "private",
            owner: false,
            chat_id: "dm".into(),
            thread_id: None,
            title: Some("Guest".into()),
        },
        None,
    )
    .await
    .unwrap();
    let (_, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    set_flag(&state, FlagTarget::User(OWNER.into()), true).await;
    switch_to_gateway(&state).await.unwrap();
    let on = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(
        chats::sync_gateway_groups(&state, &on, true).await.unwrap(),
        None
    );
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_no_progress
    );
    chats::adopt_relay_chat(&state, &on, PARTITION, "dm", "guest", None)
        .await
        .unwrap();
    set_flag(&state, FlagTarget::User(OWNER.into()), false).await;
    switch_to_gateway(&state).await.unwrap();
    let off = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(!off.gateway_no_progress);
    assert_eq!(
        chats::partition_after_gateway(&state, &off, relay.clone())
            .await
            .unwrap(),
        PARTITION
    );
    let stranger = chats::direct_partition("dm", "stranger", None);
    assert_eq!(
        chats::partition_after_gateway(&state, &off, stranger.clone())
            .await
            .unwrap(),
        stranger
    );
    // A legacy direct row may still contain proof from an old binding.
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"gateway_no_progress": true}},
        )
        .await
        .unwrap();
    // A second move gets a new gateway partition but retains this same chat.
    set_flag(&state, FlagTarget::User(OWNER.into()), true).await;
    switch_to_gateway(&state).await.unwrap();
    let on = load_channel(&state, OWNER, &row.id).await.unwrap();
    // mock_gateway asserts proof is false at the actual pending-key callback;
    // move_to_gateway then earns new proof from its complete policy echo.
    assert!(on.gateway_no_progress);
    let next = format!("conv_{}", "a".repeat(32));
    chats::adopt_relay_chat(&state, &on, &next, "dm", "guest", None)
        .await
        .unwrap();
    let preserved = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id": &chat.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preserved.partition, next);
    assert_eq!(preserved.title, chat.title);
    assert_eq!(preserved.relay_partition.as_deref(), Some(relay.as_str()));
    gateway.abort();
    agent.abort();
}

#[tokio::test]
async fn discord_transport_round_trip_drains_ingress_and_refusal_restores_relay() {
    use crate::models::coordination::CoordinationHolder;
    use crate::services::coordination_service::LeaseStore;
    for refused in [false, true] {
        let (mut state, _, agent) = setup("discord_transport_round_trip").await;
        let mut before = linked(&state, None, "discord").await;
        state
            .db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&before.channel_bot_id},
                doc! {"$set":{"platform_bot_id":"100"}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .update_one(
                doc! {"_id":&before.id},
                doc! {"$set":{"owner_sender_ids":["7"]}},
            )
            .await
            .unwrap();
        before = load_channel(&state, OWNER, &before.id).await.unwrap();
        let (_discord, _discord_guard) = native_discord_identity(&state, &before).await;
        let direct_partition = chats::direct_partition("200", "7", None);
        let dm = chats::record_chat_with_relay(
            &state,
            &before,
            &direct_partition,
            &chats::ChatFacts {
                kind: "private",
                chat_id: "200".into(),
                thread_id: None,
                owner: true,
                title: None,
            },
            None,
            None,
        )
        .await
        .unwrap();
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(
                doc! {"_id":&dm.id},
                doc! {"$set":{"members":"owner","reply_mode":"all"}},
            )
            .await
            .unwrap();
        TEST_BOT_USER_IDS
            .lock()
            .unwrap()
            .insert(before.channel_bot_id.clone(), "100".into());
        let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(refused))).await;
        set_platform_override(
            &state.db,
            gateway_flag("discord").unwrap(),
            &FlagTarget::User(OWNER.into()),
            true,
            OWNER,
        )
        .await
        .unwrap();
        let lease = LeaseStore::acquire(
            &state.db,
            &format!("discord-gateway:{}", before.channel_bot_id),
            &CoordinationHolder {
                instance_id: "test".into(),
                generation_id: Uuid::new_v4().to_string(),
            },
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
        LeaseStore::store_checkpoint(
            &state.db,
            &lease,
            bson::Bson::String("relay-before-migration".into()),
        )
        .await
        .unwrap();
        let moving = tokio::spawn({
            let state = state.clone();
            async move { switch_to_gateway(&state).await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let current = load_channel(&state, OWNER, &before.id).await.unwrap();
                if current.gateway_setup_release_id.is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            calls.lock().await.is_empty(),
            "remote creation waits for the old socket lease"
        );
        LeaseStore::release(&state.db, &lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), moving)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let on = load_channel(&state, OWNER, &before.id).await.unwrap();
        assert_eq!(on.transport, if refused { "direct" } else { "gateway" });
        assert_eq!(on.owner_sender_ids, before.owner_sender_ids);
        assert_eq!(on.route_id, before.route_id);
        assert!(on.pending_agent_api_key_id.is_none());
        let lease_row = state
            .db
            .collection::<bson::Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
            .find_one(doc! {"_id":&lease.name})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            lease_row.contains_key("checkpoint"),
            refused,
            "accepted gateway ownership must retire the old relay resume cursor"
        );
        if !refused {
            let raw = state
                .encryption_keys
                .decrypt(on.agent_key_ciphertext.as_ref().unwrap())
                .await
                .unwrap();
            let raw = Zeroizing::new(String::from_utf8(raw.to_vec()).unwrap());
            let ensured = put_conversation(
                State(state.clone()),
                Path((on.binding_id.clone().unwrap(), PARTITION.into())),
                bearer(&raw),
            )
            .await;
            assert_eq!(ensured.status(), StatusCode::OK);
            let body = native_tests::native_event(&on, "500", "/start");
            let response = body_text(respond(&state, &raw, &body, "discord-dm").await).await;
            assert!(response.contains("Hi, I'm"), "{response}");
            let adopted = state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id":&dm.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(adopted.partition, PARTITION);
            assert_eq!(adopted.members.as_deref(), Some("owner"));
            assert_eq!(adopted.reply_mode.as_deref(), Some("all"));
            set_platform_override(
                &state.db,
                gateway_flag("discord").unwrap(),
                &FlagTarget::User(OWNER.into()),
                false,
                OWNER,
            )
            .await
            .unwrap();
            switch_to_gateway(&state).await.unwrap();
            let off = load_channel(&state, OWNER, &before.id).await.unwrap();
            assert_eq!(off.transport, "direct");
            assert_eq!(off.route_id, before.route_id);
            assert_eq!(off.owner_sender_ids, vec!["7"]);
            assert_eq!(
                route(&state, &off).await.agent_api_key_id,
                off.route_api_key_id
            );
            inactive_key(&state, OWNER, on.agent_api_key_id.as_deref().unwrap()).await;
            assert_eq!(
                chats::partition_after_gateway(&state, &off, direct_partition)
                    .await
                    .unwrap(),
                PARTITION
            );
        }
        assert!(calls.lock().await.iter().any(|c| c.0 == "DELETE"));
        complete_native_drain(&state, &before).await;
        gateway.abort();
        agent.abort();
    }
}

#[tokio::test]
async fn discord_definitive_create_refusal_restores_relay_after_bounded_drain() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let (mut state, _, agent) = setup("discord_create_refusal").await;
    let before = linked(&state, None, "discord").await;
    let (_discord, _discord_guard) = native_discord_identity(&state, &before).await;
    let server = MockServer::start().await;
    state.config.jwt_issuer = "http://localhost:3001".into();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.base_url = server.uri();
    service.requires_user_credential = false;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/channel-capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"relay_authoring":{
            "native_discord_writes_enabled":true,"native_discord_auto_activation_enabled":true,"multi_consumer_writes_enabled":true,
        }})))
        .mount(&server).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"code":"invalid_request"})))
        .mount(&server)
        .await;
    set_platform_override(
        &state.db,
        gateway_flag("discord").unwrap(),
        &FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    switch_to_gateway(&state).await.unwrap();
    let after = load_channel(&state, OWNER, &before.id).await.unwrap();
    assert_eq!(after.transport, "direct");
    assert_eq!(after.route_api_key_id, before.route_api_key_id);
    assert!(after.pending_agent_api_key_id.is_none());
    assert!(after.gateway_setup_release_id.is_none());
    let release = state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .find_one(doc! {"channel_id":&before.id,"native_drain_for":{"$exists":false}})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(release.get_str("state"), Ok("settled"));
    assert_eq!(release.get_str("resolution"), Ok("not_created"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .count(),
        1
    );
    assert!(requests.iter().all(|r| r.method.as_str() != "DELETE"));
    complete_native_drain(&state, &before).await;
    agent.abort();
}
