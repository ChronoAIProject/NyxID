//! Real receipt/control delivery and native creation; Telegram/provider HTTP is local.
use super::*;
use crate::models::{
    channel_bot::ChannelBot,
    platform_binding::PlatformBinding,
    platform_setup::{PlatformSetup, SetupFacts, SetupState, SetupTemplate},
};
use crate::services::{channel_connect_link_service as links, platform_setup_service as setups};

async fn command(j: &Journey, sequence: i64, text: &str) -> String {
    j.webhook(&j.update(sequence, text, false)).await;
    let result = j.sends().await.last().unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    result
}

async fn next_personal_request(j: &Journey, account: &str, sequence: i64) -> Value {
    let expected = j.egress.0.load(Ordering::SeqCst) + 1;
    j.webhook(&j.update(sequence, "Help me with my selected setup", false))
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if j.egress.0.load(Ordering::SeqCst) >= expected {
                let receipt = j
                    .state
                    .db
                    .collection::<PlatformManagerIngress>(INGRESS)
                    .find_one(doc! {"update_id":sequence})
                    .await
                    .unwrap()
                    .unwrap();
                let execution = receipt.execution.unwrap();
                if j.state
                    .db
                    .collection::<AssistantMessage>(MESSAGES)
                    .find_one(doc! {
                        "user_id":account,"conversation_id":&execution.conversation_id,
                        "turn_id":&execution.turn_id,"role":"assistant",
                    })
                    .await
                    .unwrap()
                    .is_some()
                {
                    return j.egress.1.lock().await.last().unwrap().clone();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn personal_account_controls_management_with_legacy_foreign_binding_and_disabled_guide() {
    let j = Journey::new().await;
    assert!(!j.profile.enabled && j.profile.guide.is_none());
    let account = uuid::Uuid::new_v4().to_string();
    j.state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &account,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let now = Utc::now();
    j.state
        .db
        .collection::<PlatformBinding>(crate::models::platform_binding::COLLECTION_NAME)
        .insert_one(PlatformBinding {
            id: uuid::Uuid::new_v4().to_string(),
            platform_config_id: j.profile.id.clone(),
            manager_bot_id: "100".into(),
            profile_generation: 1,
            telegram_sender_id: "700".into(),
            telegram_chat_id: "700".into(),
            user_id: j.owner.clone(),
            generation: 1,
            revocation_generation: 0,
            is_active: true,
            created_at: now,
            updated_at: now,
            expires_at: now + Duration::days(1),
        })
        .await
        .unwrap();
    super::account_parity::confirm_account(&j, 700, &account, 5000).await;
    setups::ensure_indexes(&j.state.db).await.unwrap();
    let facts = SetupFacts {
        name: "Owned setup".into(),
        timezone: "UTC".into(),
        ..Default::default()
    };
    let own = setups::create_owned(
        &j.state.db,
        &j.state.encryption_keys,
        &account,
        &uuid::Uuid::new_v4().to_string(),
        SetupTemplate::TelegramAssistant,
        &facts,
    )
    .await
    .unwrap();
    let foreign = setups::create_owned(
        &j.state.db,
        &j.state.encryption_keys,
        &j.owner,
        &uuid::Uuid::new_v4().to_string(),
        SetupTemplate::TelegramAssistant,
        &facts,
    )
    .await
    .unwrap();
    let listed = command(&j, 5010, "/setups").await;
    assert!(listed.contains(&own.id));
    assert!(!listed.contains(&foreign.id));
    let selected = command(&j, 5011, &format!("/setup {}", own.id)).await;
    assert!(selected.contains(&format!("Selected setup {}", own.id)));
    let choice = j
        .state
        .db
        .collection::<Document>("platform_personal_setup_selections")
        .find_one(doc! {"user_id":&account})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(choice.get_str("setup_id").unwrap(), own.id);
    assert!(
        command(&j, 5012, &format!("/setup {}", foreign.id))
            .await
            .contains("unavailable for your linked account")
    );
    assert_eq!(
        j.egress.0.load(Ordering::SeqCst),
        0,
        "reserved controls must not execute a model"
    );
    let next = next_personal_request(&j, &account, 5020).await.to_string();
    assert!(next.contains(&format!("owner selected onboarding setup {}", own.id)));
    assert!(!next.contains(&foreign.id));
    // Live expiry must apply before cleanup to collecting and approval rows.
    for state in ["collecting", "awaiting_approval"] {
        j.state.db.collection::<Document>(crate::models::platform_setup::COLLECTION_NAME).update_one(doc! {"_id":&own.id},
            doc! {"$set":{"state":state,"approved_digest":"old-approval","approved_revision":1_i64,
            "expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        let text = command(
            &j,
            if state == "collecting" { 5013 } else { 5014 },
            "/setups",
        )
        .await;
        assert!(text.contains("Expired"));
        let row = j
            .state
            .db
            .collection::<PlatformSetup>(crate::models::platform_setup::COLLECTION_NAME)
            .find_one(doc! {"_id":&own.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, SetupState::Expired);
        assert!(row.approved_digest.is_none() && row.approved_revision.is_none());
    }
    let next = next_personal_request(&j, &account, 5021).await.to_string();
    assert!(
        !next.contains("owner selected onboarding setup"),
        "expired selection must not enter a new turn"
    );
    // Even a live owned selection from an earlier binding generation is ineligible.
    j.state.db.collection::<Document>(crate::models::platform_setup::COLLECTION_NAME)
        .update_one(doc! {"_id":&own.id},doc! {"$set":{"state":"collecting","expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(1))}}).await.unwrap();
    j.state
        .db
        .collection::<Document>(crate::models::platform_manager_management::SELECTIONS)
        .update_one(
            doc! {"_id":choice.get_str("_id").unwrap()},
            doc! {"$inc":{"binding_generation":1_i64}},
        )
        .await
        .unwrap();
    let next = next_personal_request(&j, &account, 5022).await.to_string();
    assert!(!next.contains("owner selected onboarding setup"));
    let manager = j
        .state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&j.profile.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(manager.user_id, j.owner);
    let stored_manager = j
        .state
        .db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&manager.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored_manager.get_str("platform_profile_id").unwrap(),
        j.profile.id
    );
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn deterministic_and_natural_creation_links_reject_browser_b_and_create_for_chat_a() {
    let j = Journey::new().await;
    let account = uuid::Uuid::new_v4().to_string();
    j.state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &account,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    // Legacy person documents omit user_type but retain normal person policy.
    j.state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id":&account}, doc! {"$unset":{"user_type":""}})
        .await
        .unwrap();
    super::account_parity::confirm_account(&j, 700, &account, 5100).await;
    let output = j.control(5110, "/onboarding").await.unwrap();
    let commands::PersonalCommandReply::Management {
        connect_token: Some(token),
    } = &output.reply
    else {
        panic!("bound creation link")
    };
    let token = zeroize::Zeroizing::new(token.to_string());
    assert!(
        links::claim(&j.state.db, &j.owner, &token, "telegram")
            .await
            .is_err()
    );
    let base = j.server.uri();
    let api = crate::services::telegram_new_api::TelegramApi {
        http: &j.state.http_client,
        base_url: &base,
    };
    assert!(matches!(
        delivery::deliver_command(&j.state, j.producers.identity.clone(), output, &api)
            .await
            .unwrap(),
        DeliveryOutcome::Sent { .. }
    ));
    assert!(
        j.sends().await.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("/connect/bot/")
    );
    let claim = links::claim(&j.state.db, &account, &token, "telegram")
        .await
        .unwrap();
    let service = telegram::service(&j.state, &base);
    let (request, launch) = service
        .begin_linked(&account, &account, &claim.link.label, &claim)
        .await
        .unwrap();
    assert_eq!(request.actor_user_id, account);
    assert_eq!(request.owner_user_id, account);
    links::release(&j.state.db, &claim, false).await.unwrap();
    let challenge = reqwest::Url::parse(&launch)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    j.webhook(&j.update(5111, &format!("/start {challenge}"), false))
        .await;
    let child = json!({"id":900,"is_bot":true,"username":"CustomerBot"});
    j.webhook(
        &json!({"update_id":5112,"managed_bot":{"user":{"id":700,"is_bot":false},"bot":child}}),
    )
    .await;
    let mut created = j.update(5113, "", false);
    created["message"]["managed_bot_created"] = json!({"bot":child});
    j.webhook(&created).await;
    telegram::provider_connection(&j.server, &request.id, 200).await;
    service.complete_pending_creations().await.unwrap();
    let owned = j
        .state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owned.user_id, account);
    let stored_owned = j
        .state
        .db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&owned.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!stored_owned.contains_key("platform_profile_id"));
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let listed = command(&j, 5114, "/bots").await;
    assert!(listed.contains(&owned.id));
    assert!(!listed.contains(&j.profile.channel_bot_id));
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&j.state.db, &account)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &j.state.db,
        &j.state.encryption_keys,
        &agent,
    )
    .await
    .unwrap();
    let (value, _) = crate::handlers::nyxbot::setup_link_tool(
        &j.state,
        &account,
        &home.id,
        "telegram-new",
        Some(&"猫".repeat(60)),
        &agent,
        crate::services::nyxbot_watch_service::ReportProvenance::LEGACY,
    )
    .await
    .unwrap();
    let token = zeroize::Zeroizing::new(
        reqwest::Url::parse(value["url"].as_str().unwrap())
            .unwrap()
            .path_segments()
            .unwrap()
            .next_back()
            .unwrap()
            .to_owned(),
    );
    assert!(
        links::claim(&j.state.db, &j.owner, &token, "telegram")
            .await
            .is_err()
    );
    let claim = links::claim(&j.state.db, &account, &token, "telegram")
        .await
        .unwrap();
    assert!(claim.link.label.len() <= 128 && claim.link.label.chars().all(|c| c == '猫'));
    assert_eq!(
        value["expires_in_minutes"],
        (claim.link.expires_at - claim.link.created_at).num_minutes()
    );
    assert_eq!(value["expires_in_minutes"], 60);
    assert_eq!(
        claim.link.callback_url.as_deref(),
        Some(
            format!(
                "{}/assistant?c={}",
                j.state.config.frontend_url.trim_end_matches('/'),
                home.id
            )
            .as_str()
        )
    );
    let (next, _) = service
        .begin_linked(&account, &account, &claim.link.label, &claim)
        .await
        .unwrap();
    assert_eq!(next.owner_user_id, account);
    assert_eq!(next.actor_user_id, account);
    links::release(&j.state.db, &claim, false).await.unwrap();
    service.cancel(&account, &next.id).await.unwrap();
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_legacy_onboarding_confirmation_survives_personal_runtime_upgrade() {
    for grouped in [false, true] {
        let j = Journey::new().await;
        let mut profile = j.profile.clone();
        profile.enabled = true;
        j.state
            .db
            .collection::<PlatformBotConfig>(PROFILES)
            .replace_one(doc! {"_id":&profile.id}, &profile)
            .await
            .unwrap();
        let key = j.state.auth_device_hmac_key.as_slice();
        let (_, code) = crate::services::platform_binding_service::begin(
            &j.state.db,
            key,
            &profile,
            "700",
            "700",
            5200,
        )
        .await
        .unwrap();
        crate::services::platform_binding_service::decide(
            &j.state.db,
            key,
            &j.owner,
            &code,
            &code,
            true,
            false,
        )
        .await
        .unwrap();
        let account = uuid::Uuid::new_v4().to_string();
        j.state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &account,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        super::account_parity::confirm_account(&j, 700, &account, 5201).await;
        let formatted = if grouped {
            format!("{}-{}", &code[..4], &code[4..])
        } else {
            code.to_string()
        }
        .to_lowercase();
        let text = zeroize::Zeroizing::new(format!("/confirm {formatted}"));
        j.webhook(&j.update(5203, &text, true)).await;
        assert!(
            crate::services::platform_binding_service::live_for_sender(
                &j.state.db,
                &profile,
                "700",
                "700"
            )
            .await
            .unwrap()
            .is_none()
        );
        j.webhook(&j.update(5204, &format!("/confirm@OtherBot {formatted}"), false))
            .await;
        assert!(
            crate::services::platform_binding_service::live_for_sender(
                &j.state.db,
                &profile,
                "700",
                "700"
            )
            .await
            .unwrap()
            .is_none()
        );
        j.webhook(&j.update(5205, &text, false)).await;
        let legacy = crate::services::platform_binding_service::live_for_sender(
            &j.state.db,
            &profile,
            "700",
            "700",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(legacy.user_id, j.owner);
        let personal = j
            .state
            .db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"telegram_sender_id":700_i64,"state":"active"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            personal.user_id, account,
            "legacy proof cannot select the personal account"
        );
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
        j.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn setup_acknowledgement_tracks_exact_selection_and_expired_choice_keeps_prior_context() {
    let j = Journey::new().await;
    j.confirm_login(5300).await;
    setups::ensure_indexes(&j.state.db).await.unwrap();
    let facts = SetupFacts {
        name: "Selection test".into(),
        timezone: "UTC".into(),
        ..Default::default()
    };
    let mut choices = Vec::new();
    for _ in 0..2 {
        choices.push(
            setups::create_owned(
                &j.state.db,
                &j.state.encryption_keys,
                &j.owner,
                &uuid::Uuid::new_v4().to_string(),
                SetupTemplate::TelegramAssistant,
                &facts,
            )
            .await
            .unwrap(),
        );
    }
    let a = &choices[0];
    let b = &choices[1];
    let delayed_a = j.control(5310, &format!("/setup {}", a.id)).await.unwrap();
    j.control(5311, &format!("/setup {}", b.id)).await.unwrap();
    let base = j.server.uri();
    let api = crate::services::telegram_new_api::TelegramApi {
        http: &j.state.http_client,
        base_url: &base,
    };
    assert!(matches!(
        delivery::deliver_command(&j.state, j.producers.identity.clone(), delayed_a, &api)
            .await
            .unwrap(),
        DeliveryOutcome::Sent { .. }
    ));
    let sent = j.sends().await.last().unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!sent.contains("Selected setup"));
    assert!(sent.contains("not the current selection"));
    let selected = j
        .state
        .db
        .collection::<Document>(crate::models::platform_manager_management::SELECTIONS)
        .find_one(doc! {"user_id":&j.owner})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.get_str("setup_id").unwrap(), b.id);
    j.control(5312, &format!("/setup {}", a.id)).await.unwrap();
    for (offset, state) in ["expired", "cancelled"].into_iter().enumerate() {
        j.state
            .db
            .collection::<Document>(crate::models::platform_setup::COLLECTION_NAME)
            .update_one(doc! {"_id":&b.id}, doc! {"$set":{"state":state}})
            .await
            .unwrap();
        let skipped = j
            .control(5320 + offset as i64, &format!("/setup {}", b.id))
            .await
            .unwrap();
        let mut session = j.state.db.client().start_session().await.unwrap();
        let update = dispatch::verified_message(
            &j.profile.id,
            "100",
            100,
            &j.profile.observation_id,
            &j.update(5320 + offset as i64, &format!("/setup {}", b.id), false),
        )
        .unwrap();
        let body = crate::services::platform_manager_management::render_in_session(
            &j.state.db,
            &mut session,
            &update,
            &skipped.effect,
            &j.state.config.frontend_url,
            None,
        )
        .await
        .unwrap();
        assert!(!body.contains("Selected setup"));
        let next = next_personal_request(&j, &j.owner, 5330 + offset as i64)
            .await
            .to_string();
        assert!(next.contains(&format!("owner selected onboarding setup {}", a.id)));
        assert!(!next.contains(&b.id));
    }
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn bot_management_status_uses_live_health_at_listing_detail_and_final_send() {
    let j = Journey::new().await;
    j.confirm_login(5400).await;
    let bots = j
        .state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME);
    let prototype = bots
        .find_one(doc! {"_id":&j.profile.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    let mut ids = Vec::new();
    for (number, (active, status, expected)) in [
        (true, "active", "active"),
        (true, "pending", "setup pending"),
        (true, "pending_webhook", "webhook setup pending"),
        (true, "failed", "failed; review connection and verify"),
        (
            true,
            "invalid",
            "invalid credentials; review connection and verify",
        ),
        (false, "active", "paused"),
        (false, "failed", "paused"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut bot = prototype.clone();
        bot.id = uuid::Uuid::new_v4().to_string();
        bot.platform = "slack".into();
        bot.platform_bot_id = format!("55{number}");
        bot.platform_bot_username = format!("owned{number}bot");
        bot.label = format!("Owned status {number}");
        bot.is_active = active;
        bot.status = status.into();
        // Typed ordinary insertion deliberately carries no shared-manager marker.
        bots.insert_one(&bot).await.unwrap();
        ids.push((bot, expected));
    }
    let listing = command(&j, 5410, "/bots").await;
    for (number, (bot, expected)) in ids.iter().enumerate() {
        assert!(
            listing.contains(&format!("{} — {} ({expected})", bot.id, bot.label)),
            "{listing}"
        );
        let detail = command(&j, 5411 + number as i64, &format!("/bot {}", bot.id)).await;
        assert!(
            detail.contains(&format!("Bot {} — {expected}\n", bot.id)),
            "{detail}"
        );
    }
    assert_eq!(
        j.egress.0.load(Ordering::SeqCst),
        0,
        "status controls never start a model turn"
    );
    let bot = &ids[0].0;
    for (sequence, text) in [
        (5430, "/bots".to_owned()),
        (5431, format!("/bot {}", bot.id)),
    ] {
        bots.update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"status":"active","is_active":true}},
        )
        .await
        .unwrap();
        let delayed = j.control(sequence, &text).await.unwrap();
        crate::services::channel_bot_service::mark_bot_failed(&j.state.db, &bot.id)
            .await
            .unwrap();
        let failed = bots.find_one(doc! {"_id":&bot.id}).await.unwrap().unwrap();
        assert!(failed.is_active);
        assert_eq!(failed.status, "failed");
        let base = j.server.uri();
        let api = crate::services::telegram_new_api::TelegramApi {
            http: &j.state.http_client,
            base_url: &base,
        };
        assert!(matches!(
            delivery::deliver_command(&j.state, j.producers.identity.clone(), delayed, &api)
                .await
                .unwrap(),
            DeliveryOutcome::Sent { .. }
        ));
        let sent = j.sends().await.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .to_owned();
        if text == "/bots" {
            assert!(
                sent.contains(&format!(
                    "{} — {} (failed; review connection and verify)",
                    bot.id, bot.label
                )),
                "{sent}"
            );
        } else {
            assert!(
                sent.contains(&format!(
                    "Bot {} — failed; review connection and verify\n",
                    bot.id
                )),
                "{sent}"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }
    // Pause this task only after management text is prepared, before the send claim.
    bots.update_one(doc! {"_id":&bot.id}, doc! {"$set":{"status":"active"}})
        .await
        .unwrap();
    let delayed = j.control(5440, &format!("/bot {}", bot.id)).await.unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let worker_barrier = barrier.clone();
    let state = j.state.clone();
    let identity = j.producers.identity.clone();
    let base = j.server.uri();
    let send = tokio::spawn(async move {
        let api = crate::services::telegram_new_api::TelegramApi {
            http: &state.http_client,
            base_url: &base,
        };
        delivery::TEST_BEFORE_COMMAND_CLAIM
            .scope(
                worker_barrier,
                delivery::deliver_command(&state, identity, delayed, &api),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait())
        .await
        .expect("management send barrier");
    crate::services::channel_bot_service::mark_bot_failed(&j.state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        bots.find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "failed"
    );
    let sent_before = j.sends().await.len();
    tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait())
        .await
        .expect("management send barrier");
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(30), send)
            .await
            .expect("management send completes after barrier release")
            .unwrap()
            .unwrap(),
        DeliveryOutcome::Refused(_)
    ));
    assert_eq!(
        j.sends().await.len(),
        sent_before,
        "stale prepared active status is never sent"
    );
    j.state.db.drop().await.unwrap();
}
