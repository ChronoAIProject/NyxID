use super::*;
use crate::models::channel_bot::ChannelBot;
use crate::services::feature_flag_service as flags;
use crate::services::nyxbot_gateway_release_service as releases;
use chrono::{Duration as ChronoDuration, Utc};
use mongodb::bson::{self, doc};
use std::sync::{Arc, atomic::AtomicBool};

async fn suspend_security_readiness(state: &AppState) {
    crate::services::security_readiness_service::stop_test_replica(&state.db).await;
    state
        .db
        .collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME)
        .update_many(
            doc! {},
            doc! {"$set": {"observed_at": bson::DateTime::from_chrono(
                Utc::now() - ChronoDuration::seconds(60)
            )}},
        )
        .await
        .unwrap();
}

async fn assert_key_revoked(state: &AppState, key_id: &str) {
    let key = state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": key_id})
        .await
        .unwrap()
        .expect("released key remains as a revoked record");
    assert_eq!(key.get_bool("is_active"), Ok(false));
}

fn answer_text(wire: &str) -> String {
    for line in wire.lines() {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if let Some(text) = value
            .pointer("/item/content/0/text")
            .and_then(serde_json::Value::as_str)
        {
            return text.to_owned();
        }
        if let Some(text) = value
            .pointer("/response/output/0/content/0/text")
            .and_then(serde_json::Value::as_str)
        {
            return text.to_owned();
        }
    }
    wire.to_owned()
}

async fn old_failed_row(
    state: &AppState,
    proof: Option<bson::DateTime>,
) -> (NyxbotChannel, String) {
    let (mut row, _, _, org_id) = org_channel_tests::org_channel(state, false).await;
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.channel_bot_id},
            doc! {"$set": {"platform": "lark", "status": "active", "is_active": true}},
        )
        .await
        .unwrap();
    if let Some(route_id) = row.route_id.take() {
        state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .delete_one(doc! {"_id": route_id})
            .await
            .unwrap();
    }
    state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_many(
            doc! {"_id": &row.route_api_key_id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    row.platform = "lark".into();
    row.transport = "direct".into();
    row.status = "failed".into();
    row.last_error = Some("org_access_lost".into());
    row.org_restore_next_attempt_at = None;
    row.org_restore_last_reason = None;
    row.org_restore_rule_version = 0;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    if let Some(created_at) = proof {
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": Uuid::new_v4().to_string(),
                "event_type": "nyxbot_channel_org_access_lost",
                "event_data": {"channel_agent_id": &row.id},
                "created_at": created_at,
                "user_id": OWNER,
            })
            .await
            .unwrap();
    }
    (row, org_id)
}

/// Build the same residue that the release worker leaves behind.  The row is
/// created as a live organization binding, retired through the production
/// `OrgAccessLost` transaction, and only the proof timestamp is adjusted so
/// the one-time recovery window can be exercised deterministically.
async fn retired_real_org_row(
    state: &AppState,
    platform: &str,
    transport: &str,
) -> (NyxbotChannel, String) {
    let (mut row, _, _, org_id) = org_channel_tests::org_channel(state, false).await;
    if platform != row.platform || transport != row.transport {
        state
            .db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &row.channel_bot_id},
                doc! {"$set": {"platform": platform}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"platform": platform, "transport": transport}},
            )
            .await
            .unwrap();
        row.platform = platform.to_owned();
        row.transport = transport.to_owned();
    }
    let release_ids = releases::retire_connection(
        state,
        &row,
        crate::models::nyxbot_gateway_release::ReleaseReason::OrgAccessLost,
        "failed",
        Some("org_access_lost"),
    )
    .await
    .unwrap();
    let _ = releases::try_inline(state, &release_ids).await;
    let audit = state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {
            "event_type": "nyxbot_channel_org_access_lost",
            "event_data.channel_agent_id": &row.id,
        })
        .await
        .unwrap()
        .expect("release wrote the proof");
    state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .update_one(
            doc! {"_id": audit.get_str("_id").unwrap()},
            doc! {"$set": {"created_at": bson::DateTime::from_millis(releases::ORG_RESTORE_CUTOVER - 1_000)}},
        )
        .await
        .unwrap();
    let retired = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    (retired, org_id)
}

async fn backdate_org_restore_proof(state: &AppState, channel_id: &str) {
    state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .update_one(
            doc! {
                "event_type": "nyxbot_channel_org_access_lost",
                "event_data.channel_agent_id": channel_id,
            },
            doc! {"$set": {"created_at": bson::DateTime::from_millis(releases::ORG_RESTORE_CUTOVER - 1_000)}},
        )
        .await
        .unwrap();
}

async fn assert_restored_admin_roundtrip(
    state: &AppState,
    predecessor: &NyxbotChannel,
    live: &NyxbotChannel,
    platform: &str,
    sender: &str,
) {
    if platform == "lark" && live.transport == "direct" {
        let issued = crate::services::org_channel_service::links::issue(&state.db, OWNER, live)
            .await
            .unwrap();
        let send = |text: String, id: &'static str| async move {
            let body = json!({
                "message_id": id,
                "correlation_id": format!("restore-{id}"),
                "platform": "lark",
                "agent": {"api_key_id": live.route_api_key_id.clone(), "name": "route"},
                "conversation": {"id": live.route_id.clone(), "platform_id": "org-private", "type": "private"},
                "sender": {"platform_id": sender},
                "content": {"type": "text", "text": text},
                "raw_platform_data": {"event": {"sender": {"sender_type": "user"}}}
            });
            store_direct_source(state, live, &body).await;
            let bytes = serde_json::to_vec(&body).unwrap();
            let token = crate::crypto::jwt::generate_relay_callback_token(
                &state.jwt_keys,
                &state.config,
                &format!("restore-{id}"),
                &live.route_api_key_id,
                id,
                "lark",
                &super::sha256_hex(&bytes),
            )
            .unwrap();
            let mut headers = HeaderMap::new();
            headers.insert("x-nyxid-callback-token", token.parse().unwrap());
            let (_watch, done) = super::watch_relay_callback(&live.id, id);
            let response = super::relay_callback(
                State(state.clone()),
                Path(live.id.clone()),
                headers,
                Bytes::from(bytes),
            )
            .await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            tokio::time::timeout(std::time::Duration::from_secs(20), done)
                .await
                .unwrap()
                .unwrap();
        };
        send(issued["code"].as_str().unwrap().to_owned(), "verify").await;
        crate::services::org_channel_service::links::confirm(
            &state.db,
            OWNER,
            live,
            issued["id"].as_str().unwrap(),
        )
        .await
        .unwrap();
        send("restoration answer".into(), "question").await;
        state
            .db
            .collection::<bson::Document>(crate::models::org_channel::LINKS)
            .insert_one(doc! {
                "_id": Uuid::new_v4().to_string(),
                "channel_id": &predecessor.id,
                "org_id": live.org_specialist.as_ref().unwrap().org_id.clone(),
                "revision": Uuid::new_v4().to_string(),
                "binding_revision": live.org_specialist.as_ref().unwrap().revision.clone(),
                "sender_hash": crate::services::org_channel_service::sender_identity(predecessor, "restore-member"),
                "user_id": OWNER,
                "active": false,
                "created_at": bson::DateTime::now(),
            })
            .await
            .unwrap();
        let member_prompt =
            super::super::org_channels::member_refusal(state, live, "restore-member", true).await;
        assert_eq!(
            member_prompt,
            "This organization connection was restored. Members must verify their chat account again in NyxID before chatting with this assistant."
        );
        return;
    }
    let restored_key = state
        .encryption_keys
        .decrypt(live.agent_key_ciphertext.as_ref().unwrap())
        .await
        .unwrap();
    let restored_key = String::from_utf8(restored_key.to_vec()).unwrap();
    if let Some(binding_id) = live.binding_id.as_deref() {
        let ensured = super::put_conversation(
            State(state.clone()),
            Path((binding_id.to_owned(), super::PARTITION.to_owned())),
            super::bearer(&restored_key),
        )
        .await;
        assert_eq!(
            ensured.status(),
            StatusCode::OK,
            "{}",
            super::body_text(ensured).await
        );
    }
    let make_event = |text: &str, id: &str| {
        if platform == "discord" {
            super::native_tests::native_event(live, id, text)
        } else {
            let mut body = super::event(text, sender, id);
            body["event_context"]["activity"]["source"]["platform"] = json!(platform);
            body["event_context"]["activity"]
                .as_object_mut()
                .unwrap()
                .remove("telegram_input");
            body["event_context"]["activity"]["actor"]["id"] = json!(sender);
            body
        }
    };
    let first = super::respond(
        state,
        &restored_key,
        &make_event(
            "restoration check",
            if platform == "discord" {
                "123456789012345"
            } else {
                "restore-admin-before"
            },
        ),
        "restore-admin-before",
    )
    .await;
    let first_text = answer_text(&super::body_text(first).await);
    assert_eq!(
        first_text,
        "Verify your organization membership in NyxID to chat with this assistant."
    );
    let issued = crate::services::org_channel_service::links::issue(&state.db, OWNER, live)
        .await
        .unwrap();
    let challenge_id = issued["id"].as_str().unwrap();
    let code = issued["code"].as_str().unwrap();
    assert!(
        crate::services::org_channel_service::links::receive(
            &state.db,
            live,
            sender,
            code,
            Some("Admin"),
        )
        .await
        .unwrap(),
    );
    crate::services::org_channel_service::links::confirm(&state.db, OWNER, live, challenge_id)
        .await
        .unwrap();
    let answered = super::respond(
        state,
        &restored_key,
        &make_event(
            "restoration answer",
            if platform == "discord" {
                "123456789012346"
            } else {
                "restore-admin-after"
            },
        ),
        "restore-admin-after",
    )
    .await;
    let answered_text = answer_text(&super::body_text(answered).await);
    assert_eq!(answered_text, "Here is your answer");

    state
        .db
        .collection::<bson::Document>(crate::models::org_channel::LINKS)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(),
            "channel_id": &predecessor.id,
            "org_id": live.org_specialist.as_ref().unwrap().org_id.clone(),
            "revision": Uuid::new_v4().to_string(),
            "binding_revision": live.org_specialist.as_ref().unwrap().revision.clone(),
            "sender_hash": crate::services::org_channel_service::sender_identity(predecessor, "restore-member"),
            "user_id": OWNER,
            "active": false,
            "created_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let member_prompt =
        super::super::org_channels::member_refusal(state, live, "restore-member", true).await;
    assert_eq!(
        member_prompt,
        "This organization connection was restored. Members must verify their chat account again in NyxID before chatting with this assistant."
    );
}

async fn drop_state(state: AppState, server: tokio::task::JoinHandle<()>) {
    server.abort();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), state.db.drop()).await;
}

#[tokio::test]
async fn org_restore_real_mongo_restores_once_preserves_guests_and_revokes_predecessor() {
    let (state, _, server) = setup("org_restore_real_success").await;
    let (predecessor, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    let guest_enabled = predecessor
        .org_specialist
        .as_ref()
        .is_some_and(|binding| binding.guests.enabled);
    let old_key = predecessor.route_api_key_id.clone();
    // Legacy person rows predate `user_type`; the restoration authority query
    // must treat their absence as a person account.
    state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": OWNER}, doc! {"$unset": {"user_type": ""}})
        .await
        .unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let live: Vec<NyxbotChannel> = state
        .db
        .collection(CHANNELS)
        .find(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(
        live[0].org_specialist.as_ref().unwrap().guests.enabled,
        guest_enabled
    );
    assert_ne!(live[0].id, predecessor.id);
    assert_eq!(
        live[0]
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .continuity_channel_id
            .as_deref(),
        Some(predecessor.id.as_str())
    );
    let mut carried = live[0].org_specialist.clone().unwrap();
    crate::services::org_channel_service::defaults::sweep(&state.db)
        .await
        .unwrap();
    let after = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &live[0].id})
        .await
        .unwrap()
        .unwrap();
    let after_binding = after.org_specialist.as_ref().unwrap();
    assert!(
        after_binding.guests.enabled,
        "untouched restored Off receives the default"
    );
    assert_ne!(after_binding.guests.revision, carried.guests.revision);
    carried.guests.enabled = true;
    carried
        .guests
        .revision
        .clone_from(&after_binding.guests.revision);
    assert_eq!(
        bson::to_bson(after_binding).unwrap(),
        bson::to_bson(&carried).unwrap()
    );
    assert_eq!(after.private_chats.as_deref(), Some("everyone"));

    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! {"_id": old_key, "is_active": true})
            .await
            .unwrap(),
        None
    );
    let count = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"})
        .await
        .unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(
                doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"}
            )
            .await
            .unwrap(),
        count
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_gateway_telegram_rebuild_uses_creator_bearer_and_new_route() {
    let (mut state, model_calls, server) = setup("org_restore_gateway_telegram").await;
    let (predecessor, _) = retired_real_org_row(&state, "telegram", "gateway").await;
    let old_route_key = predecessor.route_api_key_id.clone();
    let (calls, gateway) = super::mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let live = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"})
        .await
        .unwrap()
        .expect("gateway restoration created a live binding");
    assert_eq!(live.transport, "gateway");
    assert_ne!(live.route_api_key_id, old_route_key);
    let recorded_calls = calls.lock().await;
    let creator_subjects: Vec<_> = recorded_calls
        .iter()
        .filter(|(method, path, _)| method == "POST" && path == "/channels")
        .map(|(_, _, body)| body["_test_creator_subject"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(creator_subjects, vec![OWNER]);
    drop(recorded_calls);
    let nonrouting_puts = calls
        .lock()
        .await
        .iter()
        .filter(|(method, path, _)| method == "PUT" && !path.ends_with("/routing"))
        .count();
    assert_eq!(nonrouting_puts, 2);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("channel_connect_links")
            .count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("nyxbot_watches")
            .count_documents(doc! {"channel_id": &live.id, "status": "pending"})
            .await
            .unwrap(),
        0
    );
    let restored_key = state
        .encryption_keys
        .decrypt(live.agent_key_ciphertext.as_ref().unwrap())
        .await
        .unwrap();
    let restored_key = String::from_utf8(restored_key.to_vec()).unwrap();
    let binding_id = live
        .binding_id
        .as_deref()
        .expect("gateway rebuild stores its provider binding");
    let ensured = super::put_conversation(
        State(state.clone()),
        Path((binding_id.to_owned(), super::PARTITION.to_owned())),
        super::bearer(&restored_key),
    )
    .await;
    assert_eq!(
        ensured.status(),
        StatusCode::OK,
        "{}",
        super::body_text(ensured).await
    );
    let first = super::respond(
        &state,
        &restored_key,
        &super::event("hello", "admin-chat", "restore-admin-1"),
        "restore-admin-1",
    )
    .await;
    let first_text = answer_text(&super::body_text(first).await);
    assert_eq!(
        first_text,
        "Verify your organization membership in NyxID to chat with this assistant."
    );
    let issued = crate::services::org_channel_service::links::issue(&state.db, OWNER, &live)
        .await
        .unwrap();
    let challenge_id = issued["id"].as_str().unwrap();
    let code = issued["code"].as_str().unwrap();
    assert!(
        crate::services::org_channel_service::links::receive(
            &state.db,
            &live,
            "admin-chat",
            code,
            Some("Admin")
        )
        .await
        .unwrap(),
    );
    crate::services::org_channel_service::links::confirm(&state.db, OWNER, &live, challenge_id)
        .await
        .unwrap();
    let answered = super::respond(
        &state,
        &restored_key,
        &super::event("hello again", "admin-chat", "restore-admin-2"),
        "restore-admin-2",
    )
    .await;
    let answered_text = answer_text(&super::body_text(answered).await);
    assert_eq!(answered_text, "Here is your answer");
    let model_calls = model_calls.lock().await;
    assert_eq!(model_calls.len(), 1);
    assert_eq!(
        model_calls[0]["_test_channel_context"]["channel_id"],
        live.id
    );
    assert_eq!(model_calls[0]["_test_channel_context"]["guest"], false);
    assert_eq!(
        model_calls[0]["_test_channel_context"]["sender_kind"],
        "human"
    );
    assert_eq!(
        model_calls[0]["_test_channel_context"]["org_origin"]["org_execution"]["org_id"],
        json!(live.org_specialist.as_ref().unwrap().org_id)
    );
    assert_eq!(
        model_calls[0]["_test_channel_context"]["org_origin"]["org_execution"]["actor_id"],
        json!(OWNER)
    );
    state
        .db
        .collection::<bson::Document>(crate::models::org_channel::LINKS)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(),
            "channel_id": &predecessor.id,
            "org_id": live.org_specialist.as_ref().unwrap().org_id.clone(),
            "revision": Uuid::new_v4().to_string(),
            "binding_revision": live.org_specialist.as_ref().unwrap().revision.clone(),
            "sender_hash": crate::services::org_channel_service::sender_identity(&predecessor, "member-chat"),
            "user_id": OWNER,
            "active": false,
            "created_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let member_prompt =
        super::super::org_channels::member_refusal(&state, &live, "member-chat", true).await;
    assert_eq!(
        member_prompt,
        "This organization connection was restored. Members must verify their chat account again in NyxID before chatting with this assistant."
    );
    let unknown_prompt =
        super::super::org_channels::member_refusal(&state, &live, "never-linked", true).await;
    assert_eq!(
        unknown_prompt,
        "Verify your organization membership in NyxID to chat with this assistant."
    );
    gateway.abort();
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_lark_gateway_and_relay_use_the_linking_person_flag() {
    for (name, transport, enabled) in [
        ("org_restore_lark_gateway", "gateway", true),
        ("org_restore_lark_relay", "direct", false),
    ] {
        let (mut state, _, server) = setup(name).await;
        let (predecessor, _) = retired_real_org_row(&state, "lark", transport).await;
        crate::services::feature_flag_service::set_platform_override(
            &state.db,
            "nyxbot:gateway-lark",
            &crate::services::feature_flag_service::FlagTarget::User(OWNER.to_owned()),
            enabled,
            OWNER,
        )
        .await
        .unwrap();
        let gateway = if enabled {
            Some(
                super::mock_gateway(&mut state, Arc::new(AtomicBool::new(false)))
                    .await
                    .1,
            )
        } else {
            None
        };
        releases::restore_org_access_step(state.clone())
            .await
            .unwrap();
        let live = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"})
            .await
            .unwrap()
            .expect("lark restoration is live");
        assert_eq!(live.transport, transport);
        assert_restored_admin_roundtrip(&state, &predecessor, &live, "lark", "lark-admin").await;
        if let Some(gateway) = gateway {
            gateway.abort();
        }
        drop_state(state, server).await;
    }
}

#[tokio::test]
async fn org_restore_discord_native_drain_blocks_uncertain_and_ninety_seconds_then_restores() {
    let (mut state, _, server) = setup("org_restore_discord_native_drain").await;
    let (predecessor, _, _, _) = org_channel_tests::org_channel(&state, true).await;
    crate::services::feature_flag_service::set_platform_override(
        &state.db,
        "nyxbot:gateway-discord",
        &crate::services::feature_flag_service::FlagTarget::User(OWNER.to_owned()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    let (_discord, _discord_guard) =
        super::transport_tests::native_discord_identity(&state, &predecessor).await;
    let (calls, gateway) = super::mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    let release_ids = releases::retire_connection(
        &state,
        &predecessor,
        crate::models::nyxbot_gateway_release::ReleaseReason::OrgAccessLost,
        "failed",
        Some("org_access_lost"),
    )
    .await
    .unwrap();
    let _ = releases::try_inline(&state, &release_ids).await;
    backdate_org_restore_proof(&state, &predecessor.id).await;
    let mut guard = state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .find_one(doc! {"channel_id": &predecessor.id, "native_drain_for": {"$type": "string"}})
        .await
        .unwrap();
    if guard.is_none() {
        let id = Uuid::new_v4().to_string();
        state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &id,
                "channel_id": &predecessor.id,
                "channel_bot_id": &predecessor.channel_bot_id,
                "state": "review_required",
                "native_drain_for": &id,
                "native_not_before": bson::DateTime::now(),
            })
            .await
            .unwrap();
        guard = state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
            .find_one(doc! {"_id": &id})
            .await
            .unwrap();
    }
    let guard = guard.unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let parked = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &predecessor.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parked.org_restore_attempts, 0);
    state.db.collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .update_one(doc! {"_id": guard.get_str("_id").unwrap()}, doc! {"$set": {"native_not_before": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::seconds(90))}}).await.unwrap();
    // The first guard remains pending until the persisted 90-second floor.
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let waiting = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &predecessor.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(waiting.org_restore_attempts, 0);
    state.db.collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .update_one(doc! {"_id": guard.get_str("_id").unwrap()}, doc! {"$set": {"native_not_before": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(1))}}).await.unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .update_one(
            doc! {"_id": guard.get_str("_id").unwrap()},
            doc! {"$set": {"state":"settled", "resolution":"released"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .update_many(
            doc! {"channel_id": &predecessor.id, "_id": {"$ne": guard.get_str("_id").unwrap()}},
            doc! {"$set": {"state": "settled", "resolution": "released", "settled_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id": &predecessor.id},
            doc! {"$set": {"org_restore_next_attempt_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let live_after_guard = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"})
        .await
        .unwrap();
    assert_eq!(
        live_after_guard.as_ref().map(|row| row.status.as_str()),
        Some("active")
    );
    let post_calls = calls
        .lock()
        .await
        .iter()
        .filter(|(method, _, _)| method == "POST")
        .count();
    assert_eq!(post_calls, 1);
    let live = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status":"active"})
        .await
        .unwrap()
        .unwrap();
    assert_restored_admin_roundtrip(&state, &predecessor, &live, "discord", "7").await;
    gateway.abort();
    drop_state(state, server).await;
}

async fn assert_disqualified(name: &str, mode: &str, expected: &str) {
    let (state, _, server) = setup(name).await;
    let proof =
        (mode != "missing_proof" && mode != "proof_at_cutover" && mode != "proof_after_cutover")
            .then(|| bson::DateTime::from_millis(releases::ORG_RESTORE_CUTOVER - 1_000));
    let (row, org_id) = old_failed_row(&state, proof).await;
    match mode {
        "demoted" => {
            state
                .db
                .collection::<bson::Document>("org_memberships")
                .update_one(
                    doc! {"member_user_id": OWNER},
                    doc! {"$set":{"role":"member"}},
                )
                .await
                .unwrap();
        }
        "removed" => {
            state
                .db
                .collection::<bson::Document>("org_memberships")
                .delete_many(doc! {"member_user_id": OWNER})
                .await
                .unwrap();
        }
        "inactive_admin" => {
            state
                .db
                .collection::<bson::Document>("users")
                .update_one(doc! {"_id": OWNER}, doc! {"$set":{"is_active":false}})
                .await
                .unwrap();
        }
        "inactive_org" => {
            state
                .db
                .collection::<bson::Document>("users")
                .update_one(doc! {"_id": &org_id}, doc! {"$set":{"is_active":false}})
                .await
                .unwrap();
        }
        "bot_moved" => {
            state
                .db
                .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &row.channel_bot_id},
                    doc! {"$set":{"user_id": Uuid::new_v4().to_string()}},
                )
                .await
                .unwrap();
        }
        "bot_inactive" => {
            state
                .db
                .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &row.channel_bot_id},
                    doc! {"$set":{"is_active":false}},
                )
                .await
                .unwrap();
        }
        "bot_suspended" => {
            state
                .db
                .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &row.channel_bot_id},
                    doc! {"$set":{"status":"suspended"}},
                )
                .await
                .unwrap();
        }
        "specialist_destroyed" => {
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &row.agent_id},
                    doc! {"$set":{"destroyed_at": bson::DateTime::now()}},
                )
                .await
                .unwrap();
        }
        "specialist_moved" => {
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &row.agent_id},
                    doc! {"$set":{"user_id": Uuid::new_v4().to_string()}},
                )
                .await
                .unwrap();
        }
        "proof_at_cutover" | "proof_after_cutover" => {
            state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).insert_one(doc! {"_id":Uuid::new_v4().to_string(),"event_type":"nyxbot_channel_org_access_lost","event_data":{"channel_agent_id":&row.id},"created_at":bson::DateTime::from_millis(if mode == "proof_at_cutover" { releases::ORG_RESTORE_CUTOVER } else { releases::ORG_RESTORE_CUTOVER + 1_000 }),"user_id":OWNER}).await.unwrap();
        }
        "pending_binding" | "active_binding" => {
            let mut competing = row.clone();
            competing.id = Uuid::new_v4().to_string();
            competing.status = if mode == "pending_binding" {
                "pending"
            } else {
                "active"
            }
            .into();
            competing.org_restore_claim_id = None;
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .insert_one(&competing)
                .await
                .unwrap();
        }
        "active_route" => {
            state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {"_id":Uuid::new_v4().to_string(),"channel_bot_id":&row.channel_bot_id,"user_id":&org_id,"is_active":true}).await.unwrap();
        }
        _ => {}
    }
    let before_doc = bson::to_document(
        &state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let before_counts = (
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(doc! {"channel_bot_id": &row.channel_bot_id})
            .await
            .unwrap(),
        state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
    );
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let current = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.org_restore_last_reason.as_deref(), Some(expected));
    let mut after_doc = bson::to_document(&current).unwrap();
    let mut before_doc = before_doc;
    for field in [
        "org_restore_next_attempt_at",
        "org_restore_attempts",
        "org_restore_rule_version",
        "org_restore_last_reason",
        "org_restore_claim_id",
        "org_restore_lease_until",
        "org_restored_at",
    ] {
        before_doc.remove(field);
        after_doc.remove(field);
    }
    assert_eq!(
        after_doc, before_doc,
        "disqualification changed business fields"
    );
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(doc! {"channel_bot_id": &row.channel_bot_id})
            .await
            .unwrap(),
        before_counts.0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before_counts.1
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME,)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before_counts.2
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME,)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before_counts.3
    );
    assert_eq!(state.db.collection::<NyxbotChannel>(CHANNELS).count_documents(doc! {"channel_bot_id": &row.channel_bot_id, "_id":{"$ne": &row.id}, "org_restore_last_reason":{"$ne":"restored"}}).await.unwrap(), if mode == "pending_binding" || mode == "active_binding" { 1 } else { 0 });
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_admin_demoted_disqualifies() {
    assert_disqualified("restore_demoted", "demoted", "live_authority_changed").await;
}
#[tokio::test]
async fn org_restore_admin_removed_disqualifies() {
    assert_disqualified("restore_removed", "removed", "live_authority_changed").await;
}
#[tokio::test]
async fn org_restore_admin_account_inactive_disqualifies() {
    assert_disqualified(
        "restore_inactive_admin",
        "inactive_admin",
        "live_authority_changed",
    )
    .await;
}
#[tokio::test]
async fn org_restore_org_inactive_disqualifies() {
    assert_disqualified(
        "restore_inactive_org",
        "inactive_org",
        "live_authority_changed",
    )
    .await;
}
#[tokio::test]
async fn org_restore_bot_moved_disqualifies() {
    assert_disqualified("restore_bot_moved", "bot_moved", "live_authority_changed").await;
}
#[tokio::test]
async fn org_restore_bot_inactive_disqualifies() {
    assert_disqualified(
        "restore_bot_inactive",
        "bot_inactive",
        "live_authority_changed",
    )
    .await;
}
#[tokio::test]
async fn org_restore_bot_suspended_disqualifies() {
    assert_disqualified(
        "restore_bot_suspended",
        "bot_suspended",
        "live_authority_changed",
    )
    .await;
}
#[tokio::test]
async fn org_restore_specialist_destroyed_disqualifies() {
    assert_disqualified(
        "restore_specialist_destroyed",
        "specialist_destroyed",
        "live_authority_changed",
    )
    .await;
}
#[tokio::test]
async fn org_restore_specialist_moved_disqualifies() {
    assert_disqualified(
        "restore_specialist_moved",
        "specialist_moved",
        "live_authority_changed",
    )
    .await;
}
#[tokio::test]
async fn org_restore_pending_binding_disqualifies() {
    assert_disqualified(
        "restore_pending_binding",
        "pending_binding",
        "live_binding_or_route",
    )
    .await;
}
#[tokio::test]
async fn org_restore_active_binding_disqualifies() {
    assert_disqualified(
        "restore_active_binding",
        "active_binding",
        "live_binding_or_route",
    )
    .await;
}
#[tokio::test]
async fn org_restore_active_route_disqualifies() {
    assert_disqualified(
        "restore_active_route",
        "active_route",
        "live_binding_or_route",
    )
    .await;
}
#[tokio::test]
async fn org_restore_missing_proof_disqualifies() {
    assert_disqualified(
        "restore_missing_proof",
        "missing_proof",
        "audit_proof_missing",
    )
    .await;
}
#[tokio::test]
async fn org_restore_proof_at_cutover_disqualifies() {
    assert_disqualified(
        "restore_proof_at_cutover",
        "proof_at_cutover",
        "audit_proof_after_cutover",
    )
    .await;
}
#[tokio::test]
async fn org_restore_proof_after_cutover_disqualifies() {
    assert_disqualified(
        "restore_proof_after_cutover",
        "proof_after_cutover",
        "audit_proof_after_cutover",
    )
    .await;
}

#[tokio::test]
async fn org_restore_latest_proof_after_cutover_overrides_older_proof() {
    let (state, _, server) = setup("restore_latest_proof").await;
    let (row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).insert_one(doc! {"_id":Uuid::new_v4().to_string(),"event_type":"nyxbot_channel_org_access_lost","event_data":{"channel_agent_id":&row.id},"created_at":bson::DateTime::from_millis(releases::ORG_RESTORE_CUTOVER + 1_000),"user_id":OWNER}).await.unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap()
            .org_restore_last_reason
            .as_deref(),
        Some("audit_proof_after_cutover")
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_unavailable_defers_without_attempt_or_failure_audit() {
    let (state, _, server) = setup("restore_unavailable").await;
    let (row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    let (_, claim) = releases::claim_org_restore_candidate(&state.db)
        .await
        .unwrap()
        .unwrap();
    releases::defer_org_restore_unavailable(&state.db, &row, &claim, "readiness_unavailable")
        .await
        .unwrap();
    let parked = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parked.org_restore_attempts, 0);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"nyxbot_channel_org_access_restoration"})
            .await
            .unwrap(),
        0
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_failure_backoff_and_audit_are_reason_transition_based() {
    let (mut state, _, server) = setup("restore_backoff").await;
    let (row, _) = retired_real_org_row(&state, "telegram", "gateway").await;
    let refusal = Arc::new(AtomicBool::new(true));
    let (_calls, gateway) = super::mock_gateway(&mut state, refusal.clone()).await;
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let first = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.org_restore_attempts, 1);
    assert!(first.org_restore_next_attempt_at.unwrap() > Utc::now());
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set":{"org_restore_next_attempt_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let second = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.org_restore_attempts, 2);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {
                "event_type":"nyxbot_channel_org_access_restoration",
                "event_data.outcome":"failed"
            })
            .await
            .unwrap(),
        1
    );
    gateway.abort();
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_rebuild_failures_leave_no_half_built_state_and_recover() {
    for (name, stage) in [
        ("org_restore_gateway_refusal", 0_u8),
        ("org_restore_route_failure", 1_u8),
        ("org_restore_activation_failure", 2_u8),
    ] {
        let (mut state, _, server) = setup(name).await;
        let (predecessor, _) = retired_real_org_row(&state, "telegram", "gateway").await;
        let before_doc = bson::to_document(&predecessor).unwrap();
        let old_key = predecessor.route_api_key_id.clone();
        let key_owners = [
            predecessor
                .bot_owner_id
                .as_deref()
                .unwrap_or(&predecessor.user_id)
                .to_owned(),
            predecessor.user_id.clone(),
        ];
        let key_db = state.db.clone();
        let key_counts_before = futures::future::join_all(key_owners.iter().map(|owner| {
            let db = key_db.clone();
            let owner = owner.clone();
            async move {
                db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                    .count_documents(doc! {"user_id": &owner, "is_active": true})
                    .await
                    .unwrap()
            }
        }))
        .await;
        let refusal = Arc::new(AtomicBool::new(stage == 0));
        let (calls, gateway) = super::mock_gateway(&mut state, refusal.clone()).await;
        RESTORE_CONNECT_FAIL_STAGE
            .scope(stage, releases::restore_org_access_step(state.clone()))
            .await
            .unwrap();
        let row = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id": &predecessor.id})
            .await
            .unwrap()
            .unwrap();
        let mut before_business = before_doc.clone();
        let mut after_business = bson::to_document(&row).unwrap();
        for field in [
            "org_restore_next_attempt_at",
            "org_restore_attempts",
            "org_restore_rule_version",
            "org_restore_last_reason",
            "org_restore_claim_id",
            "org_restore_lease_until",
            "org_restored_at",
            "updated_at",
        ] {
            before_business.remove(field);
            after_business.remove(field);
        }
        assert_eq!(
            after_business, before_business,
            "predecessor business state changed"
        );
        assert_eq!(row.org_restore_attempts, 1);
        assert!(row.org_restore_next_attempt_at.unwrap() > Utc::now());
        assert_eq!(state.db.collection::<NyxbotChannel>(CHANNELS).count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id, "org_restored_at": {"$ne": null}}).await.unwrap(), 0);
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .count_documents(
                    doc! {"channel_bot_id": &predecessor.channel_bot_id, "is_active": true}
                )
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .count_documents(doc! {
                    "channel_bot_id": &predecessor.channel_bot_id,
                    "org_restored_at": {"$ne": null},
                    "status": "active",
                    "gateway_channel_id": {"$type": "string"}
                })
                .await
                .unwrap(),
            0
        );
        let key_db = state.db.clone();
        let key_counts_after = futures::future::join_all(key_owners.iter().map(|owner| {
            let db = key_db.clone();
            let owner = owner.clone();
            async move {
                db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                    .count_documents(doc! {"user_id": &owner, "is_active": true})
                    .await
                    .unwrap()
            }
        }))
        .await;
        assert_eq!(
            key_counts_after, key_counts_before,
            "all provisional keys were cleaned"
        );
        let calls = calls.lock().await;
        match stage {
            0 => assert_eq!(
                calls
                    .iter()
                    .filter(|(method, _, _)| method == "DELETE")
                    .count(),
                1
            ),
            1 => assert_eq!(
                calls
                    .iter()
                    .filter(|(method, _, _)| method == "POST")
                    .count(),
                0
            ),
            2 => assert_eq!(
                calls
                    .iter()
                    .filter(|(method, _, _)| method == "POST")
                    .count(),
                1
            ),
            _ => unreachable!(),
        }
        drop(calls);
        assert_key_revoked(&state, &old_key).await;
        assert_eq!(state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc! {"event_type":"nyxbot_channel_org_access_restoration", "event_data.outcome":"failed"}).await.unwrap(), 1);
        refusal.store(false, std::sync::atomic::Ordering::SeqCst);
        // The existing release worker settles a refused gateway channel before
        // the restoration worker may claim the next retry.
        releases::sweep(&state).await.unwrap();
        state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id": &predecessor.id},
                doc! {"$set": {"org_restore_next_attempt_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
        releases::restore_org_access_step(state.clone())
            .await
            .unwrap();
        let recovery_rows: Vec<bson::Document> = state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find(doc! {"channel_bot_id": &predecessor.channel_bot_id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            recovery_rows
                .iter()
                .any(|row| row.get_str("status") == Ok("active")),
            "recovery did not restore stage {stage} ({name}): {recovery_rows:?}"
        );
        gateway.abort();
        drop_state(state, server).await;
    }
}

#[tokio::test]
async fn org_restore_lease_expiry_during_slow_rebuild_still_leaves_one_binding() {
    let (mut state, _, server) = setup("org_restore_lease_expiry").await;
    let (predecessor, _) = retired_real_org_row(&state, "telegram", "gateway").await;
    let behavior = Arc::new(super::routing_tests::RoutingBehavior::default());
    behavior
        .pause_create
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (_calls, gateway) = super::mock_gateway_with_routing(
        &mut state,
        Arc::new(AtomicBool::new(false)),
        behavior.clone(),
    )
    .await;
    let first_state = state.clone();
    let first = tokio::spawn(async move { releases::restore_org_access_step(first_state).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        behavior.arrived.notified(),
    )
    .await
    .unwrap();
    // The first rebuild has a provisional row and is waiting on a real
    // gateway create request when its predecessor lease expires.
    state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id": &predecessor.id}, doc! {"$set": {"org_restore_lease_until": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(1))}}).await.unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    behavior
        .pause_create
        .store(false, std::sync::atomic::Ordering::SeqCst);
    behavior.resume.notify_one();
    first.await.unwrap().unwrap();
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(
                doc! {"channel_bot_id": &predecessor.channel_bot_id, "status":"active"}
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .count_documents(doc! {
                "channel_bot_id": &predecessor.channel_bot_id,
                "is_active": true
            })
            .await
            .unwrap(),
        1
    );
    assert_eq!(state.db.collection::<NyxbotChannel>(CHANNELS).count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id, "org_restored_at": {"$ne": null}, "status":"pending"}).await.unwrap(), 0);
    let completed = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &predecessor.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        completed.org_restore_last_reason.as_deref(),
        Some("restored")
    );
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id,
            "org_restore_claim_id": {"$ne": null}})
            .await
            .unwrap(),
        0
    );
    for _ in 0..2 {
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type": "nyxbot_channel_org_access_restoration",
                "event_data.predecessor_channel_id": &predecessor.id,
                "event_data.outcome": "restored"})
                .await
                .unwrap(),
            1
        );
        releases::restore_org_access_step(state.clone())
            .await
            .unwrap();
    }
    gateway.abort();
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_provisional_cleanup_does_not_retire_simultaneous_manual_connect() {
    let (state, _, server) = setup("org_restore_cleanup_race").await;
    let (predecessor, _) = retired_real_org_row(&state, "lark", "direct").await;
    let claim = "restore-claim";
    let mut provisional = predecessor.clone();
    provisional.id = Uuid::new_v4().to_string();
    provisional.status = "pending".into();
    provisional.org_restored_at = Some(Utc::now());
    provisional.org_restore_claim_id = Some(claim.into());
    let mut manual = predecessor.clone();
    manual.id = Uuid::new_v4().to_string();
    manual.status = "pending".into();
    manual.org_restored_at = None;
    manual.org_restore_claim_id = None;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_many([provisional.clone(), manual.clone()])
        .await
        .unwrap();
    cleanup_org_restore_provisional(&state, &predecessor, claim)
        .await
        .unwrap();
    assert!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id": &manual.id})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id": &provisional.id, "status":"pending"})
            .await
            .unwrap()
            .is_none()
    );
    drop_state(state, server).await;
}

#[test]
fn org_restore_error_codes_use_the_existing_gateway_contract() {
    assert!(is_org_access_lost_error(
        &crate::errors::AppError::Forbidden("org_access_lost".into(),)
    ));
    assert!(!is_org_access_lost_error(
        &crate::errors::AppError::Forbidden("other".into(),)
    ));
    assert_eq!(
        org_restore_error_reason(&crate::errors::AppError::ValidationError(
            "gateway_unavailable".into()
        )),
        "gateway_unavailable"
    );
    assert_eq!(
        org_restore_error_reason(&crate::errors::AppError::ValidationError(
            "new_gateway_code".into()
        )),
        "rebuild_failed"
    );
}

#[tokio::test]
async fn org_restore_real_release_residue_keeps_links_suspended_and_keys_revoked() {
    let (mut state, _, server) = setup("org_restore_release_residue").await;
    let (row, _, _, _) = org_channel_tests::org_channel(&state, false).await;
    let sender_hash = crate::services::org_channel_service::sender_identity(&row, "member-chat");
    state
        .db
        .collection::<bson::Document>(crate::models::org_channel::LINKS)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "channel_id": &row.id,
            "org_id": row.org_specialist.as_ref().unwrap().org_id.clone(),
            "revision": row.org_specialist.as_ref().unwrap().revision.clone(),
            "binding_revision": row.org_specialist.as_ref().unwrap().revision.clone(),
            "sender_hash": sender_hash, "user_id": OWNER, "active": true,
            "created_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": &row.user_id, "channel_id": &row.id,
            "record_scope": "platform_thread", "follow_state": "active",
            "follow_expires_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::hours(1)),
            "created_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    let release_ids = releases::retire_connection(
        &state,
        &row,
        crate::models::nyxbot_gateway_release::ReleaseReason::OrgAccessLost,
        "failed",
        Some("org_access_lost"),
    )
    .await
    .unwrap();
    let _ = releases::try_inline(&state, &release_ids).await;
    backdate_org_restore_proof(&state, &row.id).await;
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::org_channel::LINKS)
            .count_documents(doc! {"channel_id": &row.id, "active": true})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
            .count_documents(doc! {"channel_id": &row.id, "follow_state":"unavailable"})
            .await
            .unwrap(),
        1
    );
    assert_key_revoked(&state, &row.route_api_key_id).await;
    let (_calls, gateway) = super::mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let live = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"channel_bot_id": &row.channel_bot_id, "status":"active"})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(live.route_api_key_id, row.route_api_key_id);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::org_channel::LINKS)
            .count_documents(doc! {"channel_id": &row.id, "active": true})
            .await
            .unwrap(),
        0
    );
    let prompt =
        super::super::org_channels::member_refusal(&state, &live, "member-chat", true).await;
    assert_eq!(
        prompt,
        "This organization connection was restored. Members must verify their chat account again in NyxID before chatting with this assistant."
    );
    gateway.abort();
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_readiness_and_unsettled_release_defer_without_consuming_attempt() {
    let (state, _, server) = setup("restore_readiness_release").await;
    let (row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    suspend_security_readiness(&state).await;
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let unavailable = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unavailable.org_restore_attempts, 0);
    assert_eq!(unavailable.status, "failed");
    assert_eq!(
        unavailable.org_restore_last_reason.as_deref(),
        Some("readiness_unavailable")
    );
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(doc! {"channel_bot_id": &row.channel_bot_id})
            .await
            .unwrap(),
        1
    );
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let release_id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .insert_one(doc! {"_id": &release_id, "channel_id": &row.id,
        "channel_bot_id": &row.channel_bot_id, "state": "due"})
        .await
        .unwrap();
    // Cleanup may already have removed a failed provisional channel document.
    // Its release journal still blocks a new attempt until remote cleanup settles.
    for released_channel in [row.id.clone(), Uuid::new_v4().to_string()] {
        state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &release_id},
                doc! {"$set": {"channel_id": &released_channel}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"org_restore_next_attempt_at": bson::DateTime::now()}},
            )
            .await
            .unwrap();
        releases::restore_org_access_step(state.clone())
            .await
            .unwrap();
        let waiting = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(waiting.org_restore_attempts, 0);
        assert_eq!(
            waiting.org_restore_last_reason.as_deref(),
            Some("gateway_release_pending")
        );
        assert_eq!(
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .count_documents(doc! {"channel_bot_id": &row.channel_bot_id})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type": "nyxbot_channel_org_access_restoration"})
                .await
                .unwrap(),
            0
        );
    }
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_notice_is_one_typed_transition_for_linking_admin_and_org_admins() {
    let (state, _, server) = setup("restore_notice").await;
    let (row, org_id) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    let second = Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &second,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &org_id,
            &second,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let bot_doc = state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": &row.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    let bot: ChannelBot = bson::from_document(bot_doc).unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .insert(bot.id.clone(), sender);
    crate::services::channel_bot_health::notify_restored(&state.db, &bot, OWNER, true);
    let mut recipients = Vec::new();
    for _ in 0..2 {
        recipients.push(receiver.recv().await.unwrap());
    }
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .remove(&bot.id);
    recipients.sort_by(|a, b| a.0.cmp(&b.0));
    let mut expected = vec![OWNER.to_owned(), second];
    expected.sort();
    assert_eq!(
        recipients
            .iter()
            .map(|entry| entry.0.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        recipients
            .iter()
            .map(|entry| entry.1.notice_kind.clone())
            .collect::<Vec<_>>(),
        vec![
            crate::services::notification_service::ChannelBotNoticeKind::Restored,
            crate::services::notification_service::ChannelBotNoticeKind::Restored,
        ]
    );
    let expected_notice = format!(
        "{} is connected again. Members must verify their chat account again in NyxID. Guest access is on as before. Open the channel details for the current status.",
        bot.label
    );
    assert_eq!(
        recipients
            .iter()
            .map(|entry| entry.1.notice_body.clone().unwrap())
            .collect::<Vec<_>>(),
        vec![expected_notice.clone(), expected_notice]
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_channel_bot_health_handlers_report_owner_org_admin_and_restored_states() {
    let ((state, _, server), commands) =
        super::setup_with_command_monitor("org_restore_health_handlers").await;
    let (row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    let bot_doc = state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": &row.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    let bot: ChannelBot = bson::from_document(bot_doc).unwrap();
    let owner_auth = crate::test_utils::test_auth_user(OWNER);
    commands.lock().unwrap().clear();
    let owner_from_handler = crate::handlers::channel_bots::health(
        State(state.clone()),
        owner_auth.clone(),
        Path(row.channel_bot_id.clone()),
    )
    .await
    .unwrap()
    .0;
    let owner_query_count = commands.lock().unwrap().len();
    assert_eq!(owner_query_count, 20, "owner health read budget changed");
    commands.lock().unwrap().clear();
    let (main_owner, main_bot) =
        crate::handlers::channel_bots::resolve_bot_owner_for_read(&state, OWNER, &bot.id)
            .await
            .unwrap();
    let _ = crate::services::org_service::resolve_owner_access(&state.db, OWNER, &main_owner)
        .await
        .unwrap();
    let _ = crate::services::channel_bot_service::gateway_linked(&state.db, &main_bot.id)
        .await
        .unwrap();
    let _ = crate::services::channel_bot_health::discord::load(
        &state.db,
        std::slice::from_ref(&main_bot),
    )
    .await
    .unwrap();
    let owner_main_query_count = commands.lock().unwrap().len();
    assert_eq!(
        owner_main_query_count, 6,
        "origin/main owner read budget changed"
    );
    commands.lock().unwrap().clear();
    assert_eq!(owner_from_handler.nyxbot_status.as_deref(), Some("failed"));
    let org_list = crate::handlers::channel_bots::list_bots(
        State(state.clone()),
        owner_auth.clone(),
        axum::extract::Query(crate::handlers::channel_bots::ChannelBotListQuery {
            org_id: row.bot_owner_id.clone(),
            scope: None,
        }),
    )
    .await
    .unwrap()
    .0;
    let org_query_count = commands.lock().unwrap().len();
    assert_eq!(org_query_count, 3, "organization list read budget changed");
    commands.lock().unwrap().clear();
    assert_eq!(org_list.total, 1);
    assert_eq!(org_list.bots[0].id, row.channel_bot_id);
    let owner = crate::handlers::channel_bots::health_loaded(&state, &bot, false, true)
        .await
        .unwrap();
    let org = crate::handlers::channel_bots::health_loaded(&state, &bot, false, false)
        .await
        .unwrap();
    let admin = crate::handlers::channel_bots::health_loaded(&state, &bot, false, false)
        .await
        .unwrap();
    assert_eq!(owner.nyxbot_status.as_deref(), Some("failed"));
    assert_eq!(owner.nyxbot_restore_state, None);
    assert_eq!(owner.recommended_action, "none");
    assert_eq!(
        owner.recommended_action_text,
        "NyxBot is restoring this organization connection automatically; it normally returns in under a minute. Members must verify their chat account again when it returns."
    );
    assert_eq!(org.nyxbot_restore_state, admin.nyxbot_restore_state);
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let platform_roles = crate::services::role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": OWNER},
            doc! {"$set": {"role_ids": [&platform_roles.admin], "is_admin": true}},
        )
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let admin_list = crate::handlers::channel_bots::admin_list(
        State(state.clone()),
        owner_auth.clone(),
        axum::extract::Query(crate::handlers::channel_bots::AdminChannelBotQuery {
            platform: None,
            status: None,
            limit: Some(10),
            offset: Some(0),
        }),
    )
    .await
    .unwrap()
    .0;
    let admin_query_count = commands.lock().unwrap().len();
    assert_eq!(admin_query_count, 19, "admin list read budget changed");
    assert_eq!(
        admin_list
            .iter()
            .filter(|entry| entry.id == row.channel_bot_id)
            .count(),
        1
    );
    commands.lock().unwrap().clear();
    crate::handlers::admin_helpers::require_admin(&state, &owner_auth)
        .await
        .unwrap();
    let main_bots =
        crate::services::channel_bot_service::list_all_filtered(&state.db, None, None, 10, 0)
            .await
            .unwrap();
    let main_ids: Vec<String> = main_bots
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect();
    let _ = crate::services::channel_bot_service::gateway_linked_ids(&state.db, &main_ids)
        .await
        .unwrap();
    let _ = crate::services::channel_bot_health::discord::load(&state.db, &main_bots)
        .await
        .unwrap();
    let admin_main_query_count = commands.lock().unwrap().len();
    assert_eq!(
        admin_main_query_count, 5,
        "origin/main admin read budget changed"
    );
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"org_restore_last_reason": "readiness_unavailable"}},
        )
        .await
        .unwrap();
    suspend_security_readiness(&state).await;
    let waiting = crate::handlers::channel_bots::health_loaded(&state, &bot, false, true)
        .await
        .unwrap();
    assert_eq!(
        waiting.nyxbot_restore_state.as_deref(),
        Some("readiness_unavailable")
    );
    assert_eq!(
        waiting.recommended_action_text,
        "NyxBot is waiting for required safety checks to become available before restoring this organization connection. NyxID will retry automatically; no administrator rights were lost."
    );
    commands.lock().unwrap().clear();
    let waiting_from_handler = crate::handlers::channel_bots::health(
        State(state.clone()),
        owner_auth.clone(),
        Path(row.channel_bot_id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        waiting_from_handler.nyxbot_restore_state.as_deref(),
        Some("readiness_unavailable")
    );
    assert_eq!(
        waiting_from_handler.recommended_action_text,
        "NyxBot is waiting for required safety checks to become available before restoring this organization connection. NyxID will retry automatically; no administrator rights were lost."
    );
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"org_restore_last_reason": "gateway_unavailable", "org_restore_next_attempt_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::hours(2))}},
        )
        .await
        .unwrap();
    let retrying = crate::handlers::channel_bots::health_loaded(&state, &bot, false, false)
        .await
        .unwrap();
    assert_eq!(
        retrying.nyxbot_restore_state.as_deref(),
        Some("gateway_unavailable")
    );
    assert_eq!(
        retrying.recommended_action_text,
        "NyxBot is retrying the channel connection. The retry schedule can take hours; no administrator rights were lost."
    );
    commands.lock().unwrap().clear();
    let retrying_from_handler = crate::handlers::channel_bots::admin_list(
        State(state.clone()),
        owner_auth.clone(),
        axum::extract::Query(crate::handlers::channel_bots::AdminChannelBotQuery {
            platform: None,
            status: None,
            limit: Some(10),
            offset: Some(0),
        }),
    )
    .await
    .unwrap()
    .0
    .into_iter()
    .find(|entry| entry.id == row.channel_bot_id)
    .unwrap();
    assert_eq!(
        retrying_from_handler.nyxbot_restore_state.as_deref(),
        Some("gateway_unavailable")
    );
    assert_eq!(
        retrying_from_handler.recommended_action_text,
        "NyxBot is retrying the channel connection. The retry schedule can take hours; no administrator rights were lost."
    );
    // A future retry must not promise an under-a-minute return, including
    // when an earlier writer left no stable reason on the row.
    for attempts in [0, 1] {
        state.db.collection::<bson::Document>(CHANNELS).update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"org_restore_attempts": attempts}, "$unset": {"org_restore_last_reason": ""}},
        ).await.unwrap();
        let backing_off = crate::handlers::channel_bots::health(
            State(state.clone()),
            owner_auth.clone(),
            Path(row.channel_bot_id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(backing_off.recommended_action, "none");
        assert_eq!(
            backing_off.recommended_action_text,
            "NyxBot is retrying the channel connection. The retry schedule can take hours; no administrator rights were lost."
        );
        let details = super::super::channel_listing(&state, OWNER, false)
            .await
            .unwrap();
        let entry = details["channel_agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"].as_str() == Some(row.id.as_str()))
            .unwrap();
        assert_eq!(
            entry["last_error"],
            "org_access_restoration_retrying_gateway"
        );
    }
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.channel_bot_id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let genuine_loss = crate::handlers::channel_bots::health(
        State(state.clone()),
        owner_auth.clone(),
        Path(row.channel_bot_id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(genuine_loss.recommended_action, "reconnect");
    assert_eq!(
        genuine_loss.recommended_action_text,
        "This organization connection lost live administrator authority. An organization admin must reconnect it after restoring access."
    );
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.channel_bot_id},
            doc! {"$set": {"is_active": true}},
        )
        .await
        .unwrap();
    state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id": &row.id}, doc! {"$set": {"org_restore_next_attempt_at": bson::DateTime::now()}, "$unset": {"org_restore_last_reason": ""}}).await.unwrap();
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let listed = super::super::list(&state, OWNER).await.unwrap();
    let current = listed
        .iter()
        .find(|entry| entry.channel_bot_id == row.channel_bot_id && entry.status == "active")
        .expect("restored binding is listed");
    assert_eq!(listed.iter().filter(|entry| entry.id == row.id).count(), 0);
    assert_eq!(
        listed
            .iter()
            .filter(|entry| entry.channel_bot_id == row.channel_bot_id && entry.status == "active")
            .count(),
        1
    );
    let agent_details = super::super::channel_listing(&state, OWNER, false)
        .await
        .unwrap();
    let agent_rows = agent_details["channel_agents"].as_array().unwrap();
    assert_eq!(agent_rows.len(), 1);
    assert_eq!(agent_rows[0]["id"].as_str(), Some(current.id.as_str()));
    commands.lock().unwrap().clear();
    let owner_after = crate::handlers::channel_bots::health(
        State(state.clone()),
        owner_auth.clone(),
        Path(row.channel_bot_id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(owner_after.nyxbot_status.as_deref(), Some("active"));
    assert_eq!(owner_after.nyxbot_last_error, None);
    assert_eq!(owner_after.nyxbot_restore_state, None);
    let owner_after_queries = commands.lock().unwrap().len();
    assert_eq!(
        owner_after_queries, 7,
        "restored owner health read budget changed"
    );
    commands.lock().unwrap().clear();
    let org_after = crate::handlers::channel_bots::list_bots(
        State(state.clone()),
        owner_auth.clone(),
        axum::extract::Query(crate::handlers::channel_bots::ChannelBotListQuery {
            org_id: row.bot_owner_id.clone(),
            scope: None,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(org_after.total, 1);
    assert_eq!(org_after.bots[0].id, row.channel_bot_id);
    commands.lock().unwrap().clear();
    let admin_after = crate::handlers::channel_bots::admin_list(
        State(state.clone()),
        owner_auth,
        axum::extract::Query(crate::handlers::channel_bots::AdminChannelBotQuery {
            platform: None,
            status: None,
            limit: Some(10),
            offset: Some(0),
        }),
    )
    .await
    .unwrap()
    .0;
    let admin_after_entry = admin_after
        .iter()
        .find(|entry| entry.id == row.channel_bot_id)
        .expect("restored bot remains in admin list");
    assert_eq!(admin_after_entry.nyxbot_status.as_deref(), Some("active"));
    assert_eq!(admin_after_entry.nyxbot_last_error, None);
    assert_eq!(admin_after_entry.nyxbot_restore_state, None);
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_member_prompt_requires_previous_link_and_stops_for_unknown_sender() {
    let ((state, _, server), commands) =
        super::setup_with_command_monitor("restore_member_prompt").await;
    let (mut row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    row.status = "active".into();
    commands.lock().unwrap().clear();
    let ordinary =
        super::super::org_channels::member_refusal(&state, &row, "member-chat", true).await;
    assert_eq!(
        ordinary,
        "Verify your organization membership in NyxID to chat with this assistant."
    );
    assert_eq!(
        commands.lock().unwrap().len(),
        0,
        "ordinary refusal must add no reads"
    );
    row.org_restored_at = Some(Utc::now());
    let sender_id = "member-chat";
    let hash = crate::services::org_channel_service::sender_identity(&row, sender_id);
    state.db.collection::<bson::Document>(crate::models::org_channel::LINKS).insert_one(doc! {"_id":Uuid::new_v4().to_string(),"channel_id":&row.id,"org_id":row.org_specialist.as_ref().unwrap().org_id.clone(),"revision":"r","binding_revision":row.org_specialist.as_ref().unwrap().revision.clone(),"sender_hash":&hash,"user_id":OWNER,"active":false,"created_at":bson::DateTime::now()}).await.unwrap();
    let restored = super::super::org_channels::member_refusal(&state, &row, sender_id, true).await;
    let unknown =
        super::super::org_channels::member_refusal(&state, &row, "never-linked", true).await;
    assert_eq!(
        restored,
        "This organization connection was restored. Members must verify their chat account again in NyxID before chatting with this assistant."
    );
    assert_eq!(
        unknown,
        "Verify your organization membership in NyxID to chat with this assistant."
    );
    state
        .db
        .collection::<bson::Document>(crate::models::org_channel::LINKS)
        .update_one(
            doc! {"channel_id": &row.id, "sender_hash": &hash},
            doc! {"$set": {"active": true}},
        )
        .await
        .unwrap();
    let verified = super::super::org_channels::member_refusal(&state, &row, sender_id, true).await;
    assert_eq!(
        verified,
        "Verify your organization membership in NyxID to chat with this assistant."
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_personal_listing_keeps_main_current_row_selection() {
    let (state, _, server) = setup("restore_personal_listing").await;
    let (active, _) = super::channel(&state, "direct").await;
    let mut failed = active.clone();
    failed.id = Uuid::new_v4().to_string();
    failed.status = "failed".into();
    failed.last_error = Some("gateway_unavailable".into());
    failed.route_id = None;
    failed.route_api_key_id = Uuid::new_v4().to_string();
    failed.created_at = active.created_at - ChronoDuration::seconds(1);
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_one(&failed)
        .await
        .unwrap();
    let main_rows = crate::services::nyxbot_channel_query_service::list_current(&state.db, OWNER)
        .await
        .unwrap();
    let presented = super::super::list(&state, OWNER).await.unwrap();
    assert_eq!(
        presented.iter().map(|row| &row.id).collect::<Vec<_>>(),
        main_rows.iter().map(|row| &row.id).collect::<Vec<_>>()
    );
    assert_eq!(main_rows.len(), 1);
    assert_eq!(main_rows[0].id, active.id);
    let details = super::super::channel_listing(&state, OWNER, false)
        .await
        .unwrap();
    let entries = details["channel_agents"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], active.id);
    assert_eq!(entries[0]["status"], "active");
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_org_listing_prefers_other_admin_live_row_over_owned_predecessor() {
    let (state, _, server) = setup("restore_other_admin_listing").await;
    let (predecessor, org_id) = old_failed_row(&state, None).await;
    let other_admin = Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &other_admin,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &org_id,
            &other_admin,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let mut active = predecessor.clone();
    active.id = Uuid::new_v4().to_string();
    active.user_id = other_admin;
    active.status = "active".into();
    active.last_error = None;
    active.created_at = Utc::now();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_one(&active)
        .await
        .unwrap();
    let shared = super::super::list(&state, OWNER).await.unwrap();
    assert_eq!(
        shared.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec![predecessor.id.as_str(), active.id.as_str()]
    );
    let details = super::super::channel_listing(&state, OWNER, false)
        .await
        .unwrap();
    let entries = details["channel_agents"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], active.id);
    assert_eq!(entries[0]["status"], "active");
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_transport_choice_covers_telegram_discord_and_lark_gateway_or_relay() {
    let (state, _, server) = setup("restore_transport_choice").await;
    assert!(gateway_enabled(&state, OWNER, "telegram").await.unwrap());
    flags::set_platform_override(
        &state.db,
        "nyxbot:gateway-discord",
        &flags::FlagTarget::Global,
        true,
        OWNER,
    )
    .await
    .unwrap();
    assert!(gateway_enabled(&state, OWNER, "discord").await.unwrap());
    flags::set_platform_override(
        &state.db,
        "nyxbot:gateway-lark",
        &flags::FlagTarget::Global,
        true,
        OWNER,
    )
    .await
    .unwrap();
    assert!(gateway_enabled(&state, OWNER, "lark").await.unwrap());
    flags::set_platform_override(
        &state.db,
        "nyxbot:gateway-lark",
        &flags::FlagTarget::Global,
        false,
        OWNER,
    )
    .await
    .unwrap();
    assert!(!gateway_enabled(&state, OWNER, "lark").await.unwrap());
    drop_state(state, server).await;
}

#[test]
fn org_restore_health_states_have_no_owner_action_and_truthful_wait_text() {
    use crate::services::channel_bot_health::{HealthFacts, codes};
    let facts = |failure_reason| HealthFacts {
        org_guest_billing: None,
        platform: "telegram",
        status: "failed",
        webhook_registered: true,
        failure_reason,
        outbound_reason: None,
        inbound_outcome: None,
        thread_reason: None,
        subscription_registered: None,
        backoff_active: false,
        gateway: true,
        gateway_not_received: None,
        discord_intake: None,
    };
    for (code, expected) in [
        (
            codes::NYXBOT_ORG_RESTORATION_PENDING,
            "NyxBot is restoring this organization connection automatically; it normally returns in under a minute. Members must verify their chat account again when it returns.",
        ),
        (
            codes::NYXBOT_ORG_RESTORATION_WAITING_READINESS,
            "NyxBot is waiting for required safety checks to become available before restoring this organization connection. NyxID will retry automatically; no administrator rights were lost.",
        ),
        (
            codes::NYXBOT_ORG_RESTORATION_RETRYING_GATEWAY,
            "NyxBot is retrying the channel connection. The retry schedule can take hours; no administrator rights were lost.",
        ),
    ] {
        let advice = crate::services::channel_bot_health::recommend(&facts(Some(code)));
        assert_eq!(advice.action, "none");
        assert_eq!(advice.text, expected);
    }
}

#[tokio::test]
async fn org_restore_audit_and_health_queries_use_named_indexes() {
    let (state, _, server) = setup("restore_explain").await;
    releases::ensure_indexes(&state.db).await.unwrap();
    // Handler fixtures run the service index setup but not the process-wide
    // audit index migration. Recreate the already-deployed key pattern here so
    // MongoDB can show the real startup index in the explain plan.
    state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"event_type": 1, "created_at": -1})
                .build(),
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(),
            "event_type": "nyxbot_channel_org_access_lost",
            "event_data": {"channel_agent_id": "missing"},
            "created_at": bson::DateTime::now(),
            "user_id": OWNER,
        })
        .await
        .unwrap();
    let audit_explain = state
        .db
        .run_command(doc! {
            "explain": {"find": crate::models::audit_log::COLLECTION_NAME,
                "filter": {"event_type":"nyxbot_channel_org_access_lost", "event_data.channel_agent_id":"missing"},
                "sort": {"created_at": -1}, "projection": {"created_at": 1},
                "hint": "event_type_1_created_at_-1"},
            "verbosity": "executionStats"
        })
        .await
        .unwrap();
    let planner = audit_explain.get_document("queryPlanner").unwrap();
    let winning = planner.get_document("winningPlan").unwrap();
    let winning_json = serde_json::to_string(winning).unwrap();
    assert!(
        winning_json.contains("event_type_1_created_at_-1"),
        "{winning_json}"
    );
    let health_explain = state
        .db
        .run_command(doc! {
            "explain": {"aggregate": crate::models::nyxbot_channel::COLLECTION_NAME,
                "pipeline": [
                    {"$match":{"channel_bot_id":{"$in":["missing"]}}},
                    {"$project":{"_id":1,"user_id":1,"channel_bot_id":1,"bot_owner_id":1,
                        "created_at":1,"status":1,"last_error":1,"org_specialist":1,
                        "agent_id":1,"route_id":1,"org_restore_last_reason":1,
                        "org_restore_claim_id":1}},
                    {"$sort":{"channel_bot_id":1,"created_at":-1,"_id":-1}},
                    {"$group":{"_id":"$channel_bot_id","row":{"$first":"$$ROOT"}}},
                    {"$replaceWith":"$row"},
                    {"$limit":1}
                ], "cursor": {}, "hint": "nyxbot_health_latest"},
            "verbosity": "executionStats"
        })
        .await
        .unwrap();
    let health_stages = health_explain
        .get_array("stages")
        .expect("aggregate explain must expose stages");
    assert!(
        !health_stages.is_empty(),
        "health explain has no execution stages"
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_old_rule_version_is_reconsidered_once_despite_ten_year_cooldown() {
    let (state, _, server) = setup("restore_rule_version").await;
    let (row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id": &row.id}, doc! {"$set":{"org_restore_rule_version":0,"org_restore_next_attempt_at":bson::DateTime::from_chrono(Utc::now() + ChronoDuration::days(3650))}}).await.unwrap();
    let eligible = state
        .db
        .collection::<bson::Document>(CHANNELS)
        .find_one(doc! {"_id": &row.id, "org_restore_rule_version": {"$lt": releases::ORG_RESTORE_RULE_VERSION}})
        .await
        .unwrap();
    assert!(
        eligible.is_some(),
        "old rule row was not queryable: {eligible:?}"
    );
    assert!(
        releases::claim_org_restore_candidate(&state.db)
            .await
            .unwrap()
            .is_some()
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_completed_predecessor_never_retries_after_rule_bump_or_cooldown() {
    let (state, _, server) = setup("restore_completed_predecessor").await;
    let (predecessor, _) = retired_real_org_row(&state, "lark", "direct").await;
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let live = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"})
        .await
        .unwrap()
        .expect("the original retirement was restored");
    super::super::disconnect(&state, OWNER, &live.id)
        .await
        .unwrap();
    assert_eq!(live.transport, "direct");
    assert_eq!(live.agent_api_key_id, None);
    assert_key_revoked(&state, &live.route_api_key_id).await;
    let keys = state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
    let routes = state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME);
    let key_count = keys.count_documents(doc! {}).await.unwrap();
    let route_count = routes.count_documents(doc! {}).await.unwrap();
    for (version, next_attempt) in [
        (
            releases::ORG_RESTORE_RULE_VERSION - 1,
            Utc::now() + ChronoDuration::days(3650),
        ),
        (
            releases::ORG_RESTORE_RULE_VERSION,
            Utc::now() - ChronoDuration::seconds(1),
        ),
    ] {
        let channels = state.db.collection::<bson::Document>(CHANNELS);
        channels
            .update_one(
                doc! {"_id": &predecessor.id},
                doc! {"$set": {"org_restore_rule_version": version,
                "org_restore_next_attempt_at": bson::DateTime::from_chrono(next_attempt)}},
            )
            .await
            .unwrap();
        let before = channels
            .find_one(doc! {"_id": &predecessor.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.get_str("org_restore_last_reason"), Ok("restored"));
        releases::restore_org_access_step(state.clone())
            .await
            .unwrap();
        assert_eq!(
            channels
                .find_one(doc! {"_id": &predecessor.id})
                .await
                .unwrap(),
            Some(before)
        );
        assert_eq!(
            channels
                .count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id})
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            channels
                .count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id,
                "status": {"$in": ["pending", "active"]}})
                .await
                .unwrap(),
            0
        );
        assert_eq!(keys.count_documents(doc! {}).await.unwrap(), key_count);
        assert_eq!(routes.count_documents(doc! {}).await.unwrap(), route_count);
        assert_eq!(
            routes
                .count_documents(doc! {"channel_bot_id": &predecessor.channel_bot_id,
                "is_active": true})
                .await
                .unwrap(),
            0
        );
        let presented = super::super::channel_listing(&state, OWNER, false)
            .await
            .unwrap();
        assert_eq!(presented["channel_agents"].as_array().unwrap().len(), 0);
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type": "nyxbot_channel_org_access_restoration"})
            .await
            .unwrap(),
        1
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_real_mongo_proof_boundary_and_latest_after_cutover_disqualify() {
    let (state, _, server) = setup("org_restore_proof_boundary").await;
    let (row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(releases::ORG_RESTORE_CUTOVER)),
    )
    .await;
    releases::restore_org_access_step(state.clone())
        .await
        .unwrap();
    let parked = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        parked.org_restore_last_reason.as_deref(),
        Some("audit_proof_after_cutover")
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_real_mongo_racing_claims_are_single_and_expiry_is_reclaimable() {
    let (state, _, server) = setup("org_restore_claim_race").await;
    let (row, _) = old_failed_row(
        &state,
        Some(bson::DateTime::from_millis(
            releases::ORG_RESTORE_CUTOVER - 1_000,
        )),
    )
    .await;
    let (a, b) = tokio::join!(
        releases::claim_org_restore_candidate(&state.db),
        releases::claim_org_restore_candidate(&state.db)
    );
    assert_eq!(
        usize::from(a.unwrap().is_some()) + usize::from(b.unwrap().is_some()),
        1
    );
    state.db.collection::<bson::Document>(CHANNELS).update_one(doc! {"_id": &row.id}, doc! {"$set":{"org_restore_lease_until": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(1))}}).await.unwrap();
    assert!(
        releases::claim_org_restore_candidate(&state.db)
            .await
            .unwrap()
            .is_some()
    );
    drop_state(state, server).await;
}

#[tokio::test]
async fn org_restore_guest_defaults_preserves_on_without_wallet_and_reports_billing() {
    Box::pin(async {
        let (mut state, model_calls, server) = setup("org_restore_guest_defaults_no_wallet").await;
        let (predecessor, org_id) = retired_real_org_row(&state, "telegram", "gateway").await;
        state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id": &predecessor.id},
                doc! {"$set": {
                    "org_specialist.guests.enabled": true,
                    "org_specialist.guests.revision": Uuid::new_v4().to_string(),
                    "private_chats": "everyone",
                }},
            )
            .await
            .unwrap();
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .count_documents(doc! {"owner_id": &org_id})
                .await
                .unwrap(),
            0
        );
        let (_, gateway) = super::mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
        releases::restore_org_access_step(state.clone())
            .await
            .unwrap();
        let live = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"channel_bot_id": &predecessor.channel_bot_id, "status": "active"})
            .await
            .unwrap()
            .expect("automatic restoration succeeds without billing");
        let policy = &live.org_specialist.as_ref().unwrap().guests;
        assert!(policy.enabled);
        assert_eq!(
            policy.continuity_channel_id.as_deref(),
            Some(predecessor.id.as_str())
        );
        crate::services::org_channel_service::defaults::sweep(&state.db)
            .await
            .unwrap();
        let after = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id": &live.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_bson(&after.org_specialist).unwrap(),
            bson::to_bson(&live.org_specialist).unwrap()
        );
        assert_eq!(after.private_chats.as_deref(), Some("everyone"));
        let health = crate::handlers::channel_bots::health(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(live.channel_bot_id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(health.nyxbot_status.as_deref(), Some("active"));
        assert_eq!(health.recommended_action, "enable_billing");
        let key = state
            .encryption_keys
            .decrypt(live.agent_key_ciphertext.as_ref().unwrap())
            .await
            .unwrap();
        let key = String::from_utf8(key.to_vec()).unwrap();
        assert_eq!(
            super::put_conversation(
                State(state.clone()),
                Path((
                    live.binding_id.clone().unwrap(),
                    super::PARTITION.to_owned()
                )),
                super::bearer(&key),
            )
            .await
            .status(),
            StatusCode::OK
        );
        let reply = super::respond(
            &state,
            &key,
            &super::event("Hello without billing", "7", "restored-no-wallet"),
            "restored-no-wallet",
        )
        .await;
        assert!(
            super::body_text(reply)
                .await
                .contains(crate::services::org_channel_service::UNAVAILABLE)
        );
        assert!(model_calls.lock().await.is_empty());
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::org_channel::HOLDS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {"billing_owner_id": &org_id})
                .await
                .unwrap(),
            0
        );
        gateway.abort();
        drop_state(state, server).await;
    })
    .await;
}
