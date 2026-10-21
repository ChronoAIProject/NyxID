use super::*;
use crate::models::{
    platform_entry_identity::NativeEntryConfiguration,
    platform_manager_delivery::{DeliveryReference, DeliveryState},
    platform_manager_ingress::{IngressAudience, IngressPhase, PlatformManagerIngress},
};
use crate::services::platform_entry_native::tests as native;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[path = "platform_native_direct_tests.rs"]
mod direct;

async fn retain_admission(
    state: &AppState,
    bot: &ChannelBot,
    configuration: &NativeEntryConfiguration,
    profile: &str,
    adapter: &dyn PlatformAdapter,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> crate::services::platform_entry_receipts::NativeAdmission {
    let batch = crate::services::platform_entry_native::prepare(
        &state.db,
        &state.encryption_keys,
        adapter,
        bot,
        headers,
        body,
        configuration,
    )
    .await
    .unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let retained = crate::services::platform_entry_receipts::record_in_session(
        &state.db,
        &mut session,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        crate::services::platform_entry_receipts::NativeReceiptInput {
            profile_id: profile,
            epoch: 1,
            configuration,
            batch: &batch,
            index: 0,
            relay_message_id: &uuid::Uuid::new_v4().to_string(),
        },
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    retained
}

async fn retain(
    state: &AppState,
    bot: &ChannelBot,
    configuration: &NativeEntryConfiguration,
    profile: &str,
    adapter: &dyn PlatformAdapter,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> NativeEntryReceipt {
    retain_admission(state, bot, configuration, profile, adapter, headers, body)
        .await
        .receipt()
        .clone()
}

async fn claim(
    state: &AppState,
    receipt: &NativeEntryReceipt,
    adapter: &dyn PlatformAdapter,
) -> AppResult<NativeSendAuthority> {
    let now = Utc::now();
    let entry = StoredEntryIdentity::NativeV2 {
        platform_entry: receipt.configuration.identity.clone(),
    };
    let sender = StoredSenderIdentity::NativeV2 {
        platform_sender: receipt.sender.clone(),
    };
    let ingress = PlatformManagerIngress {
        id: uuid::Uuid::new_v4().to_string(),
        claim_id: uuid::Uuid::new_v4().to_string(),
        profile_id: receipt.profile_id.clone(),
        manager_bot_id: receipt.configuration.identity.platform_bot_id.clone(),
        entry: entry.clone(),
        sender: sender.clone(),
        event: StoredEventIdentity::NativeV2 {
            platform_event_id: receipt.event_id.clone(),
            platform_message_id: receipt.message_id.clone(),
        },
        manager_authority_epoch: 1,
        observation_id: receipt.id.clone(),
        original_private_command: true,
        audience: IngressAudience::Control,
        phase: IngressPhase::Admitted,
        execution: None,
        decision: None,
        reply_id: None,
        control_effect: None,
        delivery_recovery_queued: false,
        interrupted_reason: None,
        effect_revision: 0,
        admitted_at: now,
        start_before: receipt.expires_at,
        settled_at: None,
    };
    let claim_id = uuid::Uuid::new_v4().to_string();
    let delivery = PlatformManagerDelivery {
        id: uuid::Uuid::new_v4().to_string(),
        dedupe_key: uuid::Uuid::new_v4().to_string(),
        profile_id: receipt.profile_id.clone(),
        manager_bot_id: receipt.configuration.identity.platform_bot_id.clone(),
        entry,
        sender,
        manager_authority_epoch: 1,
        ingress_id: Some(ingress.id.clone()),
        ingress_claim_id: Some(ingress.claim_id.clone()),
        origin: None,
        origin_turn_id: None,
        reference: DeliveryReference::Command {
            effect: Some(crate::models::platform_manager_ingress::ManagerCommandEffect::Help),
        },
        state: DeliveryState::Dispatching,
        claim_id: Some(claim_id.clone()),
        telegram_message_ids: vec![],
        platform_message_ids: vec![],
        expires_at: receipt.expires_at,
        retry_at: now,
        updated_at: now,
    };
    state
        .db
        .collection::<PlatformManagerIngress>(
            crate::models::platform_manager_ingress::COLLECTION_NAME,
        )
        .insert_one(&ingress)
        .await?;
    state
        .db
        .collection::<PlatformManagerDelivery>(DELIVERIES)
        .insert_one(&delivery)
        .await?;
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    let result = capture_in_session(
        state,
        &mut session,
        &delivery,
        &claim_id,
        &ingress,
        adapter,
        receipt.expires_at,
    )
    .await;
    match result {
        Ok(value) => {
            session.commit_transaction().await?;
            Ok(value)
        }
        Err(error) => {
            session.abort_transaction().await?;
            Err(error)
        }
    }
}

pub(crate) async fn discord_fixture(base: &str) -> (AppState, ChannelBot, NativeEntryReceipt) {
    let db = crate::test_utils::connect_transaction_test_database("native_discord_reply").await;
    let state = crate::test_utils::test_app_state(db.clone());
    crate::services::platform_entry_migration::ensure(&db)
        .await
        .unwrap();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
    let mut bot = native::bot();
    bot.platform = "discord".into();
    bot.platform_bot_id = "200".into();
    bot.app_id = Some("100".into());
    bot.public_key = Some(hex::encode(signing.verifying_key().as_bytes()));
    bot.bot_token_encrypted = state
        .encryption_keys
        .encrypt(b"synthetic-discord-bot-token")
        .await
        .unwrap();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &bot.user_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let profile = uuid::Uuid::new_v4().to_string();
    let mut row = bson::to_document(&bot).unwrap();
    row.insert("platform_profile_id", &profile);
    row.insert("platform_entry_version", 2);
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
    let adapter =
        crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(base);
    let mut configuration = native::configuration(
        &bot,
        NativeControlSurface::DiscordCommand {
            application_id: "100".into(),
            command_id: "300".into(),
            command_digest: crate::services::platform_entry_discord_controls::schema_digest(),
        },
    );
    configuration.credential_fingerprint = EntryCredentialSnapshot::load(&db, &adapter, &bot)
        .await
        .unwrap()
        .fingerprint()
        .unwrap();
    let now = Utc::now();
    db.collection::<crate::models::platform_bot_config::PlatformBotConfig>(
        crate::models::platform_bot_config::COLLECTION_NAME,
    )
    .insert_one(crate::models::platform_bot_config::PlatformBotConfig {
        id: profile.clone(),
        channel_bot_id: bot.id.clone(),
        manager_bot_id: bot.platform_bot_id.clone(),
        identity_version: Some(2),
        native_entry: Some(configuration.clone()),
        observation_id: configuration.credential_fingerprint.clone(),
        execution_profile:
            crate::models::platform_bot_config::PlatformExecutionProfile::NativeEntry,
        generation: 1,
        enabled: false,
        guide: None,
        funding_owner_user_id: uuid::Uuid::new_v4().to_string(),
        configured_by: bot.user_id.clone(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    })
    .await
    .unwrap();
    db.collection::<crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls>(
        crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
    )
    .insert_one(
        crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls {
            id: uuid::Uuid::new_v4().to_string(),
            platform_config_id: profile.clone(),
            contract_version: 2,
            manager_bot_id: bot.platform_bot_id.clone(),
            entry: StoredEntryIdentity::NativeV2 {
                platform_entry: configuration.identity.clone(),
            },
            manager_authority_epoch: 1,
            personal_control_revision: 1,
            public_policy_revision: 1,
            personal_enabled: true,
            public_enabled: false,
            emergency_stopped: false,
            remembered_personal_enabled: None,
            remembered_public_enabled: None,
            retired: false,
            public_policy: None,
            effect_revision: 0,
            cleanup_pending: false,
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .unwrap();
    let body = serde_json::to_vec(&native::discord_message("help", None)).unwrap();
    let headers = native::discord_headers(&signing, &body, Utc::now().timestamp());
    crate::services::platform_entry_native::authenticate_discord_ack(
        &db, &adapter, &bot.id, &headers, &body,
    )
    .await
    .unwrap();
    let receipt = retain(
        &state,
        &bot,
        &configuration,
        &profile,
        &adapter,
        &headers,
        &body,
    )
    .await;
    (state, bot, receipt)
}

#[test]
fn signed_discord_receipt_claim_uses_only_its_encrypted_interaction_followup() {
    crate::test_utils::run_native_journey_test(
        "signed_discord_receipt_claim_uses_only_its_encrypted_interaction_followup",
        || async {
            let server = MockServer::start().await;
            let (state, _, receipt) = discord_fixture(&server.uri()).await;
            Mock::given(method("POST"))
                .and(path("/webhooks/100/synthetic-interaction-token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"id":"opaque-sent-id"})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let adapter =
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &server.uri(),
                );
            let authority = claim(&state, &receipt, &adapter).await.unwrap();
            assert!(!format!("{authority:?}").contains("synthetic-interaction"));
            let result = send_claimed_for_test(
                &state,
                authority,
                Zeroizing::new("Use /nyx status to check this chat.".into()),
                &adapter,
                &server.uri(),
            )
            .await
            .unwrap();
            assert!(
                matches!(result,NativeSendOutcome::Sent {platform_message_id} if platform_message_id=="opaque-sent-id")
            );
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            assert!(!requests[0].headers.contains_key("authorization"));
            assert_eq!(requests[0].url.query(), Some("wait=true"));
            let stored = state
                .db
                .collection::<NativeReplyHandle>(HANDLES)
                .find_one(doc! {"receipt_id":&receipt.id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                !serde_json::to_string(&stored)
                    .unwrap()
                    .contains("synthetic-interaction-token")
            );
            state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn missing_expired_substituted_and_overlong_discord_reply_handles_never_fall_back_to_channels() {
    crate::test_utils::run_native_journey_test(
        "missing_expired_substituted_and_overlong_discord_reply_handles_never_fall_back_to_channels",
        || async {
            for mode in [
                "missing",
                "expired",
                "substituted",
                "corrupt",
                "long-unicode",
            ] {
                let server = MockServer::start().await;
                let (state, _, receipt) = discord_fixture(&server.uri()).await;
                let adapter =
                    crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                        &server.uri(),
                    );
                let rows = state.db.collection::<bson::Document>(HANDLES);
                match mode {
                    "missing" => {
                        rows.delete_one(doc! {"receipt_id":&receipt.id})
                            .await
                            .unwrap();
                    }
                    "expired" => {
                        rows.update_one(doc! {"receipt_id":&receipt.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
                    }
                    "substituted" => {
                        rows.update_one(
                            doc! {"receipt_id":&receipt.id},
                            doc! {"$set":{"binding_digest":"00".repeat(32)}},
                        )
                        .await
                        .unwrap();
                    }
                    "corrupt" => {
                        rows.update_one(doc! {"receipt_id":&receipt.id},doc! {"$set":{"ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![7;32]}}}).await.unwrap();
                    }
                    _ => {}
                }
                match claim(&state, &receipt, &adapter).await {
                    Err(_) => assert!(matches!(mode, "missing" | "expired" | "substituted")),
                    Ok(authority) => {
                        let body = if mode == "long-unicode" {
                            "🦉".repeat(1001)
                        } else {
                            "Reply".into()
                        };
                        assert!(matches!(
                            send_claimed_for_test(
                                &state,
                                authority,
                                Zeroizing::new(body),
                                &adapter,
                                &server.uri()
                            )
                            .await
                            .unwrap(),
                            NativeSendOutcome::Refused
                        ));
                    }
                }
                assert!(
                    server.received_requests().await.unwrap().is_empty(),
                    "{mode}: no webhook or /channels fallback"
                );
                state.db.drop().await.unwrap();
            }
        },
    );
}

#[test]
fn captured_connection_token_never_adopts_same_id_successor_and_refresh_keeps_epoch_identity() {
    crate::test_utils::run_native_journey_test(
        "captured_connection_token_never_adopts_same_id_successor_and_refresh_keeps_epoch_identity",
        || async {
            for replacement in [true, false] {
                let (state, bot, configuration, profile) = native::stored_x_fixture().await;
                let server = MockServer::start().await;
                let adapter = crate::services::channel_adapters::x::XAdapter {
                    api_base: Some(server.uri()),
                };
                let body = native::stored_x_message(&bot, "help");
                let receipt = retain(
                    &state,
                    &bot,
                    &configuration,
                    &profile,
                    &adapter,
                    &native::x_headers(&body),
                    &body,
                )
                .await;
                let authority = claim(&state, &receipt, &adapter).await.unwrap();
                let successor = state
                    .encryption_keys
                    .encrypt(b"synthetic-successor-token")
                    .await
                    .unwrap();
                let rows = state
                    .db
                    .collection::<crate::models::user_api_key::UserApiKey>(
                        crate::models::user_api_key::COLLECTION_NAME,
                    );
                let epoch = if replacement { 2_i64 } else { 1_i64 };
                rows.update_one(doc! {"_id":bot.connection_id.as_ref().unwrap()},vec![doc! {"$set":{
            "credential_epoch":if replacement {bson::Bson::Document(doc! {"$add":[{"$ifNull":["$credential_epoch",1_i64]},1_i64]})}else{bson::Bson::Int64(1)},
            "access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:successor},"updated_at":bson::DateTime::now(),
        }}]).await.unwrap();
                assert_eq!(
                    rows.find_one(doc! {"_id":bot.connection_id.as_ref().unwrap()})
                        .await
                        .unwrap()
                        .unwrap()
                        .credential_epoch,
                    epoch
                );
                Mock::given(method("POST"))
                    .and(path("/2/dm_conversations/2-10/messages"))
                    .and(header("authorization", "Bearer synthetic-user-token"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(serde_json::json!({"data":{"dm_event_id":"900"}})),
                    )
                    .expect(1)
                    .mount(&server)
                    .await;
                assert!(matches!(
                    send_claimed_for_test(
                        &state,
                        authority,
                        Zeroizing::new("Hello".into()),
                        &adapter,
                        &server.uri()
                    )
                    .await
                    .unwrap(),
                    NativeSendOutcome::Sent { .. }
                ));
                assert_eq!(server.received_requests().await.unwrap().len(), 1);
                let current = EntryCredentialSnapshot::load(&state.db, &adapter, &bot)
                    .await
                    .unwrap();
                assert_eq!(
                    current.fingerprint().unwrap() == configuration.credential_fingerprint,
                    !replacement,
                    "ordinary token refresh keeps the admitted identity"
                );
                state.db.drop().await.unwrap();
            }
        },
    );
}

#[test]
fn token_expired_after_ingress_refreshes_before_final_claim_and_epoch_change_refuses() {
    crate::test_utils::run_native_journey_test(
        "token_expired_after_ingress_refreshes_before_final_claim_and_epoch_change_refuses",
        || async {
            use wiremock::matchers::body_string_contains;
            for replace in [false, true] {
                let (state, bot, configuration, profile) = native::stored_x_fixture().await;
                let server = MockServer::start().await;
                let adapter = crate::services::channel_adapters::x::XAdapter {
                    api_base: Some(server.uri()),
                };
                let body = native::stored_x_message(&bot, "help");
                let receipt = retain(
                    &state,
                    &bot,
                    &configuration,
                    &profile,
                    &adapter,
                    &native::x_headers(&body),
                    &body,
                )
                .await;
                state
                    .db
                    .collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
                    .update_one(
                        doc! {"slug":"twitter"},
                        doc! {"$set":{"token_url":format!("{}/token",server.uri())}},
                    )
                    .await
                    .unwrap();
                let refresh = state
                    .encryption_keys
                    .encrypt(b"synthetic-refresh-token")
                    .await
                    .unwrap();
                let rows = state
                    .db
                    .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME);
                rows.update_one(doc! {"_id":bot.connection_id.as_ref().unwrap()},doc! {"$set":{
            "expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(1)),
            "refresh_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:refresh},
        }}).await.unwrap();
                if replace {
                    rows.update_one(doc! {"_id":bot.connection_id.as_ref().unwrap()},vec![doc! {"$set":{"credential_epoch":{"$add":[{"$ifNull":["$credential_epoch",1_i64]},1_i64]}}}]).await.unwrap();
                }
                let scopes = crate::services::channel_adapters::x::REQUIRED_SCOPES
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>()
                    .join("  ");
                Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"synthetic-refreshed-send-token","refresh_token":"synthetic-successor-refresh","expires_in":3600,"scope":scopes})))
            .expect(if replace {0}else{1}).mount(&server).await;
                let result =
                    refresh_before_claim(&state, &receipt, receipt.expires_at, &adapter).await;
                if replace {
                    assert!(result.is_err());
                    assert!(
                        server.received_requests().await.unwrap().is_empty(),
                        "changed epoch refuses before OAuth or chat send"
                    );
                } else {
                    result.unwrap();
                    let authority = claim(&state, &receipt, &adapter).await.unwrap();
                    Mock::given(method("POST"))
                        .and(path("/2/dm_conversations/2-10/messages"))
                        .and(header(
                            "authorization",
                            "Bearer synthetic-refreshed-send-token",
                        ))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .set_body_json(serde_json::json!({"data":{"dm_event_id":"901"}})),
                        )
                        .expect(1)
                        .mount(&server)
                        .await;
                    assert!(matches!(
                        send_claimed_for_test(
                            &state,
                            authority,
                            Zeroizing::new("Hello".into()),
                            &adapter,
                            &server.uri()
                        )
                        .await
                        .unwrap(),
                        NativeSendOutcome::Sent { .. }
                    ));
                    let current = state
                        .db
                        .collection::<crate::models::user_api_key::UserApiKey>(
                            crate::models::user_api_key::COLLECTION_NAME,
                        )
                        .find_one(doc! {"_id":bot.connection_id.as_ref().unwrap()})
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(current.credential_epoch, 1);
                    assert_eq!(server.received_requests().await.unwrap().len(), 2);
                }
                state.db.drop().await.unwrap();
            }
        },
    );
}

#[derive(Debug)]
struct StalledKeyProvider;
#[async_trait::async_trait]
impl crate::crypto::key_provider::KeyProvider for StalledKeyProvider {
    async fn wrap_dek(
        &self,
        _: &[u8],
    ) -> Result<crate::crypto::key_provider::WrappedKey, crate::errors::AppError> {
        std::future::pending().await
    }
    async fn unwrap_dek(
        &self,
        _: &crate::crypto::key_provider::WrappedKey,
    ) -> Result<Zeroizing<Vec<u8>>, crate::errors::AppError> {
        std::future::pending().await
    }
    fn current_key_id(&self) -> u8 {
        0
    }
    fn has_key_id(&self, _: u8) -> bool {
        true
    }
    fn has_previous_key(&self) -> bool {
        false
    }
}

#[test]
fn stalled_discord_and_connection_decryption_are_bounded_without_late_provider_send() {
    crate::test_utils::run_native_journey_test(
        "stalled_discord_and_connection_decryption_are_bounded_without_late_provider_send",
        || async {
            for discord in [true, false] {
                let server = MockServer::start().await;
                let (mut state, mut authority, adapter): (
                    AppState,
                    NativeSendAuthority,
                    Box<dyn PlatformAdapter>,
                ) = if discord {
                    let (state, _, receipt) = discord_fixture(&server.uri()).await;
                    let adapter =
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &server.uri(),
                );
                    let authority = claim(&state, &receipt, &adapter).await.unwrap();
                    (state, authority, Box::new(adapter))
                } else {
                    let (state, bot, config, profile) = native::stored_x_fixture().await;
                    let adapter = crate::services::channel_adapters::x::XAdapter {
                        api_base: Some(server.uri()),
                    };
                    let body = native::stored_x_message(&bot, "help");
                    let receipt = retain(
                        &state,
                        &bot,
                        &config,
                        &profile,
                        &adapter,
                        &native::x_headers(&body),
                        &body,
                    )
                    .await;
                    let authority = claim(&state, &receipt, &adapter).await.unwrap();
                    (state, authority, Box::new(adapter))
                };
                authority.deadline = Utc::now() + Duration::milliseconds(150);
                state.encryption_keys =
                    std::sync::Arc::new(crate::crypto::aes::EncryptionKeys::with_provider(
                        std::sync::Arc::new(StalledKeyProvider),
                    ));
                let started = std::time::Instant::now();
                let outcome = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    send_claimed_for_test(
                        &state,
                        authority,
                        Zeroizing::new("Hello".into()),
                        adapter.as_ref(),
                        &server.uri(),
                    ),
                )
                .await
                .expect("entire decrypt and send lifetime is bounded")
                .unwrap();
                assert!(matches!(outcome, NativeSendOutcome::Refused));
                assert!(started.elapsed() < std::time::Duration::from_secs(2));
                tokio::task::yield_now().await;
                assert!(server.received_requests().await.unwrap().is_empty());
                state.db.drop().await.unwrap();
            }
        },
    );
}

struct NativeSettledHook;
#[async_trait::async_trait]
impl crate::services::assistant_nyxagent_runtime::OwnerTurnSettledHook for NativeSettledHook {
    async fn settled(
        &self,
        _: crate::models::assistant_conversation::AssistantConversation,
        _: crate::services::platform_manager_contract::ReplyRef,
    ) {
    }
}

async fn committed_discord_help(
    state: &AppState,
    bot: &ChannelBot,
    original: &NativeEntryReceipt,
    adapter: &dyn PlatformAdapter,
) -> (
    NativeEntryReceipt,
    crate::services::platform_manager_command_service::CommittedCommand,
) {
    use crate::services::{
        platform_manager_command_service as commands,
        platform_manager_contract::{OriginalManagerCommand, TurnInput},
        platform_personal_binding_service::LifecycleProducer,
    };
    crate::services::platform_manager_ingress_service::ensure_indexes(&state.db)
        .await
        .unwrap();
    crate::services::platform_manager_delivery::ensure_indexes(&state.db)
        .await
        .unwrap();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
    let mut interaction = native::discord_message("help", None);
    // The lower snowflake bits distinguish this interaction from fixture setup.
    let id: u64 = interaction["id"].as_str().unwrap().parse().unwrap();
    interaction["id"] = serde_json::json!((id + 1).to_string());
    let body = serde_json::to_vec(&interaction).unwrap();
    let headers = native::discord_headers(&signing, &body, Utc::now().timestamp());
    let admission = retain_admission(
        state,
        bot,
        &original.configuration,
        &original.profile_id,
        adapter,
        &headers,
        &body,
    )
    .await;
    let command = OriginalManagerCommand::from_verified(
        admission.verified_update(),
        TurnInput {
            channel_uploads: Vec::new(),
            text: "/help".into(),
        },
    )
    .unwrap();
    let producers = crate::services::platform_manager_transport::compose(
        state.clone(),
        std::sync::Arc::new(NativeSettledHook),
    );
    let lifecycle = LifecycleProducer {
        personal: producers.personal.clone(),
        public: producers.public.clone(),
    };
    let committed = commands::execute(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        state.audit_chain_hmac_key.as_slice(),
        &command,
        commands::PersonalCommandKind::Help,
        &lifecycle,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        commands::execute(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            state.audit_chain_hmac_key.as_slice(),
            &command,
            commands::PersonalCommandKind::Help,
            &lifecycle,
        )
        .await
        .unwrap()
        .is_none()
    );
    (admission.receipt().clone(), committed)
}

#[test]
fn signed_discord_command_uses_real_admission_and_final_delivery_claim_once() {
    crate::test_utils::run_native_journey_test(
        "signed_discord_command_uses_real_admission_and_final_delivery_claim_once",
        || async {
            use crate::services::{
                platform_manager_contract::DeliveryOutcome, platform_manager_delivery as delivery,
                platform_personal_binding_service::IdentityProducer,
            };
            let server = MockServer::start().await;
            let base = server.uri();
            let (state, bot, original) = discord_fixture(&base).await;
            let adapter = std::sync::Arc::new(
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &base,
                ),
            );
            let (receipt, command) =
                committed_discord_help(&state, &bot, &original, adapter.as_ref()).await;
            let ingress = command.ingress.clone();
            let expected = command
                .reply
                .render(&state.config.frontend_url, &command.manager.entry)
                .unwrap();
            Mock::given(method("POST"))
                .and(path("/webhooks/100/synthetic-interaction-token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"id":"native-command-id"})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let transport = std::sync::Arc::new(NativeTestTransport {
                adapter,
                discord_base: base.clone(),
            });
            let api = crate::services::telegram_new_api::TelegramApi {
                http: &state.http_client,
                base_url: &base,
            };
            let outcome = with_transport_for_test(
                transport,
                delivery::deliver_command(
                    &state,
                    std::sync::Arc::new(IdentityProducer),
                    command,
                    &api,
                ),
            )
            .await
            .unwrap();
            assert!(
                matches!(outcome, DeliveryOutcome::NativeSent { platform_message_ids }
        if platform_message_ids == ["native-command-id"])
            );
            let stored = state
                .db
                .collection::<PlatformManagerDelivery>(DELIVERIES)
                .find_one(doc! {"ingress_id":&ingress.receipt_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.state, DeliveryState::Sent);
            assert!(stored.claim_id.is_some());
            assert_eq!(stored.platform_message_ids, ["native-command-id"]);
            assert!(stored.telegram_message_ids.is_empty());
            let stored_ingress = state
                .db
                .collection::<PlatformManagerIngress>(
                    crate::models::platform_manager_ingress::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&ingress.receipt_id,"claim_id":&ingress.claim_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored_ingress.observation_id, receipt.id);
            assert_eq!(stored_ingress.phase, IngressPhase::Settled);
            let producers = crate::services::platform_manager_transport::compose(
                state.clone(),
                std::sync::Arc::new(NativeSettledHook),
            );
            assert_eq!(
                delivery::sweep(
                    &state,
                    producers.identity.clone(),
                    producers.public.clone(),
                    10
                )
                .await
                .unwrap()
                .processed,
                0
            );
            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests.len(),
                1,
                "every possible provider path uses this server"
            );
            assert_eq!(
                requests[0].url.path(),
                "/webhooks/100/synthetic-interaction-token"
            );
            assert_eq!(requests[0].url.query(), Some("wait=true"));
            assert!(!requests[0].headers.contains_key("authorization"));
            let posted: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(posted["content"], expected.as_str());
            assert!(expected.contains("/nyx login"));
            assert!(expected.contains("/nyx ask question:"));
            state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn real_discord_command_refuses_missing_or_expired_handle_without_generic_send() {
    crate::test_utils::run_native_journey_test(
        "real_discord_command_refuses_missing_or_expired_handle_without_generic_send",
        || async {
            use crate::services::{
                platform_manager_contract::DeliveryOutcome, platform_manager_delivery as delivery,
                platform_personal_binding_service::IdentityProducer,
            };
            for missing in [true, false] {
                let server = MockServer::start().await;
                let base = server.uri();
                let (state, bot, original) = discord_fixture(&base).await;
                let adapter = std::sync::Arc::new(
                    crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                        &base,
                    ),
                );
                let (receipt, command) =
                    committed_discord_help(&state, &bot, &original, adapter.as_ref()).await;
                let ingress = command.ingress.clone();
                let handles = state.db.collection::<NativeReplyHandle>(HANDLES);
                if missing {
                    assert_eq!(
                        handles
                            .delete_one(doc! {"receipt_id":&receipt.id})
                            .await
                            .unwrap()
                            .deleted_count,
                        1
                    );
                } else {
                    assert_eq!(
                handles.update_one(doc! {"receipt_id":&receipt.id},
                    doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}})
                    .await.unwrap().matched_count,
                1
            );
                }
                let transport = std::sync::Arc::new(NativeTestTransport {
                    adapter,
                    discord_base: base.clone(),
                });
                let api = crate::services::telegram_new_api::TelegramApi {
                    http: &state.http_client,
                    base_url: &base,
                };
                let outcome = with_transport_for_test(
                    transport,
                    delivery::deliver_command(
                        &state,
                        std::sync::Arc::new(IdentityProducer),
                        command,
                        &api,
                    ),
                )
                .await
                .unwrap();
                assert!(matches!(outcome, DeliveryOutcome::Refused(_)));
                let row = state
                    .db
                    .collection::<PlatformManagerDelivery>(DELIVERIES)
                    .find_one(doc! {"ingress_id":&ingress.receipt_id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(row.state, DeliveryState::Refused);
                assert!(row.platform_message_ids.is_empty());
                assert!(server.received_requests().await.unwrap().is_empty());
                state.db.drop().await.unwrap();
            }
        },
    );
}

#[test]
fn native_final_claim_and_terminal_persistence_share_one_deadline_without_resend() {
    crate::test_utils::run_native_journey_test(
        "native_final_claim_and_terminal_persistence_share_one_deadline_without_resend",
        || async {
            use crate::services::{
                platform_manager_contract::DeliveryOutcome,
                platform_manager_delivery::{self as delivery, NativeDeliveryStall},
                platform_personal_binding_service::IdentityProducer,
            };
            use std::{
                sync::{
                    Arc,
                    atomic::{AtomicBool, Ordering},
                },
                time::Duration,
            };
            for after_provider in [false, true] {
                let server = MockServer::start().await;
                let base = server.uri();
                let (state, bot, original) = discord_fixture(&base).await;
                let adapter = Arc::new(
                    crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                        &base,
                    ),
                );
                let (_, command) =
                    committed_discord_help(&state, &bot, &original, adapter.as_ref()).await;
                let ingress = command.ingress.clone();
                Mock::given(method("POST"))
                    .and(path("/webhooks/100/synthetic-interaction-token"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(
                        serde_json::json!({"id":"confirmed-before-lost-settlement"}),
                    ))
                    .expect(u64::from(after_provider))
                    .mount(&server)
                    .await;
                let transport = Arc::new(NativeTestTransport {
                    adapter,
                    discord_base: base.clone(),
                });
                let stall = Arc::new(NativeDeliveryStall {
                    entered: AtomicBool::new(false),
                    duration: Duration::from_secs(10),
                });
                let api = crate::services::telegram_new_api::TelegramApi {
                    http: &state.http_client,
                    base_url: &base,
                };
                let work = with_transport_for_test(
                    transport,
                    delivery::TEST_NATIVE_SEND_BUDGET.scope(
                        Duration::from_secs(3),
                        delivery::deliver_command(
                            &state,
                            Arc::new(IdentityProducer),
                            command,
                            &api,
                        ),
                    ),
                );
                let started = std::time::Instant::now();
                let outcome = tokio::time::timeout(Duration::from_secs(8), async {
                    if after_provider {
                        delivery::TEST_NATIVE_FINISH_STALL
                            .scope(stall.clone(), work)
                            .await
                    } else {
                        delivery::TEST_NATIVE_CLAIM_STALL
                            .scope(stall.clone(), work)
                            .await
                    }
                })
                .await
                .expect("native caller, claim and final persistence have a bounded lifetime")
                .unwrap();
                assert!(
                    stall.entered.load(Ordering::SeqCst),
                    "the selected actual claim/persistence stage was reached"
                );
                assert!(started.elapsed() < Duration::from_secs(6));
                assert!(if after_provider {
                    matches!(outcome, DeliveryOutcome::Uncertain)
                } else {
                    matches!(outcome, DeliveryOutcome::Refused(_))
                });
                let row = state
                    .db
                    .collection::<PlatformManagerDelivery>(DELIVERIES)
                    .find_one(doc! {"ingress_id":&ingress.receipt_id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    row.state,
                    if after_provider {
                        DeliveryState::Dispatching
                    } else {
                        DeliveryState::Pending
                    }
                );
                assert!(
                    row.platform_message_ids.is_empty(),
                    "no unconfirmed durable Sent projection"
                );
                state.db.collection::<PlatformManagerDelivery>(DELIVERIES).update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"updated_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(3))}},
        ).await.unwrap();
                let producers = crate::services::platform_manager_transport::compose(
                    state.clone(),
                    Arc::new(NativeSettledHook),
                );
                assert_eq!(
                    delivery::sweep(
                        &state,
                        producers.identity.clone(),
                        producers.public.clone(),
                        10
                    )
                    .await
                    .unwrap()
                    .processed,
                    1
                );
                let recovered = state
                    .db
                    .collection::<PlatformManagerDelivery>(DELIVERIES)
                    .find_one(doc! {"_id":&row.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    recovered.state,
                    if after_provider {
                        DeliveryState::Uncertain
                    } else {
                        DeliveryState::Refused
                    }
                );
                tokio::time::sleep(Duration::from_millis(30)).await;
                let requests = server.received_requests().await.unwrap();
                assert_eq!(requests.len(), usize::from(after_provider));
                assert!(requests.iter().all(
                    |request| request.url.path() == "/webhooks/100/synthetic-interaction-token"
                ));
                state.db.drop().await.unwrap();
            }
        },
    );
}

#[test]
fn native_preparation_failure_cleanup_is_bounded_and_never_sends_late() {
    crate::test_utils::run_native_journey_test(
        "native_preparation_failure_cleanup_is_bounded_and_never_sends_late",
        || async {
            use crate::services::{
                platform_manager_contract::DeliveryOutcome,
                platform_manager_delivery::{self as delivery, NativeDeliveryStall},
                platform_personal_binding_service::IdentityProducer,
            };
            use std::{
                sync::{
                    Arc,
                    atomic::{AtomicBool, Ordering},
                },
                time::Duration,
            };
            let server = MockServer::start().await;
            let base = server.uri();
            let (state, bot, original) = discord_fixture(&base).await;
            let adapter = Arc::new(
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &base,
                ),
            );
            let (receipt, command) =
                committed_discord_help(&state, &bot, &original, adapter.as_ref()).await;
            let ingress = command.ingress.clone();
            // The command is committed, then its retained transport evidence disappears.
            // Preparation must refuse; its failure cleanup must use the same deadline.
            assert_eq!(
                state
                    .db
                    .collection::<NativeEntryReceipt>(
                        crate::models::platform_entry_identity::NATIVE_RECEIPTS,
                    )
                    .delete_one(doc! {"_id":&receipt.id})
                    .await
                    .unwrap()
                    .deleted_count,
                1
            );
            let transport = Arc::new(NativeTestTransport {
                adapter,
                discord_base: base.clone(),
            });
            let stall = Arc::new(NativeDeliveryStall {
                entered: AtomicBool::new(false),
                duration: Duration::from_secs(10),
            });
            let api = crate::services::telegram_new_api::TelegramApi {
                http: &state.http_client,
                base_url: &base,
            };
            let started = std::time::Instant::now();
            let outcome = tokio::time::timeout(
                Duration::from_secs(8),
                with_transport_for_test(
                    transport,
                    delivery::TEST_NATIVE_SEND_BUDGET.scope(
                        Duration::from_secs(3),
                        delivery::TEST_NATIVE_CLEANUP_STALL.scope(
                            stall.clone(),
                            delivery::deliver_command(
                                &state,
                                Arc::new(IdentityProducer),
                                command,
                                &api,
                            ),
                        ),
                    ),
                ),
            )
            .await
            .expect("failure cleanup cannot extend the held send permit indefinitely")
            .unwrap();
            assert!(
                stall.entered.load(Ordering::SeqCst),
                "the failure cleanup was actually reached"
            );
            assert!(started.elapsed() < Duration::from_secs(6));
            assert!(matches!(outcome, DeliveryOutcome::Refused(_)));
            let row = state
                .db
                .collection::<PlatformManagerDelivery>(DELIVERIES)
                .find_one(doc! {"ingress_id":&ingress.receipt_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.state,
                DeliveryState::Pending,
                "failed cleanup leaves the exact durable row recoverable"
            );
            let producers = crate::services::platform_manager_transport::compose(
                state.clone(),
                Arc::new(NativeSettledHook),
            );
            assert_eq!(
                delivery::sweep(
                    &state,
                    producers.identity.clone(),
                    producers.public.clone(),
                    10
                )
                .await
                .unwrap()
                .processed,
                1
            );
            let recovered = state
                .db
                .collection::<PlatformManagerDelivery>(DELIVERIES)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(recovered.state, DeliveryState::Refused);
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(server.received_requests().await.unwrap().is_empty());
            state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn retained_signed_native_receipt_produces_real_login_status_and_manager_admission() {
    crate::test_utils::run_native_journey_test(
        "retained_signed_native_receipt_produces_real_login_status_and_manager_admission",
        || async {
            use crate::services::{
                platform_manager_command_service as commands,
                platform_manager_contract::{
                    IngressAdmission, ManagerAudience, OriginalManagerCommand, TurnInput,
                    UnavailableReason,
                },
                platform_personal_binding_service::{IdentityProducer, LifecycleProducer},
            };
            let server = MockServer::start().await;
            let (state, bot, original) = discord_fixture(&server.uri()).await;
            crate::services::platform_manager_ingress_service::ensure_indexes(&state.db)
                .await
                .unwrap();
            crate::services::platform_personal_binding_service::ensure_indexes(&state.db)
                .await
                .unwrap();
            crate::services::platform_link_intent_service::ensure_indexes(&state.db)
                .await
                .unwrap();
            let producers = crate::services::platform_manager_transport::compose(
                state.clone(),
                std::sync::Arc::new(NativeSettledHook),
            );
            let lifecycle = LifecycleProducer {
                personal: producers.personal.clone(),
                public: producers.public.clone(),
            };
            let signing = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
            let adapter =
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &server.uri(),
                );
            for (selected, argument, canonical) in [
                ("status", None, "/status"),
                ("login", None, "/login"),
                ("status", None, "/status"),
                (
                    "ask",
                    Some(("question", "What is NyxID?")),
                    "What is NyxID?",
                ),
            ] {
                let body =
                    serde_json::to_vec(&native::discord_message(selected, argument)).unwrap();
                let headers = native::discord_headers(&signing, &body, Utc::now().timestamp());
                let admitted = retain_admission(
                    &state,
                    &bot,
                    &original.configuration,
                    &original.profile_id,
                    &adapter,
                    &headers,
                    &body,
                )
                .await;
                let update = admitted.verified_update();
                assert_eq!(update.observation_id, admitted.receipt().id);
                assert_ne!(
                    update.observation_id,
                    admitted.receipt().configuration.credential_fingerprint
                );
                if let Some(kind) = commands::personal_command(canonical, "") {
                    let command = OriginalManagerCommand::from_verified(
                        update.clone(),
                        TurnInput {
                            channel_uploads: Vec::new(),
                            text: canonical.into(),
                        },
                    )
                    .unwrap();
                    let result = commands::execute(
                        &state.db,
                        state.auth_device_hmac_key.as_slice(),
                        state.audit_chain_hmac_key.as_slice(),
                        &command,
                        kind.clone(),
                        &lifecycle,
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    match (selected, &result.reply) {
                        ("login", commands::PersonalCommandReply::Link { .. }) => {}
                        ("status", commands::PersonalCommandReply::Status(_)) => {}
                        _ => panic!("native command did not reach its real producer"),
                    }
                    assert!(
                        commands::execute(
                            &state.db,
                            state.auth_device_hmac_key.as_slice(),
                            state.audit_chain_hmac_key.as_slice(),
                            &command,
                            kind,
                            &lifecycle
                        )
                        .await
                        .unwrap()
                        .is_none()
                    );
                    let stored = state
                        .db
                        .collection::<PlatformManagerIngress>(
                            crate::models::platform_manager_ingress::COLLECTION_NAME,
                        )
                        .find_one(doc! {"_id":result.ingress.receipt_id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(stored.observation_id, admitted.receipt().id);
                    assert_eq!(stored.entry, update.entry);
                } else {
                    let result =
                        crate::services::platform_manager_dispatch_service::admit_manager_update(
                            &state.db,
                            &IdentityProducer,
                            &update,
                        )
                        .await
                        .unwrap();
                    assert!(matches!(
                        result,
                        IngressAdmission::Claimed {
                            audience: ManagerAudience::Fixed(UnavailableReason::PublicDisabled),
                            ..
                        }
                    ));
                    assert!(matches!(
                        crate::services::platform_manager_dispatch_service::admit_manager_update(
                            &state.db,
                            &IdentityProducer,
                            &update
                        )
                        .await
                        .unwrap(),
                        IngressAdmission::Duplicate { .. }
                    ));
                }
            }
            assert!(server.received_requests().await.unwrap().is_empty());
            state.db.drop().await.unwrap();
        },
    );
}
