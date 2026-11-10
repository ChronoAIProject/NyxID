use super::*;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
use crate::models::user::{COLLECTION_NAME as USERS, UserType};

fn facts(platform: &'static str, status: &'static str) -> HealthFacts<'static> {
    HealthFacts {
        platform,
        status,
        webhook_registered: true,
        failure_reason: None,
        outbound_reason: None,
        subscription_registered: None,
        backoff_active: false,
        gateway: false,
        discord_intake: None,
    }
}

/// (platform, status, webhook_registered, failure_reason, backoff, expected action)
type Row = (
    &'static str,
    &'static str,
    bool,
    Option<&'static str>,
    bool,
    &'static str,
);

#[test]
fn recommendation_table_covers_status_mode_and_failure_codes() {
    use codes::*;
    // (platform, status, webhook_registered, failure_reason, backoff, expected action)
    let rows: &[Row] = &[
        ("x", "active", true, None, false, "none"),
        ("x", "active", false, None, false, "none"),
        ("x", "active", false, None, true, "wait_rate_limit"),
        (
            "x",
            "active",
            false,
            Some(RATE_LIMITED),
            true,
            "wait_rate_limit",
        ),
        ("x", "active", false, Some(RATE_LIMITED), false, "none"),
        (
            "x",
            "failed",
            true,
            Some(BILLING_SUSPENDED),
            false,
            "enable_billing",
        ),
        (
            "x",
            "failed",
            false,
            Some(RECONNECT_REQUIRED),
            false,
            "reconnect",
        ),
        ("x", "failed", false, Some(AUTH_REVOKED), false, "reconnect"),
        (
            "x",
            "failed",
            false,
            Some(SCOPES_MISSING),
            false,
            "reconnect",
        ),
        (
            "x",
            "failed",
            false,
            Some(APP_ACCESS_DENIED),
            false,
            "contact_admin",
        ),
        (
            "x",
            "failed",
            false,
            Some(PROVIDER_CREDITS),
            false,
            "contact_admin",
        ),
        (
            "x",
            "active",
            false,
            Some(WEBHOOK_UNCONFIGURED),
            false,
            "contact_admin",
        ),
        (
            "x",
            "failed",
            false,
            Some(WEBHOOK_REQUIRED),
            false,
            "resubscribe",
        ),
        (
            "x",
            "failed",
            false,
            Some(WEBHOOK_SETUP_FAILED),
            false,
            "resubscribe",
        ),
        (
            "x",
            "active",
            true,
            Some(SUBSCRIPTION_MISSING),
            false,
            "resubscribe",
        ),
        (
            "x",
            "active",
            true,
            Some(UNREGISTERED),
            false,
            "resubscribe",
        ),
        (
            "x",
            "failed",
            false,
            Some(PROVIDER_UNAVAILABLE),
            true,
            "verify",
        ),
        // Unknown or legacy free-text causes on a failed bot: reconnect.
        (
            "x",
            "failed",
            false,
            Some("Reconnect the account"),
            false,
            "reconnect",
        ),
        ("x", "failed", false, None, false, "reconnect"),
        // Other platforms never get the X-only resubscribe action.
        (
            "lark",
            "active",
            true,
            Some(SUBSCRIPTION_MISSING),
            false,
            "verify",
        ),
        (
            "telegram",
            "failed",
            true,
            Some(AUTH_REVOKED),
            false,
            "reconnect",
        ),
        ("whatsapp", "active", true, None, false, "none"),
    ];
    for (platform, status, registered, reason, backoff, expected) in rows {
        let facts = HealthFacts {
            platform,
            status,
            webhook_registered: *registered,
            failure_reason: *reason,
            outbound_reason: None,
            subscription_registered: None,
            backoff_active: *backoff,
            gateway: false,
            discord_intake: None,
        };
        assert_eq!(recommend(&facts).action, *expected, "{facts:?}");
    }
}

#[test]
fn advice_text_is_platform_specific_and_separates_provider_from_nyxid_credits() {
    let mut x = facts("x", "failed");
    x.failure_reason = Some(codes::PROVIDER_CREDITS);
    let text = recommend(&x).text;
    assert!(text.contains("X API credits"), "{text}");
    assert!(text.contains("separate from NyxID credits"), "{text}");
    let mut lark = facts("lark", "failed");
    lark.failure_reason = Some(codes::AUTH_REVOKED);
    let text = recommend(&lark).text;
    assert!(text.contains("Lark account"), "{text}");
    assert!(!text.contains("X "), "{text}");
}

#[test]
fn subscription_outbound_and_gateway_facts_drive_the_action() {
    let mut f = facts("x", "active");
    f.subscription_registered = Some(false);
    assert_eq!(recommend(&f).action, "resubscribe");
    f.subscription_registered = Some(true);
    f.outbound_reason = Some(codes::RATE_LIMITED);
    assert_eq!(recommend(&f).action, "wait_rate_limit");
    f.outbound_reason = Some(codes::SCOPES_MISSING);
    assert_eq!(recommend(&f).action, "reconnect");
    f.outbound_reason = Some(codes::RECIPIENT_UNREACHABLE);
    assert_eq!(recommend(&f).action, "none");
    f.failure_reason = Some(codes::BILLING_SUSPENDED);
    f.gateway = true;
    assert_eq!(recommend(&f).action, "gateway");
}

#[test]
fn backoff_is_compared_with_now() {
    let mut bot: ChannelBot = bson::from_document(doc! {
        "_id": "b", "user_id": "u", "platform": "x", "label": "x",
        "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "platform_bot_id": "10", "platform_bot_username": "x", "webhook_registered": false,
        "webhook_secret_hash": "", "status": "active", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    })
    .unwrap();
    let now = Utc::now();
    bot.poll_backoff_until = Some(now - chrono::Duration::minutes(1));
    assert!(!HealthFacts::for_bot(&bot, now, false).backoff_active);
    assert_eq!(
        recommend(&HealthFacts::for_bot(&bot, now, false)).action,
        "none"
    );
    bot.poll_backoff_until = Some(now + chrono::Duration::minutes(1));
    assert!(HealthFacts::for_bot(&bot, now, false).backoff_active);
    assert_eq!(
        recommend(&HealthFacts::for_bot(&bot, now, false)).action,
        "wait_rate_limit"
    );
    assert_eq!(HealthFacts::for_bot(&bot, now, false).mode(), "poll");
    bot.webhook_registered = true;
    assert_eq!(HealthFacts::for_bot(&bot, now, false).mode(), "webhook");
}

#[test]
fn failure_codes_classify_errors() {
    assert_eq!(
        failure_code(&AppError::ChannelPlatformError(
            "X: rate limited; retry after 5s (HTTP 429)".into()
        )),
        codes::RATE_LIMITED
    );
    assert_eq!(
        failure_code(&AppError::ChannelPlatformError("X: down (HTTP 503)".into())),
        codes::PROVIDER_UNAVAILABLE
    );
    assert_eq!(
        failure_code(&AppError::ChannelPlatformError("no status".into())),
        codes::PROVIDER_ERROR
    );
    assert_eq!(
        failure_code(&AppError::ChannelConversationNotReachable("x".into())),
        codes::RECIPIENT_UNREACHABLE
    );
    assert_eq!(
        failure_code(&AppError::InsufficientCredits),
        codes::BILLING_SUSPENDED
    );
    assert_eq!(
        failure_code(&AppError::ValidationError("connection revoked".into())),
        codes::RECONNECT_REQUIRED
    );
}

#[tokio::test]
async fn unauthenticated_rejections_are_throttled_counters_without_bot_ids() {
    let db = crate::test_utils::connect_test_database("channel_webhook_rejections")
        .await
        .unwrap();
    // A platform name unique to this test keeps the process-wide throttle isolated.
    let platform = format!("test-{}", uuid::Uuid::new_v4().simple());
    for _ in 0..5 {
        record_webhook_rejection(&db, &platform, "signature").await;
    }
    let rows = list_webhook_rejections(&db).await.unwrap();
    let row = rows.iter().find(|r| r.platform == platform).unwrap();
    // The first rejection flushes; the next four wait for the throttle window.
    assert_eq!(row.count, 1);
    assert_eq!(row.reason, "signature");
    assert_eq!(row.id, format!("{platform}:signature"));
    reset_rejection_throttle();
    record_webhook_rejection(&db, &platform, "signature").await;
    let rows = list_webhook_rejections(&db).await.unwrap();
    // A new throttle window flushes again on its first rejection.
    assert_eq!(
        rows.iter().find(|r| r.platform == platform).unwrap().count,
        2
    );
}

#[tokio::test]
async fn failure_notices_go_to_every_org_admin_or_the_personal_owner() {
    let db = crate::test_utils::connect_test_database("channel_bot_failure_notice")
        .await
        .unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let admins = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    let viewer = uuid::Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(crate::test_utils::test_user(&org, UserType::Org))
        .await
        .unwrap();
    for (member, role) in [
        (&admins[0], OrgRole::Admin),
        (&admins[1], OrgRole::Admin),
        (&viewer, OrgRole::Viewer),
    ] {
        db.collection(USERS)
            .insert_one(crate::test_utils::test_user(member, UserType::Person))
            .await
            .unwrap();
        db.collection(MEMBERSHIPS)
            .insert_one(crate::test_utils::test_membership(&org, member, role, None))
            .await
            .unwrap();
    }
    let mut bot: ChannelBot = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": &org, "platform": "x", "label": "Org bot",
        "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "platform_bot_id": "10", "platform_bot_username": "x", "webhook_registered": true,
        "webhook_secret_hash": "", "status": "failed", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    })
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    TEST_NOTICES.lock().unwrap().insert(bot.id.clone(), tx);
    let advice = recommend(&facts("x", "failed"));
    notify_failure(&db, &bot, &advice);
    let mut recipients = Vec::new();
    for _ in 0..2 {
        let (recipient, context) =
            tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
        assert_eq!(context.recommended_action, "reconnect");
        assert_eq!(context.platform_name, "X");
        recipients.push(recipient);
    }
    recipients.sort();
    let mut expected = admins.to_vec();
    expected.sort();
    assert_eq!(
        recipients, expected,
        "admins only, never the viewer or the org user"
    );

    // A personal bot notifies its owner.
    let owner = uuid::Uuid::new_v4().to_string();
    bot.user_id = owner.clone();
    notify_failure(&db, &bot, &advice);
    let (recipient, _) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recipient, owner);
    TEST_NOTICES.lock().unwrap().remove(&bot.id);
}

fn discord_bot() -> ChannelBot {
    bson::from_document(doc! {
        "_id":"discord-health","user_id":"owner","platform":"discord","label":"Bot",
        "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
        "platform_bot_id":"100","platform_bot_username":"bot","webhook_registered":false,
        "webhook_secret_hash":"","status":"active","is_active":true,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    })
    .unwrap()
}

#[test]
fn discord_intake_states_never_report_healthy_without_current_evidence() {
    use crate::models::discord_gateway::DiscordGatewayStatus;
    let mut bot = discord_bot();
    for state in [
        "stopped",
        "disabled",
        "waiting",
        "connecting",
        "disconnected",
        "connected",
    ] {
        bot.diagnostics.discord_gateway = Some(DiscordGatewayStatus {
            state: state.into(),
            reason: None,
            updated_at: Utc::now(),
        });
        let mut facts = HealthFacts::for_bot(&bot, Utc::now(), false);
        facts.discord_intake = Some(discord::project(&bot, None, None, true));
        assert_eq!(
            recommend(&facts).action == "none",
            state == "connected",
            "{state}"
        );
        facts.discord_intake = Some(discord::project(&bot, None, None, false));
        assert_ne!(recommend(&facts).action, "none", "stale {state}");
        facts.discord_intake = Some(discord::project(
            &bot,
            None,
            Some("release_unresolved"),
            true,
        ));
        assert_eq!(
            facts.discord_intake.as_ref().unwrap().code,
            Some(discord::RELAY_BLOCKED)
        );
        assert_eq!(recommend(&facts).action, "contact_admin");
    }
    let unloaded = HealthFacts::for_bot(&bot, Utc::now(), false);
    assert!(unloaded.discord_intake.is_none());
    assert_eq!(recommend(&unloaded).action, "none");
    assert_eq!(recommend(&unloaded).text, "Healthy.");
    let gateway = HealthFacts::for_bot(&bot, Utc::now(), true);
    assert!(gateway.discord_intake.is_none());
    assert_eq!(recommend(&gateway).action, "gateway");
}

#[tokio::test]
async fn discord_health_live_lease_and_release_fences_are_batched_without_provider_calls() {
    let db = crate::test_utils::connect_test_database("discord_health_evidence")
        .await
        .expect("review Mongo required");
    let mut bot = discord_bot();
    discord_flag(&db, None, "global", None, true).await;
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    bot.diagnostics.discord_gateway = Some(crate::models::discord_gateway::DiscordGatewayStatus {
        state: "connected".into(),
        reason: None,
        updated_at: Utc::now() - chrono::Duration::days(2),
    });
    assert_eq!(
        discord::load(&db, std::slice::from_ref(&bot))
            .await
            .unwrap()[&bot.id]
            .code,
        Some(discord::RELAY_UNCONFIRMED)
    );
    let inventory = setup_evidence(&db, &bot, false).await.unwrap();
    assert_ne!(inventory["recommended_action"], "none");
    assert_eq!(
        inventory["discord_intake"]["code"],
        discord::RELAY_UNCONFIRMED
    );
    db.collection::<bson::Document>(crate::models::coordination::LEASE_COLLECTION_NAME).insert_one(doc!{"_id":format!("discord-gateway:{}",bot.id),"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(1))}).await.unwrap();
    assert!(
        discord::load(&db, std::slice::from_ref(&bot))
            .await
            .unwrap()[&bot.id]
            .code
            .is_none()
    );
    db.collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
        .insert_one(doc! {"_id":"uncertain","channel_bot_id":&bot.id,"state":"review_required"})
        .await
        .unwrap();
    let loaded = discord::load(&db, std::slice::from_ref(&bot))
        .await
        .unwrap();
    assert_eq!(loaded[&bot.id].code, Some(discord::RELAY_BLOCKED));
    assert_eq!(loaded[&bot.id].block_reason, Some("release_unresolved"));
    bot.platform = "lark".into();
    assert!(discord::load(&db, &[bot]).await.unwrap().is_empty());
    db.drop().await.unwrap();
}

async fn discord_flag(
    db: &mongodb::Database,
    org: Option<&str>,
    kind: &str,
    target: Option<&str>,
    enabled: bool,
) {
    db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(), "flag_key":crate::services::feature_flag_service::DISCORD_GATEWAY_FLAG_KEY,
        "org_user_id":org,"target_kind":kind,"target_key":target,"enabled":enabled,
        "updated_by":"test","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
}

#[tokio::test]
async fn discord_health_flag_off_is_neutral_and_enabled_unready_is_blocked() {
    let db = crate::test_utils::connect_test_database("discord_health_flags")
        .await
        .unwrap();
    let mut bot = discord_bot();
    for state in [None, Some("stopped"), Some("connected")] {
        bot.diagnostics.discord_gateway =
            state.map(|s| crate::models::discord_gateway::DiscordGatewayStatus {
                state: s.into(),
                reason: None,
                updated_at: Utc::now(),
            });
        let loaded = discord::load(&db, std::slice::from_ref(&bot))
            .await
            .unwrap();
        let intake = &loaded[&bot.id];
        assert_eq!(intake.receiving_side, "interactions_only");
        assert!(intake.code.is_none());
        let inventory = setup_evidence(&db, &bot, false).await.unwrap();
        assert_eq!(inventory["recommended_action"], "none");
        assert_eq!(inventory["recommended_action_text"], "Healthy.");
    }
    discord_flag(&db, None, "user", Some(&bot.user_id), true).await;
    let loaded = discord::load(&db, std::slice::from_ref(&bot))
        .await
        .unwrap();
    assert_eq!(loaded[&bot.id].code, Some(discord::RELAY_BLOCKED));
    assert_eq!(loaded[&bot.id].block_reason, Some("readiness_unavailable"));
    assert!(
        discord::advice(&loaded[&bot.id])
            .unwrap()
            .text
            .contains("fleet readiness")
    );
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    let loaded = discord::load(&db, std::slice::from_ref(&bot))
        .await
        .unwrap();
    assert_eq!(loaded[&bot.id].code, Some(discord::RELAY_UNCONFIRMED));
    bot.status = "failed".into();
    bot.diagnostics.failure_reason = Some(codes::AUTH_REVOKED.into());
    assert!(
        discord::load(&db, std::slice::from_ref(&bot))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        setup_evidence(&db, &bot, false).await.unwrap()["recommended_action"],
        "reconnect"
    );
    bot.status = "active".into();
    bot.is_active = false;
    assert!(discord::load(&db, &[bot]).await.unwrap().is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn discord_health_batch_matches_personal_org_global_and_revoked_membership_precedence() {
    use crate::services::feature_flag_service as flags;
    let db = crate::test_utils::connect_test_database("discord_health_precedence")
        .await
        .unwrap();
    crate::services::security_readiness_service::ready_test_replica(&db).await;
    discord_flag(&db, None, "global", None, true).await;
    discord_flag(&db, Some("org"), "org", None, false).await;
    discord_flag(&db, None, "user", Some("personal"), true).await;
    discord_flag(&db, None, "user", Some("disabled"), false).await;
    for (person, revoked) in [("member", false), ("personal", false), ("former", true)] {
        db.collection::<bson::Document>(MEMBERSHIPS).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"org_user_id":"org","member_user_id":person,"role":"member",
            "created_at":bson::DateTime::now(),"revoked_at":if revoked {bson::Bson::DateTime(bson::DateTime::now())}else{bson::Bson::Null},
        }).await.unwrap();
    }
    let bots: Vec<_> = ["member", "personal", "former", "disabled", "org", "global"]
        .into_iter()
        .map(|owner| {
            let mut bot = discord_bot();
            bot.id = owner.into();
            bot.user_id = owner.into();
            bot
        })
        .collect();
    let loaded = discord::load(&db, &bots).await.unwrap();
    for (case, bot) in bots.into_iter().enumerate() {
        // Test-only individual resolution is the existing ingress authority;
        // the production loader resolves all these owners in two batched reads.
        let expected =
            flags::personal_flag_enabled(&db, &bot.user_id, flags::DISCORD_GATEWAY_FLAG_KEY)
                .await
                .unwrap();
        assert_eq!(
            loaded[&bot.id].code.is_some(),
            expected,
            "owner case #{case}"
        );
    }
    db.drop().await.unwrap();
}
