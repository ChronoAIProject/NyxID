use super::*;
use crate::models::assistant_agent::AgentGrants;
use crate::services::assistant_team_service::{self as team, GrantChange};
use serde_json::json;

async fn reload(f: &mut Fixture) {
    f.channel = management::load(&f.org.state.db, &f.org.admin, &f.channel.id, true)
        .await
        .unwrap();
}

async fn rebuild(f: &mut Fixture) {
    let (row, _) = Box::pin(crate::handlers::nyxbot::connect(
        &f.org.state,
        &f.org.admin,
        None,
        &f.bot.id,
        &f.agent,
    ))
    .await
    .unwrap();
    assert_ne!(row.id, f.channel.id);
    assert_eq!(row.status, "active");
    f.org
        .state
        .db
        .collection::<Document>("nyxbot_threads")
        .insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"channel_id":&row.id,"user_id":&row.user_id,
            "partition":"private","kind":"private","platform_chat_id":"chat",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        })
        .await
        .unwrap();
    f.channel = row;
}

#[tokio::test]
async fn org_channel_guest_settings_one_switch_defaults_and_fences_edits() {
    let mut f = Fixture::new("org_guest_settings_enable").await;
    f.wallet().await;
    let db = f.org.state.db.clone();
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"private_chats":"owner"}},
        )
        .await
        .unwrap();
    reload(&mut f).await;
    let input: management::GuestSettings =
        serde_json::from_value(json!({"enabled":true,"expected_revision":""})).unwrap();
    for actor in [&f.org.member, &f.org.viewer, &f.org.outsider] {
        assert!(
            management::update(&db, actor, &f.channel, input.clone())
                .await
                .is_err()
        );
    }
    let stale = f.channel.clone();
    management::update(&db, &f.org.admin, &f.channel, input.clone())
        .await
        .unwrap();
    reload(&mut f).await;
    let view = management::view(&db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    assert_eq!(view["daily_credits"], view["default_daily_credits"]);
    assert_eq!(view["guest_enabled"], true);
    assert_eq!(f.channel.private_chats.as_deref(), Some("everyone"));
    assert!(f.guest("new-guest").await.org_execution.unwrap().guest);
    assert!(matches!(
        management::update(&db, &f.org.admin, &f.channel, input.clone()).await,
        Err(AppError::Conflict(_))
    ));
    // Even a legacy client with no revision must not overwrite a concurrently read policy.
    let mut legacy = input;
    legacy.expected_revision = None;
    assert!(matches!(
        management::update(&db, &f.org.admin, &stale, legacy).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        management::view(&db, &f.org.admin, &f.channel)
            .await
            .unwrap()["guest_revision"],
        view["guest_revision"]
    );
    // A later budget-only edit cannot reopen a private audience the admin closed.
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"private_chats":"owner"}},
        )
        .await
        .unwrap();
    reload(&mut f).await;
    management::patch(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(json!({
            "daily_credits":"2","expected_revision":view["guest_revision"]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    reload(&mut f).await;
    assert_eq!(f.channel.private_chats.as_deref(), Some("owner"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_guest_settings_inactive_rows_are_readable_but_not_editable() {
    let mut f = Fixture::new("org_guest_settings_inactive").await;
    let db = f.org.state.db.clone();
    for status in ["pending", "failed"] {
        db.collection::<Document>("nyxbot_channels")
            .update_one(
                doc! {"_id":&f.channel.id},
                doc! {"$set":{"status":status,"error":"org_access_lost"}},
            )
            .await
            .unwrap();
        reload(&mut f).await;
        let member = management::load(&db, &f.org.member, &f.channel.id, false)
            .await
            .unwrap();
        let view = management::view(&db, &f.org.member, &member).await.unwrap();
        assert_eq!(view["channel_status"], status);
        assert_eq!(view["can_manage"], false);
        assert!(matches!(
            management::update(
                &db,
                &f.org.admin,
                &f.channel,
                serde_json::from_value(json!({"enabled":false})).unwrap()
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_guest_settings_rebuild_preserves_policy_and_daily_ledger() {
    let mut f = Fixture::new("org_guest_settings_rebuild").await;
    f.wallet().await;
    let service = f.org.service(&f.org.org, "lookup").await;
    team::set_grants(
        &f.org.state.db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Add(AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    f.guests(vec![service.clone()]).await;
    let db = f.org.state.db.clone();
    let original = f.channel.clone();
    let origin = f.guest("visitor").await;
    let access = validate(&db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    let payer = access.channel().unwrap().guest_payer().unwrap();
    reserve(&db, &payer, "before-rebuild").await.unwrap();
    assert!(
        Box::pin(crate::handlers::nyxbot::connect(
            &f.org.state,
            &f.org.member,
            None,
            &f.bot.id,
            &f.agent,
        ))
        .await
        .is_err()
    );
    // The actual connect path rebuilds a connection whose route disappeared.
    db.collection::<Document>("channel_conversations")
        .update_one(
            doc! {"_id":&f.channel.route_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    rebuild(&mut f).await;
    let saved = &f.channel.org_specialist.as_ref().unwrap().guests;
    assert!(saved.enabled);
    assert_eq!(saved.daily_credits, Some(Credits::from_whole(1)));
    assert_eq!(saved.service_ids, vec![service]);
    assert_ne!(
        saved.revision,
        original.org_specialist.unwrap().guests.revision
    );
    assert_eq!(
        saved.continuity_channel_id.as_deref(),
        Some(original.id.as_str())
    );
    assert_eq!(
        management::view(&db, &f.org.admin, &f.channel)
            .await
            .unwrap()["reserved_today_credits"],
        "0.6"
    );
    let origin = f.guest("visitor").await;
    let access = validate(&db, &origin, &f.org.org, &f.agent.id)
        .await
        .unwrap();
    let payer = access.channel().unwrap().guest_payer().unwrap();
    assert!(reserve(&db, &payer, "after-rebuild").await.is_err());
    budget::settle(&db, "before-rebuild", "0.5".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        management::view(&db, &f.org.admin, &f.channel)
            .await
            .unwrap()["spent_today_credits"],
        "0.5"
    );
    // Another explicit disconnect/recreate keeps the same budget identity.
    crate::handlers::nyxbot::disconnect(&f.org.state, &f.org.admin, &f.channel.id)
        .await
        .unwrap();
    rebuild(&mut f).await;
    assert_eq!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .continuity_channel_id
            .as_deref(),
        Some(original.id.as_str())
    );
    assert_eq!(
        management::view(&db, &f.org.admin, &f.channel)
            .await
            .unwrap()["spent_today_credits"],
        "0.5"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_guest_settings_restore_drops_changed_revoked_node_and_moved_services() {
    let mut f = Fixture::new("org_guest_settings_narrow").await;
    f.wallet().await;
    let mut ids = Vec::new();
    for name in ["unchanged", "changed", "revoked", "node", "moved"] {
        ids.push(f.org.service(&f.org.org, name).await);
    }
    let db = f.org.state.db.clone();
    team::set_grants(
        &db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Add(AgentGrants {
            service_ids: ids.clone(),
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    f.guests(ids.clone()).await;
    db.collection::<Document>("assistant_agents").update_one(doc! {"_id":&f.agent.id},
        doc! {"$set":{format!("guest_access.{}",ids[1]):"read"},"$pull":{"grants.service_ids":&ids[2]}}).await.unwrap();
    for (id, fields) in [
        (&ids[3], doc! {"node_id":"node"}),
        (&ids[4], doc! {"user_id":&f.org.outsider}),
    ] {
        db.collection::<Document>("user_services")
            .update_one(doc! {"_id":id}, doc! {"$set":fields})
            .await
            .unwrap();
    }
    // A failed binding retains its policy for recovery just like a disconnected one.
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"status":"failed","error":"org_access_lost"}},
        )
        .await
        .unwrap();
    db.collection::<Document>("channel_conversations")
        .update_one(
            doc! {"_id":&f.channel.route_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    rebuild(&mut f).await;
    assert_eq!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .service_ids,
        vec![ids[0].clone()]
    );
    assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_guest_settings_restore_stops_at_latest_incompatible_predecessor() {
    let mut f = Fixture::new("org_guest_settings_boundary").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let db = f.org.state.db.clone();
    crate::handlers::nyxbot::disconnect(&f.org.state, &f.org.admin, &f.channel.id)
        .await
        .unwrap();
    let predecessor = f.channel.clone();
    for mismatch in ["org", "agent", "personal"] {
        let mut latest = predecessor.clone();
        latest.id = uuid::Uuid::new_v4().to_string();
        latest.created_at = chrono::Utc::now();
        latest.status = "failed".into();
        match mismatch {
            "org" => latest.org_specialist.as_mut().unwrap().org_id = f.org.outsider.clone(),
            "agent" => latest.agent_id = Some(uuid::Uuid::new_v4().to_string()),
            _ => latest.org_specialist = None,
        }
        db.collection::<NyxbotChannel>("nyxbot_channels")
            .insert_one(&latest)
            .await
            .unwrap();
        rebuild(&mut f).await;
        assert!(
            !f.channel.org_specialist.as_ref().unwrap().guests.enabled,
            "{mismatch}"
        );
        crate::handlers::nyxbot::disconnect(&f.org.state, &f.org.admin, &f.channel.id)
            .await
            .unwrap();
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_guest_settings_tool_matches_web_authority_and_revision_contract() {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_team_tools as tools,
    };
    let mut f = Fixture::new("org_guest_settings_tool").await;
    f.wallet().await;
    let db = &f.org.state.db;
    let thread = crate::services::assistant_authority_tests::new_orchestrator(
        &f.org.state,
        &f.org.admin,
        "Manage guest access",
    )
    .await;
    let chat = acks::for_key(db, &f.org.admin, Some(&thread.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let args = json!({"channel_agent_id":f.channel.id,"org_guests":{"enabled":true,"expected_revision":""}});
    tools::validate("update_channel_access", &args).unwrap();
    assert!(
        tools::validate(
            "update_channel_access",
            &json!({"channel_agent_id":f.channel.id,"org_guests":{"enabled":true}})
        )
        .is_err()
    );
    let service = crate::services::assistant_account_tools::virtual_service(&chat);
    let hints =
        crate::services::mcp_service::tool_annotations("nyxid__update_channel_access", &[service])
            .unwrap();
    assert!(
        hints["title"]
            .as_str()
            .is_some_and(|title| !title.is_empty())
    );
    assert_eq!(hints["readOnlyHint"], false);
    assert!(hints["destructiveHint"].is_boolean());
    let mut guest = chat.clone();
    guest.guest = true;
    let (value, error) = crate::handlers::assistant_team::execute_tool(
        &f.org.state,
        &guest,
        "nyxid__update_channel_access",
        &args,
    )
    .await;
    assert!(error);
    assert_eq!(value, acks::guest_refusal());
    let mut specialist = chat.clone();
    specialist.role = crate::models::assistant_conversation::AgentRole::Subagent;
    let (value, error) = crate::handlers::assistant_team::execute_tool(
        &f.org.state,
        &specialist,
        "nyxid__update_channel_access",
        &args,
    )
    .await;
    assert!(error);
    assert_eq!(value["error"], "orchestrator_only");
    let (saved, error) = crate::handlers::assistant_team::execute_tool(
        &f.org.state,
        &chat,
        "nyxid__update_channel_access",
        &args,
    )
    .await;
    assert!(!error, "{saved}");
    assert_eq!(saved["guest_enabled"], true);
    assert_eq!(saved["daily_credits"], "1");
    assert_eq!(saved["description"], management::ORG_GUEST_EXPLANATION);
    let (_, error) = crate::handlers::assistant_team::execute_tool(
        &f.org.state,
        &chat,
        "nyxid__update_channel_access",
        &args,
    )
    .await;
    assert!(error, "stale tool revision must be rejected");
    let service_id = f.org.service(&f.org.org, "tool-service").await;
    team::set_grants(
        db,
        &f.org.admin,
        &f.agent.id,
        GrantChange::Add(AgentGrants {
            service_ids: vec![service_id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let input = json!({"daily_credits":"2","service_ids":[service_id],"expected_revision":saved["guest_revision"]});
    assert!(
        crate::handlers::org_specialist_channels::update_tool(
            &f.org.state,
            &f.org.member,
            &f.channel.id,
            &input
        )
        .await
        .is_err()
    );
    let (mixed, error) = crate::handlers::assistant_team::execute_tool(
        &f.org.state,
        &chat,
        "nyxid__update_channel_access",
        &json!({"channel_agent_id":f.channel.id,"org_guests":input,"private_chat_posts":true}),
    )
    .await;
    assert!(error, "{mixed}");
    let (saved, error) = crate::handlers::assistant_team::execute_tool(
        &f.org.state,
        &chat,
        "nyxid__update_channel_access",
        &json!({"channel_agent_id":f.channel.id,"org_guests":input}),
    )
    .await;
    assert!(!error, "{saved}");
    assert_eq!(saved["daily_credits"], "2");
    assert_eq!(saved["delegated_service_ids"], json!([service_id]));
    assert_eq!(saved["guest_enabled"], true);
    crate::services::assistant_settings_service::update(
        db,
        &f.org.admin,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (value, error) = crate::handlers::assistant_team::execute_tool(&f.org.state, &chat, "nyxid__update_channel_access",
        &json!({"channel_agent_id":f.channel.id,"org_guests":{"enabled":false,"expected_revision":saved["guest_revision"]}})).await;
    assert!(error);
    assert!(value.get("acknowledgement_id").is_some(), "{value}");
    reload(&mut f).await;
    assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_access_2283_flag_off_management_keeps_forbidden_while_transport_suspends() {
    use axum::{http::StatusCode, response::IntoResponse};
    fn forbidden<T>(result: AppResult<T>) {
        let error = result.err().expect("flag-off must refuse management");
        assert!(
            matches!(
                error,
                AppError::Forbidden(_) | AppError::ForbiddenWithGuidance { .. }
            ),
            "{error}"
        );
        assert_eq!(error.into_response().status(), StatusCode::FORBIDDEN);
    }
    let mut f = Fixture::new("org_guest_settings_flag_off_403").await;
    f.wallet().await;
    f.guests(vec![]).await;
    let origin = f.guest("guest").await;
    let db = &f.org.state.db;
    crate::services::feature_flag_service::set_platform_override(
        db,
        FLAG,
        &crate::services::feature_flag_service::FlagTarget::Global,
        false,
        &f.org.admin,
    )
    .await
    .unwrap();
    forbidden(enabled(db, &f.org.admin).await);
    forbidden(require_target(db, &f.org.admin, &f.agent).await);
    forbidden(link_target(db, &f.org.admin, &f.bot, &f.agent).await);
    forbidden(validate(db, &origin, &f.org.org, &f.agent.id).await);
    forbidden(
        management::update(
            db,
            &f.org.admin,
            &f.channel,
            serde_json::from_value(json!({"enabled":false})).unwrap(),
        )
        .await,
    );
    let mut session = db.client().start_session().await.unwrap();
    forbidden(
        crate::services::org_agent_service::authorize_guest_change(
            db,
            &f.org.admin,
            &f.agent,
            &mut session,
        )
        .await,
    );
    forbidden(fence_management(db, &mut session, &f.channel, &f.org.admin).await);
    assert_eq!(
        transport_access(db, &f.channel).await.unwrap(),
        OrgAccess::Unavailable
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_guest_settings_rebuild_keeps_member_and_erasure_tombstones() {
    let mut f = Fixture::new("org_guest_settings_tombstones").await;
    f.wallet().await;
    f.guests(vec![]).await;
    f.link(&f.org.member, "member-sender").await;
    let original = f.channel.id.clone();
    for _ in 0..2 {
        crate::handlers::nyxbot::disconnect(&f.org.state, &f.org.admin, &f.channel.id)
            .await
            .unwrap();
        rebuild(&mut f).await;
        assert_eq!(
            f.channel
                .org_specialist
                .as_ref()
                .unwrap()
                .guests
                .continuity_channel_id
                .as_deref(),
            Some(original.as_str())
        );
        assert!(
            sender(
                &f.org.state.db,
                &f.channel,
                "member-sender",
                true,
                &f.agent.id
            )
            .await
            .is_err()
        );
        // Confirmation creates only a fresh member link, never reuses an old execution lane.
        assert!(
            !f.link(&f.org.member, "member-sender")
                .await
                .org_execution
                .unwrap()
                .guest
        );
    }
    lifecycle::erase_person(&f.org.state.db, &f.org.member)
        .await
        .unwrap();
    crate::handlers::nyxbot::disconnect(&f.org.state, &f.org.admin, &f.channel.id)
        .await
        .unwrap();
    rebuild(&mut f).await;
    assert!(
        sender(
            &f.org.state.db,
            &f.channel,
            "member-sender",
            true,
            &f.agent.id
        )
        .await
        .is_err()
    );
    assert!(
        f.guest("unclaimed-visitor")
            .await
            .org_execution
            .unwrap()
            .guest
    );
    f.org.state.db.drop().await.unwrap();
}
