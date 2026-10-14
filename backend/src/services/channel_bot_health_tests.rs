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
