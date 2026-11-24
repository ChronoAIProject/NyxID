//! Managed Meta setup validates its default without changing ordinary routing.
use super::*;
use crate::services::{channel_routing_service as routes, key_service};

async fn assigned(
    state: &AppState,
    platform: &str,
) -> (
    Router,
    String,
    crate::models::assistant_agent::AssistantAgent,
    NyxbotChannel,
) {
    let agent = specialist(state).await;
    let bot = bot_doc(platform, "Routing contract inbox");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .insert_one(bot)
        .await
        .unwrap();
    let (router, token) = app(state).await;
    let (status, result) = action(&router, &token, &bot_id, &agent.id, "assign").await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let row = load_channel(state, OWNER, result["channel_id"].as_str().unwrap())
        .await
        .unwrap();
    (router, token, agent, row)
}

async fn callback(state: &AppState, key: &str, url: &str) {
    key_service::update_api_key_scope_with_scope_authorization(
        &state.db,
        OWNER,
        Some(OWNER),
        key,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(Some(url)),
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn customer_setup_refuses_drifted_defaults_and_repairs_only_its_unclaimed_assignment() {
    for fault in ["key", "default", "callback"] {
        let (state, _calls, _server) = setup("meta_default_drift").await;
        let (router, token, agent, row) = assigned(&state, "instagram").await;
        let other = key(&state, "Explicit conversation callback").await;
        callback(&state, &other.id, "https://callbacks.example.test/custom").await;
        let explicit = routes::create_conversation(
            &state.db,
            OWNER,
            Some(&row.channel_bot_id),
            "instagram",
            "customer-override",
            "private",
            None,
            &other.id,
            false,
            false,
        )
        .await
        .unwrap();
        match fault {
            "key" => {
                routes::update_conversation(
                    &state.db,
                    row.route_id.as_deref().unwrap(),
                    OWNER,
                    Some(&other.id),
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
            }
            "default" => {
                routes::update_conversation(
                    &state.db,
                    row.route_id.as_deref().unwrap(),
                    OWNER,
                    None,
                    Some(false),
                    None,
                    None,
                )
                .await
                .unwrap();
            }
            _ => {
                callback(
                    &state,
                    &row.route_api_key_id,
                    "https://callbacks.example.test/changed",
                )
                .await
            }
        }
        let (status, result) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::CONFLICT, "{fault}: {result}");
        let (status, result) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "assign").await;
        if fault == "key" {
            assert_eq!(status, StatusCode::CONFLICT, "{result}");
            let current = load_channel(&state, OWNER, &row.id).await.unwrap();
            assert_eq!(current.status, "active");
            assert!(
                key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
                    .await
                    .is_ok()
            );
            assert_eq!(
                routes::resolve_agent(&state.db, &row.channel_bot_id, "new-customer", None, OWNER)
                    .await
                    .unwrap()
                    .unwrap()
                    .api_key_id,
                other.id
            );
        } else {
            assert_eq!(status, StatusCode::OK, "{fault}: {result}");
            let current = load_channel(&state, OWNER, result["channel_id"].as_str().unwrap())
                .await
                .unwrap();
            let default =
                routes::resolve_agent(&state.db, &row.channel_bot_id, "new-customer", None, OWNER)
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(default.api_key_id, current.route_api_key_id);
            assert_eq!(
                default.callback_url,
                crate::services::nyxbot_channel_query_service::direct_callback(
                    &state.config.base_url,
                    &current.id
                )
            );
        }
        let custom = routes::resolve_agent(
            &state.db,
            &row.channel_bot_id,
            "customer-override",
            None,
            OWNER,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(custom.conversation.id, explicit.id);
        assert_eq!(custom.api_key_id, other.id);
    }
}

#[tokio::test]
async fn meta_repair_preserves_an_api_restored_route_and_preexisting_competing_default() {
    for competing in [false, true] {
        let (state, _calls, _server) = setup("meta_repair_live_route").await;
        let (router, token, agent, row) = assigned(&state, "messenger").await;
        routes::update_conversation(
            &state.db,
            row.route_id.as_deref().unwrap(),
            OWNER,
            None,
            None,
            Some(false),
            None,
        )
        .await
        .unwrap();
        let other = key(&state, "Owner chosen default").await;
        callback(&state, &other.id, "https://callbacks.example.test/chosen").await;
        let chosen = if competing {
            Some(
                routes::create_conversation(
                    &state.db,
                    OWNER,
                    Some(&row.channel_bot_id),
                    "messenger",
                    "*",
                    "private",
                    None,
                    &other.id,
                    true,
                    false,
                )
                .await
                .unwrap(),
            )
        } else {
            None
        };
        crate::services::durable_wake_tests::resume_after(
            "channel_repair_before_retirement",
            async {
                let (status, result) =
                    action(&router, &token, &row.channel_bot_id, &agent.id, "assign").await;
                assert_eq!(status, StatusCode::CONFLICT, "{result}");
            },
            async {
                if !competing {
                    routes::update_conversation(
                        &state.db,
                        row.route_id.as_deref().unwrap(),
                        OWNER,
                        None,
                        None,
                        Some(true),
                        None,
                    )
                    .await
                    .unwrap();
                }
            },
        )
        .await;
        let current = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(current.status, "active");
        assert_eq!(current.link_code_hash, row.link_code_hash);
        assert!(
            key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
                .await
                .is_ok()
        );
        let target = routes::resolve_agent(
            &state.db,
            &row.channel_bot_id,
            "fresh-customer",
            None,
            OWNER,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            target.conversation.id,
            chosen
                .as_ref()
                .map(|route| route.id.as_str())
                .unwrap_or(row.route_id.as_deref().unwrap())
        );
    }
}

#[tokio::test]
async fn fresh_meta_setup_does_not_replace_a_default_selected_after_preflight() {
    let (state, _calls, _server) = setup("meta_new_default_race").await;
    let agent = specialist(&state).await;
    let bot = bot_doc("instagram", "New inbox");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .insert_one(bot)
        .await
        .unwrap();
    let (router, token) = app(&state).await;
    let other = key(&state, "Owner selected callback").await;
    callback(&state, &other.id, "https://callbacks.example.test/selected").await;
    crate::services::durable_wake_tests::resume_after(
        "meta_default_before_insert",
        async {
            let (status, result) = action(&router, &token, &bot_id, &agent.id, "assign").await;
            assert_eq!(status, StatusCode::CONFLICT, "{result}");
        },
        async {
            routes::create_conversation(
                &state.db,
                OWNER,
                Some(&bot_id),
                "instagram",
                "*",
                "private",
                None,
                &other.id,
                true,
                false,
            )
            .await
            .unwrap();
        },
    )
    .await;
    let selected = routes::resolve_agent(&state.db, &bot_id, "fresh-customer", None, OWNER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.api_key_id, other.id);
    assert!(
        key_service::get_api_key(&state.db, OWNER, &other.id)
            .await
            .is_ok()
    );
    // The ordinary route API retains its explicit replace-default operation.
    let replacement = key(&state, "Explicit replacement").await;
    callback(
        &state,
        &replacement.id,
        "https://callbacks.example.test/replacement",
    )
    .await;
    let normal = routes::create_conversation(
        &state.db,
        OWNER,
        Some(&bot_id),
        "instagram",
        "*",
        "private",
        None,
        &replacement.id,
        true,
        false,
    )
    .await
    .unwrap();
    let current = routes::resolve_agent(&state.db, &bot_id, "next-customer", None, OWNER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.conversation.id, normal.id);
    assert_eq!(current.api_key_id, replacement.id);
}

#[tokio::test]
async fn meta_repair_and_check_agree_on_revoked_or_expired_stored_keys() {
    for fault in ["revoked-agent", "expired-agent", "expired-route"] {
        let (state, _calls, _server) = setup("meta_repair_key_eligibility").await;
        let (router, token, agent, row) = assigned(&state, "instagram").await;
        let stored = key(&state, "Retained direct provider key").await;
        let target = if fault == "expired-route" {
            &row.route_api_key_id
        } else {
            state
                .db
                .collection::<bson::Document>(CHANNELS)
                .update_one(
                    doc! {"_id": &row.id},
                    doc! {"$set": {"agent_api_key_id": &stored.id}},
                )
                .await
                .unwrap();
            &stored.id
        };
        let change = if fault == "revoked-agent" {
            doc! {"is_active": false}
        } else {
            doc! {"expires_at": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(1))}
        };
        state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(doc! {"_id": target}, doc! {"$set": change})
            .await
            .unwrap();
        let (status, check) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::CONFLICT, "{fault}: {check}");
        let (status, repaired) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "assign").await;
        assert_eq!(status, StatusCode::OK, "{fault}: {repaired}");
        assert_ne!(repaired["channel_id"], row.id);
        let (status, healthy) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::OK, "{fault}: {healthy}");
        assert_eq!(healthy["channel_id"], repaired["channel_id"]);
    }
}
