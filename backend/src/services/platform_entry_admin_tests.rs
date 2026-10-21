use super::*;
use std::sync::Arc;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

pub(crate) struct NoPublic;
pub(crate) struct Settled;
#[async_trait::async_trait]
impl super::super::assistant_nyxagent_runtime::OwnerTurnSettledHook for Settled {
    async fn settled(
        &self,
        _: crate::models::assistant_conversation::AssistantConversation,
        _: super::super::platform_manager_contract::ReplyRef,
    ) {
    }
}

pub(crate) async fn execute_command(
    state: &AppState,
    admission: &super::super::platform_entry_receipts::NativeAdmission,
    kind: super::super::platform_manager_command_service::PersonalCommandKind,
) -> AppResult<Option<super::super::platform_manager_command_service::CommittedCommand>> {
    use super::super::platform_manager_contract::{OriginalManagerCommand, TurnInput};
    let command = OriginalManagerCommand::from_verified(
        admission.verified_update(),
        TurnInput {
            channel_uploads: Vec::new(),
            text: admission.text().into(),
        },
    )
    .ok_or_else(refused)?;
    let producers =
        super::super::platform_manager_transport::compose(state.clone(), Arc::new(Settled));
    let lifecycle = super::super::platform_personal_binding_service::LifecycleProducer {
        personal: producers.personal.clone(),
        public: producers.public.clone(),
    };
    super::super::platform_manager_command_service::execute(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        state.audit_chain_hmac_key.as_slice(),
        &command,
        kind,
        &lifecycle,
    )
    .await
}

async fn x_admission(
    fixture: &Fixture,
    profile: &PlatformBotConfig,
    epoch: i64,
    text: &str,
    event: &str,
) -> super::super::platform_entry_receipts::NativeAdmission {
    let bot = super::super::channel_bot_service::get_bot(&fixture.state.db, &fixture.bot.id)
        .await
        .unwrap();
    let mut message: Value = serde_json::from_slice(
        &super::super::platform_entry_native::tests::stored_x_message(&bot, text),
    )
    .unwrap();
    message["data"]["payload"]["direct_message_events"][0]["id"] = event.into();
    let body = serde_json::to_vec(&message).unwrap();
    let configuration = profile.native_entry.as_ref().unwrap();
    let batch = super::super::platform_entry_native::prepare(
        &fixture.state.db,
        &fixture.state.encryption_keys,
        fixture.transport.adapter.as_ref(),
        &bot,
        &super::super::platform_entry_native::tests::x_headers(&body),
        &body,
        configuration,
    )
    .await
    .unwrap();
    let mut session = fixture.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let admission = super::super::platform_entry_receipts::record_in_session(
        &fixture.state.db,
        &mut session,
        &fixture.state.encryption_keys,
        fixture.state.auth_device_hmac_key.as_slice(),
        super::super::platform_entry_receipts::NativeReceiptInput {
            profile_id: &profile.id,
            epoch,
            configuration,
            batch: &batch,
            index: 0,
            relay_message_id: &uuid::Uuid::new_v4().to_string(),
        },
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    admission
}

#[async_trait::async_trait]
impl super::super::platform_bot_runtime_control_service::PublicControlReadiness for NoPublic {
    async fn probe(
        &self,
        _: &Database,
        _: &super::super::platform_manager_contract::ManagerRuntimePolicy,
    ) -> AppResult<super::super::platform_manager_contract::PublicReadinessEvidence> {
        panic!("personal enable needs no public probe")
    }
    async fn revoke_public_manager(
        &self,
        _: &Database,
        _: &mut ClientSession,
        _: &super::super::platform_manager_contract::ManagerRuntimePolicy,
    ) -> AppResult<()> {
        panic!("no public session")
    }
    async fn fence(
        &self,
        _: &Database,
        _: &mut ClientSession,
        _: &super::super::platform_manager_contract::ManagerRuntimePolicy,
        _: &super::super::platform_manager_contract::PublicReadinessEvidence,
    ) -> AppResult<()> {
        panic!("no public proof")
    }
}

struct Fixture {
    state: AppState,
    bot: ChannelBot,
    funding: String,
    server: MockServer,
    transport: Arc<super::super::platform_native_reply_service::NativeTestTransport>,
}

impl Fixture {
    async fn new() -> Self {
        let (mut state, bot, _, profile) =
            super::super::platform_entry_native::tests::stored_x_fixture().await;
        state
            .db
            .collection::<Document>(BOTS)
            .update_one(
                doc! {"_id":&bot.id},
                doc! {"$unset":{"platform_profile_id":"","platform_entry_version":""}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .delete_one(doc! {"_id":&profile})
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .delete_many(doc! {"platform_config_id":profile})
            .await
            .unwrap();
        super::super::platform_bot_config_service::ensure_indexes(&state.db)
            .await
            .unwrap();
        super::super::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let role = super::super::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap()
            .admin;
        state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&bot.user_id}, doc! {"$set":{"role_ids":[role]}})
            .await
            .unwrap();
        let funding = super::super::org_service::create_org_user(
            &state.db,
            "Native public funding",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
        super::super::org_service::create_membership(
            &state.db,
            &funding,
            &bot.user_id,
            crate::models::org_membership::OrgRole::Admin,
            crate::models::org_membership::MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        let billed_service =
            super::super::channel_x_tests::billing::enable_billing(&mut state, &bot.user_id).await;
        use crate::models::service_billing::{OperationPrice, PricingSyncStatus};
        use crate::services::channel_billing_service::XChannelOperation;
        let operations =
            [XChannelOperation::AccountVerify, XChannelOperation::DmSend].map(|operation| {
                OperationPrice {
                    operation: operation.key().into(),
                    credits_per_unit: "2".into(),
                    lago_metric_code: crate::services::billing::pricing::operation_metric_code(
                        "api-twitter",
                        "byok",
                        operation.key(),
                    )
                    .unwrap(),
                    sync_status: PricingSyncStatus::Synced,
                    sync_error: None,
                }
            });
        state.db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
            .update_one(doc! {"_id":&billed_service}, doc! {"$set":{"billing.byok_pricing.operations":bson::to_bson(&operations).unwrap()}}).await.unwrap();
        for operation in &operations {
            state.db.collection::<Document>(crate::models::billing_rate_cache::COLLECTION_NAME)
                .insert_one(doc! {"_id":format!("{}:*",operation.lago_metric_code),"lago_metric_code":&operation.lago_metric_code,
                    "credits_per_unit_micros":2_000_000_i64,"synced_at":bson::DateTime::now()}).await.unwrap();
        }
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/2/users/me"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"data":{"id":"10","username":"local-native"}}),
                ),
            )
            .mount(&server)
            .await;
        let transport = Arc::new(
            super::super::platform_native_reply_service::NativeTestTransport {
                adapter: Arc::new(super::super::channel_adapters::x::XAdapter {
                    api_base: Some(server.uri()),
                }),
                discord_base: server.uri(),
            },
        );
        Self {
            state,
            bot,
            funding,
            server,
            transport,
        }
    }

    async fn convert(
        &self,
        transport: NativeTransportChoice,
    ) -> AppResult<NativeConversionOutcome> {
        self.convert_as(&self.bot.user_id, &self.funding, transport)
            .await
    }

    async fn convert_as(
        &self,
        actor: &str,
        funding: &str,
        transport: NativeTransportChoice,
    ) -> AppResult<NativeConversionOutcome> {
        super::super::platform_native_reply_service::with_transport_for_test(
            self.transport.clone(),
            convert(
                &self.state,
                actor,
                &self.bot.id,
                funding,
                transport,
                None,
                None,
            ),
        )
        .await
    }

    async fn pending_marker(&self) -> Document {
        self.state
            .db
            .collection::<Document>(BOTS)
            .find_one(doc! {"_id":&self.bot.id})
            .await
            .unwrap()
            .unwrap()
            .get_document("platform_entry_pending")
            .unwrap()
            .clone()
    }
}

#[test]
fn actual_native_x_conversion_uses_billed_verification_with_original_transport_payer() {
    crate::test_utils::run_native_journey_test(
        "actual_native_x_conversion_uses_billed_verification_with_original_transport_payer",
        || async {
            let fixture = Fixture::new().await;
            let NativeConversionOutcome::Converted(profile) = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                fixture.convert(NativeTransportChoice::Direct),
            )
            .await
            .unwrap()
            .unwrap() else {
                panic!("direct conversion")
            };
            let rows = super::super::channel_x_tests::billing::settled(&fixture.state).await;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].billing_owner_id, fixture.bot.user_id);
            assert_eq!(rows[0].actor_user_id, fixture.bot.user_id);
            assert_eq!(
                rows[0].credential_class,
                crate::models::usage_meter::CredentialClass::NyxidPlatformOauthApp
            );
            assert_eq!(rows[0].operation.as_deref(), Some("get_me"));
            assert_eq!(rows[0].quantity, Some(1));
            assert!(rows[0].billing_request_id.starts_with("x-account-verify:"));
            let wallet: crate::models::billing_wallet::BillingWallet = fixture
                .state
                .db
                .collection(crate::models::billing_wallet::COLLECTION_NAME)
                .find_one(doc! {"owner_id":&fixture.bot.user_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                wallet.pending_lago_debits,
                crate::models::credits::Credits::from_whole(2)
            );
            assert_eq!(profile.funding_owner_user_id, fixture.funding);
            assert_eq!(profile.identity_version, Some(2));
            assert_eq!(
                profile.native_entry.as_ref().unwrap().transport,
                EntryTransport::Direct
            );
            assert_eq!(fixture.server.received_requests().await.unwrap().len(), 1);
            let change = super::super::platform_bot_runtime_control_service::RuntimeChange {
                public_deployment_digest: None,
                initialize: true,
                expected_profile_generation: Some(profile.generation),
                expected_manager_authority_epoch: 0,
                expected_personal_control_revision: 0,
                expected_public_policy_revision: 0,
                personal_enabled: Some(true),
                public_enabled: Some(false),
                emergency_stopped: None,
                public_policy: None,
            };
            let actor = super::super::audit_service::AuditActor::from_auth_user(
                &crate::test_utils::test_auth_user(&fixture.bot.user_id),
            );
            let controls = super::super::platform_bot_runtime_control_service::update(
                &fixture.state.db,
                &actor,
                b"native-runtime-test-audit",
                &profile.id,
                &change,
                &NoPublic,
            )
            .await
            .unwrap();
            assert_eq!(controls.contract_version, 2);
            assert!(controls.personal_enabled);
            assert!(!controls.public_enabled);
            assert_eq!(
                fixture
                    .state
                    .db
                    .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
                    .count_documents(doc! {"provider":"telegram-new"})
                    .await
                    .unwrap(),
                0
            );
            fixture.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn native_x_conversion_refuses_unfunded_verification_without_provider_effect_and_keeps_attempt() {
    crate::test_utils::run_native_journey_test(
        "native_x_conversion_refuses_unfunded_verification_without_provider_effect_and_keeps_attempt",
        || async {
            let fixture = Fixture::new().await;
            fixture
                .state
                .db
                .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .update_one(
                    doc! {"owner_id":&fixture.bot.user_id},
                    doc! {"$set":{"balance_credits":crate::models::credits::Credits::ZERO}},
                )
                .await
                .unwrap();
            assert!(matches!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    fixture.convert(NativeTransportChoice::Direct)
                )
                .await
                .unwrap(),
                Err(AppError::InsufficientCredits)
            ));
            assert!(fixture.server.received_requests().await.unwrap().is_empty());
            let marker = fixture.pending_marker().await;
            assert_eq!(marker.get_str("actor").unwrap(), fixture.bot.user_id);
            assert_eq!(marker.get_str("funding").unwrap(), fixture.funding);
            assert_eq!(marker.get_str("transport").unwrap(), "direct");
            assert!(
                super::super::channel_x_tests::billing::settled(&fixture.state)
                    .await
                    .is_empty()
            );
            fixture.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn funding_revocation_after_pending_claim_refuses_paid_verification_and_retains_exact_attempt() {
    crate::test_utils::run_native_journey_test(
        "funding_revocation_after_pending_claim_refuses_paid_verification_and_retains_exact_attempt",
        || async {
            for change in ["membership", "organization"] {
                let fixture = Fixture::new().await;
                let barrier = Arc::new(tokio::sync::Barrier::new(2));
                let run = BEFORE_PROVIDER_VERIFY.scope(
                    barrier.clone(),
                    fixture.convert(NativeTransportChoice::Direct),
                );
                let mutate = async {
                    tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait())
                        .await
                        .unwrap();
                    let before = fixture.pending_marker().await;
                    if change == "membership" {
                        fixture
                    .state
                    .db
                    .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                    .update_one(
                        doc! {"org_user_id":&fixture.funding,"member_user_id":&fixture.bot.user_id},
                        doc! {"$set":{"revoked_at":bson::DateTime::now()}},
                    )
                    .await
                    .unwrap();
                    } else {
                        fixture
                            .state
                            .db
                            .collection::<Document>(crate::models::user::COLLECTION_NAME)
                            .update_one(
                                doc! {"_id":&fixture.funding},
                                doc! {"$set":{"is_active":false}},
                            )
                            .await
                            .unwrap();
                    }
                    tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait())
                        .await
                        .unwrap();
                    before
                };
                let (result, before) =
                    tokio::time::timeout(std::time::Duration::from_secs(30), async {
                        tokio::join!(run, mutate)
                    })
                    .await
                    .unwrap();
                assert!(result.is_err(), "{change}");
                assert_eq!(fixture.pending_marker().await, before, "{change}");
                assert!(
                    fixture.server.received_requests().await.unwrap().is_empty(),
                    "{change}"
                );
                assert_eq!(
                    fixture
                        .state
                        .db
                        .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                        .count_documents(doc! {})
                        .await
                        .unwrap(),
                    0,
                    "{change}"
                );
                fixture.state.db.drop().await.unwrap();
            }
        },
    );
}

#[test]
fn direct_only_x_refuses_gateway_before_pending_refresh_billing_or_provider() {
    crate::test_utils::run_native_journey_test(
        "direct_only_x_refuses_gateway_before_pending_refresh_billing_or_provider",
        || async {
            let fixture = Fixture::new().await;
            assert!(
                fixture
                    .convert(NativeTransportChoice::Gateway)
                    .await
                    .is_err()
            );
            let bot = fixture
                .state
                .db
                .collection::<Document>(BOTS)
                .find_one(doc! {"_id":&fixture.bot.id})
                .await
                .unwrap()
                .unwrap();
            assert!(!bot.contains_key("platform_entry_pending"));
            assert!(fixture.server.received_requests().await.unwrap().is_empty());
            assert_eq!(
                fixture
                    .state
                    .db
                    .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            fixture.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn ordinary_pending_setup_and_shared_route_keys_refuse_before_sticky_native_claim() {
    crate::test_utils::run_native_journey_test(
        "ordinary_pending_setup_and_shared_route_keys_refuse_before_sticky_native_claim",
        || async {
            for blocker in ["pending_setup", "shared_route_key"] {
                let fixture = Fixture::new().await;
                let route = uuid::Uuid::new_v4().to_string();
                let key = uuid::Uuid::new_v4().to_string();
                let routes = fixture
                    .state
                    .db
                    .collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME);
                let record = doc! {"_id":&route,"user_id":&fixture.bot.user_id,"channel_bot_id":&fixture.bot.id,"platform":"x","platform_conversation_id":"ordinary-private","platform_conversation_type":"private","agent_api_key_id":&key,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()};
                routes.insert_one(record.clone()).await.unwrap();
                if blocker == "pending_setup" {
                    fixture.state.db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME).insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_bot_id":&fixture.bot.id,"status":"pending","transport":"gateway","pending_agent_api_key_id":uuid::Uuid::new_v4().to_string()}).await.unwrap();
                } else {
                    let mut shared = record;
                    shared.insert("_id", uuid::Uuid::new_v4().to_string());
                    shared.insert("channel_bot_id", uuid::Uuid::new_v4().to_string());
                    routes.insert_one(shared).await.unwrap();
                }
                assert!(
                    fixture
                        .convert(NativeTransportChoice::Direct)
                        .await
                        .is_err(),
                    "{blocker}"
                );
                let bot = fixture
                    .state
                    .db
                    .collection::<Document>(BOTS)
                    .find_one(doc! {"_id":&fixture.bot.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(!bot.contains_key("platform_entry_pending"), "{blocker}");
                assert!(
                    fixture.server.received_requests().await.unwrap().is_empty(),
                    "{blocker}"
                );
                assert_eq!(
                    fixture
                        .state
                        .db
                        .collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
                        .count_documents(doc! {})
                        .await
                        .unwrap(),
                    0,
                    "{blocker}"
                );
                super::super::channel_routing_service::delete_conversation(
                    &fixture.state.db,
                    &route,
                    &fixture.bot.user_id,
                )
                .await
                .unwrap();
                assert!(
                    !routes
                        .find_one(doc! {"_id":route})
                        .await
                        .unwrap()
                        .unwrap()
                        .get_bool("is_active")
                        .unwrap()
                );
                fixture.state.db.drop().await.unwrap();
            }
        },
    );
}

async fn enable_personal(
    fixture: &Fixture,
    profile: &PlatformBotConfig,
) -> crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls {
    enable_personal_as(&fixture.state, &fixture.bot.user_id, profile).await
}

pub(crate) async fn enable_personal_as(
    state: &AppState,
    actor: &str,
    profile: &PlatformBotConfig,
) -> crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls {
    let current = super::super::platform_bot_runtime_control_service::get(&state.db, &profile.id)
        .await
        .unwrap();
    let change = super::super::platform_bot_runtime_control_service::RuntimeChange {
        public_deployment_digest: None,
        initialize: current.is_none(),
        expected_profile_generation: current.is_none().then_some(profile.generation),
        expected_manager_authority_epoch: current
            .as_ref()
            .map_or(0, |row| row.manager_authority_epoch),
        expected_personal_control_revision: current
            .as_ref()
            .map_or(0, |row| row.personal_control_revision),
        expected_public_policy_revision: current
            .as_ref()
            .map_or(0, |row| row.public_policy_revision),
        personal_enabled: Some(true),
        public_enabled: Some(false),
        emergency_stopped: None,
        public_policy: None,
    };
    super::super::platform_bot_runtime_control_service::update(
        &state.db,
        &super::super::audit_service::AuditActor::from_auth_user(
            &crate::test_utils::test_auth_user(actor),
        ),
        b"native-review-test-audit",
        &profile.id,
        &change,
        &NoPublic,
    )
    .await
    .unwrap()
}

#[test]
fn active_native_rotation_requires_reviewed_paused_update_and_explicit_reenable() {
    crate::test_utils::run_native_journey_test(
        "active_native_rotation_requires_reviewed_paused_update_and_explicit_reenable",
        || async {
            let fixture = Fixture::new().await;
            let NativeConversionOutcome::Converted(first) = fixture
                .convert(NativeTransportChoice::Direct)
                .await
                .unwrap()
            else {
                panic!("initial native conversion")
            };
            let controls = enable_personal(&fixture, &first).await;
            let old_configuration = first.native_entry.as_ref().unwrap().clone();
            super::super::platform_link_intent_service::ensure_indexes(&fixture.state.db)
                .await
                .unwrap();
            super::super::platform_manager_ingress_service::ensure_indexes(&fixture.state.db)
                .await
                .unwrap();
            let admission = x_admission(
                &fixture,
                &first,
                controls.manager_authority_epoch,
                "status",
                "501",
            )
            .await;
            let login = x_admission(
                &fixture,
                &first,
                controls.manager_authority_epoch,
                "login",
                "502",
            )
            .await;
            let linked = execute_command(
                &fixture.state,
                &login,
                super::super::platform_manager_command_service::PersonalCommandKind::Link,
            )
            .await
            .unwrap()
            .unwrap();
            let super::super::platform_manager_command_service::PersonalCommandReply::Link {
                token,
            } = linked.reply
            else {
                panic!("real login intent")
            };
            let challenge = super::super::platform_link_intent_service::review(
                &fixture.state.db,
                fixture.state.auth_device_hmac_key.as_slice(),
                &fixture.bot.user_id,
                &token,
            )
            .await
            .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let approved = super::super::platform_link_intent_service::approve(
                &fixture.state.db,
                fixture.state.auth_device_hmac_key.as_slice(),
                &fixture.bot.user_id,
                &challenge.id,
                &challenge.review_digest,
                false,
            )
            .await
            .unwrap();
            let confirmation = approved.confirmation_command.unwrap();
            let before = review_lifecycle(&fixture.state, &fixture.bot.user_id, &fixture.bot.id)
                .await
                .unwrap();
            fixture
                .state
                .db
                .collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},
                    doc! {"$set":{"credential_epoch":2_i64}},
                )
                .await
                .unwrap();
            assert!(
                reserve_reviewed_update(
                    &fixture.state,
                    &fixture.bot.user_id,
                    &fixture.funding,
                    NativeTransportChoice::Direct,
                    &before,
                    None,
                    None
                )
                .await
                .is_err()
            );
            let reviewed = review_lifecycle(&fixture.state, &fixture.bot.user_id, &fixture.bot.id)
                .await
                .unwrap();
            reserve_reviewed_update(
                &fixture.state,
                &fixture.bot.user_id,
                &fixture.funding,
                NativeTransportChoice::Direct,
                &reviewed,
                None,
                None,
            )
            .await
            .unwrap();
            let stopped = super::super::platform_bot_runtime_control_service::get(
                &fixture.state.db,
                &first.id,
            )
            .await
            .unwrap()
            .unwrap();
            assert!(stopped.manager_authority_epoch > controls.manager_authority_epoch);
            assert!(!stopped.personal_enabled && !stopped.public_enabled);
            let update = admission.verified_update();
            let mut session = fixture.state.db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            assert!(
                super::super::platform_bot_runtime_control_service::fence_manager_in_session(
                    &fixture.state.db,
                    &mut session,
                    &first.id,
                    &update.manager_bot_id,
                    &update.entry,
                    Some(&update.observation_id)
                )
                .await
                .unwrap()
                .is_none()
            );
            session.abort_transaction().await.unwrap();
            assert!(
                fixture
                    .state
                    .db
                    .collection::<Document>(BOTS)
                    .find_one(
                        doc! {"_id":&fixture.bot.id,"platform_entry_pending":{"$exists":true}}
                    )
                    .await
                    .unwrap()
                    .is_some()
            );
            let NativeConversionOutcome::Converted(updated) = fixture
                .convert(NativeTransportChoice::Direct)
                .await
                .unwrap()
            else {
                panic!("reviewed update")
            };
            assert_eq!(updated.id, first.id);
            assert!(!updated.enabled);
            assert!(updated.generation > first.generation);
            assert!(
                updated.native_entry.as_ref().unwrap().transport_generation
                    > old_configuration.transport_generation
            );
            let paused = super::super::platform_bot_runtime_control_service::get(
                &fixture.state.db,
                &updated.id,
            )
            .await
            .unwrap()
            .unwrap();
            assert!(!paused.personal_enabled && !paused.public_enabled);
            let enabled = enable_personal(&fixture, &updated).await;
            assert!(enabled.personal_enabled);
            assert_eq!(
                enabled.manager_authority_epoch,
                stopped.manager_authority_epoch
            );
            assert_eq!(
                super::super::channel_bot_service::get_bot(&fixture.state.db, &fixture.bot.id)
                    .await
                    .unwrap()
                    .user_id,
                fixture.bot.user_id
            );
            // Re-enabling cannot reinterpret an old receipt or revive a cancelled
            // browser-approved proof, including confirmation from a fresh event.
            assert!(
                execute_command(
                    &fixture.state,
                    &admission,
                    super::super::platform_manager_command_service::PersonalCommandKind::Status
                )
                .await
                .is_err()
            );
            assert!(
                super::super::platform_link_intent_service::review(
                    &fixture.state.db,
                    fixture.state.auth_device_hmac_key.as_slice(),
                    &fixture.bot.user_id,
                    &token
                )
                .await
                .is_err()
            );
            let old_confirmation = x_admission(
                &fixture,
                &updated,
                enabled.manager_authority_epoch,
                confirmation.as_str(),
                "503",
            )
            .await;
            assert!(
                execute_command(
                    &fixture.state,
                    &old_confirmation,
                    super::super::platform_manager_command_service::PersonalCommandKind::Confirm
                )
                .await
                .is_err()
            );
            let cancelled = fixture
                .state
                .db
                .collection::<crate::models::platform_link_intent::PlatformLinkChallenge>(
                    crate::models::platform_link_intent::CHALLENGES,
                )
                .find_one(doc! {"_id":&challenge.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                cancelled.state,
                crate::models::platform_link_intent::ChallengeState::Cancelled
            );
            assert_eq!(
                fixture
                    .state
                    .db
                    .collection::<Document>(
                        crate::models::platform_personal_binding::COLLECTION_NAME
                    )
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            let fresh_status = x_admission(
                &fixture,
                &updated,
                enabled.manager_authority_epoch,
                "status",
                "504",
            )
            .await;
            assert!(
                execute_command(
                    &fixture.state,
                    &fresh_status,
                    super::super::platform_manager_command_service::PersonalCommandKind::Status
                )
                .await
                .unwrap()
                .is_some()
            );
            let fresh_login = x_admission(
                &fixture,
                &updated,
                enabled.manager_authority_epoch,
                "login",
                "505",
            )
            .await;
            let fresh = execute_command(
                &fixture.state,
                &fresh_login,
                super::super::platform_manager_command_service::PersonalCommandKind::Link,
            )
            .await
            .unwrap()
            .unwrap();
            let super::super::platform_manager_command_service::PersonalCommandReply::Link {
                token: fresh_token,
            } = fresh.reply
            else {
                panic!("fresh proof")
            };
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let fresh_review = super::super::platform_link_intent_service::review(
                &fixture.state.db,
                fixture.state.auth_device_hmac_key.as_slice(),
                &fixture.bot.user_id,
                &fresh_token,
            )
            .await
            .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let fresh_approved = super::super::platform_link_intent_service::approve(
                &fixture.state.db,
                fixture.state.auth_device_hmac_key.as_slice(),
                &fixture.bot.user_id,
                &fresh_review.id,
                &fresh_review.review_digest,
                false,
            )
            .await
            .unwrap();
            let fresh_confirmation = x_admission(
                &fixture,
                &updated,
                enabled.manager_authority_epoch,
                fresh_approved
                    .confirmation_command
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "506",
            )
            .await;
            let confirmed = execute_command(
                &fixture.state,
                &fresh_confirmation,
                super::super::platform_manager_command_service::PersonalCommandKind::Confirm,
            )
            .await
            .unwrap()
            .unwrap();
            assert!(matches!(
                confirmed.reply,
                super::super::platform_manager_command_service::PersonalCommandReply::Confirmed
            ));
            let binding = fixture
                .state
                .db
                .collection::<crate::models::platform_personal_binding::PlatformPersonalBinding>(
                    crate::models::platform_personal_binding::COLLECTION_NAME,
                )
                .find_one(doc! {"platform_config_id":&updated.id,"state":"active"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(binding.user_id, fixture.bot.user_id);
            assert_eq!(
                binding.manager_authority_epoch,
                enabled.manager_authority_epoch
            );
            let status = x_admission(
                &fixture,
                &updated,
                enabled.manager_authority_epoch,
                "status",
                "507",
            )
            .await;
            let committed = execute_command(
                &fixture.state,
                &status,
                super::super::platform_manager_command_service::PersonalCommandKind::Status,
            )
            .await
            .unwrap()
            .unwrap();
            assert!(matches!(
                committed.reply,
                super::super::platform_manager_command_service::PersonalCommandReply::Status(
                    crate::models::platform_manager_ingress::ChatLoginStatus::LoggedIn
                )
            ));
            assert_eq!(fixture.server.received_requests().await.unwrap().len(), 2);
            fixture.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn failed_native_setup_can_review_owner_bound_successor_connection_and_retry() {
    crate::test_utils::run_native_journey_test(
        "failed_native_setup_can_review_owner_bound_successor_connection_and_retry",
        || async {
            let fixture = Fixture::new().await;
            fixture
                .state
                .db
                .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
                .update_one(
                    doc! {"owner_id":&fixture.bot.user_id},
                    doc! {"$set":{"balance_credits":crate::models::credits::Credits::ZERO}},
                )
                .await
                .unwrap();
            assert!(
                fixture
                    .convert(NativeTransportChoice::Direct)
                    .await
                    .is_err()
            );
            let old_marker = fixture.pending_marker().await;
            let keys = fixture
                .state
                .db
                .collection::<Document>(crate::models::user_api_key::COLLECTION_NAME);
            let mut successor = keys
                .find_one(doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()})
                .await
                .unwrap()
                .unwrap();
            let successor_id = uuid::Uuid::new_v4().to_string();
            successor.insert("_id", &successor_id);
            keys.insert_one(successor).await.unwrap();
            keys.update_one(
                doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},
                doc! {"$set":{"status":"revoked"}},
            )
            .await
            .unwrap();
            let review = review_lifecycle(&fixture.state, &fixture.bot.user_id, &fixture.bot.id)
                .await
                .unwrap();
            super::super::platform_native_reply_service::with_transport_for_test(
                fixture.transport.clone(),
                reserve_reviewed_update(
                    &fixture.state,
                    &fixture.bot.user_id,
                    &fixture.funding,
                    NativeTransportChoice::Direct,
                    &review,
                    Some(&successor_id),
                    None,
                ),
            )
            .await
            .unwrap();
            let retained = fixture.pending_marker().await;
            assert_eq!(
                old_marker.get_str("profile_id").unwrap(),
                retained.get_str("profile_id").unwrap()
            );
            assert_eq!(retained.get_str("actor").unwrap(), fixture.bot.user_id);
            // Failed retry still leaves an inspectable exact reservation.
            assert!(
                fixture
                    .convert(NativeTransportChoice::Direct)
                    .await
                    .is_err()
            );
            assert_eq!(fixture.pending_marker().await, retained);
            assert!(
                review_lifecycle(&fixture.state, &fixture.bot.user_id, &fixture.bot.id)
                    .await
                    .is_ok()
            );
            assert!(fixture.server.received_requests().await.unwrap().is_empty());
            fixture
        .state
        .db
        .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
        .update_one(
            doc! {"owner_id":&fixture.bot.user_id},
            doc! {"$set":{"balance_credits":crate::models::credits::Credits::from_whole(100)}},
        )
        .await
        .unwrap();
            assert!(matches!(
                fixture
                    .convert(NativeTransportChoice::Direct)
                    .await
                    .unwrap(),
                NativeConversionOutcome::Converted(_)
            ));
            assert_eq!(fixture.server.received_requests().await.unwrap().len(), 1);
            fixture.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn failed_native_setup_successor_admin_and_funding_preserve_original_owner_and_transport_payer() {
    crate::test_utils::run_native_journey_test(
        "failed_native_setup_successor_admin_and_funding_preserve_original_owner_and_transport_payer",
        || async {
            let fixture = Fixture::new().await;
            let wallets = fixture
                .state
                .db
                .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME);
            wallets
                .update_one(
                    doc! {"owner_id":&fixture.bot.user_id},
                    doc! {"$set":{"balance_credits":crate::models::credits::Credits::ZERO}},
                )
                .await
                .unwrap();
            assert!(
                fixture
                    .convert(NativeTransportChoice::Direct)
                    .await
                    .is_err()
            );
            let before = fixture.pending_marker().await;
            assert!(fixture.server.received_requests().await.unwrap().is_empty());

            let successor_id = uuid::Uuid::new_v4().to_string();
            let mut successor =
                crate::test_utils::test_user(&successor_id, crate::models::user::UserType::Person);
            successor.role_ids = vec![
                super::super::role_service::get_platform_role_ids(&fixture.state.db)
                    .await
                    .unwrap()
                    .admin,
            ];
            fixture
                .state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .insert_one(successor)
                .await
                .unwrap();
            fixture
                .state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&fixture.bot.user_id},
                    doc! {"$set":{"role_ids":[]}},
                )
                .await
                .unwrap();
            fixture
                .state
                .db
                .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                .update_one(
                    doc! {"org_user_id":&fixture.funding,"member_user_id":&fixture.bot.user_id},
                    doc! {"$set":{"revoked_at":bson::DateTime::now()}},
                )
                .await
                .unwrap();
            let new_funding = super::super::org_service::create_org_user(
                &fixture.state.db,
                "Reviewed successor funding",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
            super::super::org_service::create_membership(
                &fixture.state.db,
                &new_funding,
                &successor_id,
                crate::models::org_membership::OrgRole::Admin,
                crate::models::org_membership::MemberScopeSource::Inherit,
                None,
            )
            .await
            .unwrap();
            assert!(
                fixture
                    .convert(NativeTransportChoice::Direct)
                    .await
                    .is_err()
            );
            let review = review_lifecycle(&fixture.state, &successor_id, &fixture.bot.id)
                .await
                .unwrap();
            super::super::platform_native_reply_service::with_transport_for_test(
                fixture.transport.clone(),
                reserve_reviewed_update(
                    &fixture.state,
                    &successor_id,
                    &new_funding,
                    NativeTransportChoice::Direct,
                    &review,
                    None,
                    None,
                ),
            )
            .await
            .unwrap();
            let reserved = fixture.pending_marker().await;
            assert_eq!(
                reserved.get_str("profile_id").unwrap(),
                before.get_str("profile_id").unwrap()
            );
            assert_eq!(reserved.get_str("actor").unwrap(), successor_id);
            assert_eq!(reserved.get_str("funding").unwrap(), new_funding);
            assert!(
                fixture
                    .convert_as(&successor_id, &new_funding, NativeTransportChoice::Direct)
                    .await
                    .is_err()
            );
            assert_eq!(fixture.pending_marker().await, reserved);
            assert!(fixture.server.received_requests().await.unwrap().is_empty());
            wallets
        .update_one(
            doc! {"owner_id":&fixture.bot.user_id},
            doc! {"$set":{"balance_credits":crate::models::credits::Credits::from_whole(100)}},
        )
        .await
        .unwrap();
            let NativeConversionOutcome::Converted(profile) = fixture
                .convert_as(&successor_id, &new_funding, NativeTransportChoice::Direct)
                .await
                .unwrap()
            else {
                panic!("reviewed successor conversion")
            };
            assert_eq!(profile.configured_by, successor_id);
            assert_eq!(profile.funding_owner_user_id, new_funding);
            assert!(!profile.enabled);
            let bot =
                super::super::channel_bot_service::get_bot(&fixture.state.db, &fixture.bot.id)
                    .await
                    .unwrap();
            assert_eq!(bot.user_id, fixture.bot.user_id);
            assert_eq!(bot.connection_id, fixture.bot.connection_id);
            let meters = super::super::channel_x_tests::billing::settled(&fixture.state).await;
            assert_eq!(meters.len(), 1);
            assert_eq!(meters[0].billing_owner_id, fixture.bot.user_id);
            assert_eq!(meters[0].actor_user_id, fixture.bot.user_id);
            assert_eq!(fixture.server.received_requests().await.unwrap().len(), 1);
            fixture.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn reviewed_native_update_refreshes_expired_same_epoch_oauth_without_invalidating_scope_review() {
    crate::test_utils::run_native_journey_test(
        "reviewed_native_update_refreshes_expired_same_epoch_oauth_without_invalidating_scope_review",
        || async {
            let fixture = Fixture::new().await;
            let NativeConversionOutcome::Converted(profile) = fixture
                .convert(NativeTransportChoice::Direct)
                .await
                .unwrap()
            else {
                panic!("conversion")
            };
            enable_personal(&fixture, &profile).await;
            let scopes = super::super::channel_adapters::x::REQUIRED_SCOPES;
            let reordered = scopes
                .iter()
                .rev()
                .copied()
                .chain(scopes.first().copied())
                .collect::<Vec<_>>()
                .join("  ");
            Mock::given(method("POST")).and(path("/token")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "access_token":"synthetic-refreshed-token","refresh_token":"synthetic-successor-refresh","expires_in":3600,"scope":reordered}))).expect(1).mount(&fixture.server).await;
            fixture
                .state
                .db
                .collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
                .update_one(
                    doc! {"slug":"twitter"},
                    doc! {"$set":{"token_url":format!("{}/token",fixture.server.uri())}},
                )
                .await
                .unwrap();
            let encrypted = fixture
                .state
                .encryption_keys
                .encrypt(b"synthetic-refresh-token")
                .await
                .unwrap();
            fixture.state.db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME).update_one(doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},doc! {"$set":{
        "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::minutes(1)),
        "refresh_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
            let review = review_lifecycle(&fixture.state, &fixture.bot.user_id, &fixture.bot.id)
                .await
                .unwrap();
            assert_eq!(
                fixture.server.received_requests().await.unwrap().len(),
                1,
                "inspection performs no OAuth refresh"
            );
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                super::super::platform_native_reply_service::with_transport_for_test(
                    fixture.transport.clone(),
                    reserve_reviewed_update(
                        &fixture.state,
                        &fixture.bot.user_id,
                        &fixture.funding,
                        NativeTransportChoice::Direct,
                        &review,
                        None,
                        None,
                    ),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            let connection = fixture
                .state
                .db
                .collection::<crate::models::user_api_key::UserApiKey>(
                    crate::models::user_api_key::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(connection.credential_epoch, 1);
            assert_eq!(connection.token_scopes.as_deref(), Some(reordered.as_str()));
            assert!(
                connection.expires_at.unwrap() > chrono::Utc::now() + chrono::Duration::minutes(30)
            );
            assert_eq!(fixture.server.received_requests().await.unwrap().len(), 2);
            let NativeConversionOutcome::Converted(updated) = fixture
                .convert(NativeTransportChoice::Direct)
                .await
                .unwrap()
            else {
                panic!("refreshed update")
            };
            assert_eq!(
                updated
                    .native_entry
                    .as_ref()
                    .unwrap()
                    .credential_fingerprint,
                profile
                    .native_entry
                    .as_ref()
                    .unwrap()
                    .credential_fingerprint
            );
            assert!(
                !super::super::platform_bot_runtime_control_service::get(
                    &fixture.state.db,
                    &profile.id
                )
                .await
                .unwrap()
                .unwrap()
                .personal_enabled
            );
            fixture.server.verify().await;
            fixture.state.db.drop().await.unwrap();
        },
    );
}

#[path = "platform_native_x_delivery_tests.rs"]
mod paid_delivery;

#[test]
fn basic_native_setup_and_successor_admin_repair_need_no_sponsor_membership() {
    crate::test_utils::run_native_journey_test("basic_native_successor_repair", || async {
        let fixture = Fixture::new().await;
        let NativeConversionOutcome::Converted(first) = fixture
            .convert_as(
                &fixture.bot.user_id,
                &fixture.bot.user_id,
                NativeTransportChoice::Direct,
            )
            .await
            .unwrap()
        else {
            panic!("Direct setup");
        };
        assert_eq!(first.funding_owner_user_id, fixture.bot.user_id);
        let successor = uuid::Uuid::new_v4().to_string();
        let mut person =
            crate::test_utils::test_user(&successor, crate::models::user::UserType::Person);
        person.role_ids = vec![
            super::super::role_service::get_platform_role_ids(&fixture.state.db)
                .await
                .unwrap()
                .admin,
        ];
        fixture
            .state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(person)
            .await
            .unwrap();
        let reviewed = review_lifecycle(&fixture.state, &successor, &fixture.bot.id)
            .await
            .unwrap();
        reserve_reviewed_update(
            &fixture.state,
            &successor,
            &successor,
            NativeTransportChoice::Direct,
            &reviewed,
            None,
            None,
        )
        .await
        .unwrap();
        let NativeConversionOutcome::Converted(updated) = fixture
            .convert_as(&successor, &successor, NativeTransportChoice::Direct)
            .await
            .unwrap()
        else {
            panic!("Reviewed repair");
        };
        assert!(updated.generation > first.generation);
        assert_eq!(updated.funding_owner_user_id, successor);
        assert_eq!(
            updated.native_entry.as_ref().unwrap().identity,
            first.native_entry.as_ref().unwrap().identity
        );
        fixture.state.db.drop().await.unwrap();
    });
}

#[test]
fn unavailable_oauth_returns_repair_status_and_disconnected_inventory_without_hiding_metadata() {
    crate::test_utils::run_native_journey_test("native_oauth_repair_status", || async {
        let fixture = Fixture::new().await;
        let NativeConversionOutcome::Converted(profile) = fixture
            .convert_as(
                &fixture.bot.user_id,
                &fixture.bot.user_id,
                NativeTransportChoice::Direct,
            )
            .await
            .unwrap()
        else {
            panic!("Direct setup");
        };
        enable_personal(&fixture, &profile).await;
        let runtime_rows = fixture
            .state
            .db
            .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME);
        runtime_rows
            .update_one(
                doc! {"platform_config_id":&profile.id},
                doc! {"$set":{"public_enabled":true,"emergency_stopped":false}},
            )
            .await
            .unwrap();
        let runtime =
            super::super::platform_bot_runtime_control_service::get(&fixture.state.db, &profile.id)
                .await
                .unwrap()
                .unwrap();
        let admission = x_admission(
            &fixture,
            &profile,
            runtime.manager_authority_epoch,
            "hello",
            "501",
        )
        .await;
        let binding = super::super::platform_personal_binding_service::tests::binding(
            &admission.verified_update(),
            &fixture.bot.user_id,
        );
        let display = super::super::platform_personal_binding_service::native_manager_display(
            &fixture.state.db,
            &profile.id,
            &profile.manager_bot_id,
            &profile.native_entry.as_ref().unwrap().identity,
        )
        .await
        .unwrap();
        assert!(display.is_some());
        assert!(
            super::super::platform_bot_status_service::view(&fixture.state.db, Some(&runtime))
                .await
                .unwrap()
                .faq_ready
        );
        let keys = fixture
            .state
            .db
            .collection::<Document>(crate::models::user_api_key::COLLECTION_NAME);
        let original = keys
            .find_one(doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()})
            .await
            .unwrap()
            .unwrap();
        let owners = fixture
            .state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME);
        owners
            .update_one(
                doc! {"_id":&fixture.bot.user_id},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        assert_eq!(
            super::super::platform_bot_status_service::view(&fixture.state.db, Some(&runtime))
                .await
                .unwrap()
                .state,
            "connection_repair_required",
            "disabled credential owner"
        );
        assert_eq!(
            super::super::platform_personal_binding_service::inventory_status(
                &fixture.state.db,
                &binding
            )
            .await
            .unwrap()
            .state,
            crate::models::platform_personal_binding::PersonalBindingState::Revoked,
            "disabled credential owner"
        );
        owners
            .update_one(
                doc! {"_id":&fixture.bot.user_id},
                doc! {"$set":{"is_active":true}},
            )
            .await
            .unwrap();
        let requests_before_inventory = fixture.server.received_requests().await.unwrap().len();
        for (cause, change) in [
            ("revoked", doc! {"status":"revoked"}),
            ("missing scopes", doc! {"token_scopes":""}),
            (
                "expired without refresh",
                doc! {"expires_at":bson::DateTime::from_millis(0)},
            ),
        ] {
            keys.update_one(
                doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},
                doc! {"$set":change},
            )
            .await
            .unwrap();
            let status =
                super::super::platform_bot_status_service::view(&fixture.state.db, Some(&runtime))
                    .await
                    .unwrap();
            assert_eq!(status.state, "connection_repair_required", "{cause}");
            assert!(!status.faq_ready);
            let inventory = super::super::platform_personal_binding_service::inventory_status(
                &fixture.state.db,
                &binding,
            )
            .await
            .unwrap();
            assert_eq!(
                inventory.state,
                crate::models::platform_personal_binding::PersonalBindingState::Revoked,
                "{cause}"
            );
            assert_eq!(inventory.availability, super::super::platform_personal_binding_service::BindingAvailability::TemporarilyUnavailable, "{cause}");
            assert_eq!(
                fixture.server.received_requests().await.unwrap().len(),
                requests_before_inventory,
                "{cause}: inventory is read-only and makes no provider request"
            );
            assert!(
                super::super::platform_personal_binding_service::native_manager_display(
                    &fixture.state.db,
                    &profile.id,
                    &profile.manager_bot_id,
                    &profile.native_entry.as_ref().unwrap().identity
                )
                .await
                .unwrap()
                .is_some()
            );
            keys.replace_one(
                doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},
                original.clone(),
            )
            .await
            .unwrap();
        }
        let refresh = fixture
            .state
            .encryption_keys
            .encrypt(b"synthetic-refresh-token")
            .await
            .unwrap();
        let refresh = bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: refresh,
        };
        keys.update_one(
            doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0),
            "refresh_token_encrypted":refresh.clone()}},
        )
        .await
        .unwrap();
        let requests_before_status = fixture.server.received_requests().await.unwrap().len();
        let refreshable =
            super::super::platform_bot_status_service::view(&fixture.state.db, Some(&runtime))
                .await
                .unwrap();
        assert_eq!(
            refreshable.state, "active",
            "expired refreshable credentials retain the entry identity"
        );
        assert!(refreshable.faq_ready);
        assert_eq!(
            super::super::platform_personal_binding_service::inventory_status(
                &fixture.state.db,
                &binding
            )
            .await
            .unwrap()
            .state,
            crate::models::platform_personal_binding::PersonalBindingState::Active
        );
        assert_eq!(
            fixture.server.received_requests().await.unwrap().len(),
            requests_before_status,
            "status and inventory never perform provider refresh"
        );
        use super::super::platform_bot_runtime_control_service::{RuntimeChange, update};
        let actor = super::super::audit_service::AuditActor::from_auth_user(
            &crate::test_utils::test_auth_user(&fixture.bot.user_id),
        );
        let mut change = RuntimeChange {
            initialize: false,
            expected_profile_generation: None,
            expected_manager_authority_epoch: runtime.manager_authority_epoch,
            expected_personal_control_revision: runtime.personal_control_revision,
            expected_public_policy_revision: runtime.public_policy_revision,
            personal_enabled: None,
            public_enabled: None,
            emergency_stopped: Some(true),
            public_policy: None,
            public_deployment_digest: None,
        };
        let paused = update(
            &fixture.state.db,
            &actor,
            b"native-review-test-audit",
            &profile.id,
            &change,
            &NoPublic,
        )
        .await
        .unwrap();
        assert!(paused.emergency_stopped);
        assert_eq!(
            super::super::platform_bot_status_service::view(&fixture.state.db, Some(&paused))
                .await
                .unwrap()
                .state,
            "paused"
        );
        change.expected_personal_control_revision = paused.personal_control_revision;
        change.expected_public_policy_revision = paused.public_policy_revision;
        change.personal_enabled = Some(true);
        change.public_enabled = Some(true);
        change.emergency_stopped = Some(false);
        let bot = super::super::channel_bot_service::get_bot(&fixture.state.db, &fixture.bot.id)
            .await
            .unwrap();
        let snapshot = super::super::platform_entry_credentials::EntryCredentialSnapshot::load(
            &fixture.state.db,
            fixture.transport.adapter.as_ref(),
            &bot,
        )
        .await
        .unwrap();
        assert!(snapshot.token_usable_or_refreshable());
        keys.update_one(
            doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},
            doc! {"$unset":{"refresh_token_encrypted":""}},
        )
        .await
        .unwrap();
        let mut session = fixture.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            !snapshot
                .fence_runtime_controls_in_session(&fixture.state.db, &mut session)
                .await
                .unwrap(),
            "removing refresh access after snapshot capture must fence Resume"
        );
        session.abort_transaction().await.unwrap();
        assert!(
            matches!(
                update(
                    &fixture.state.db,
                    &actor,
                    b"native-review-test-audit",
                    &profile.id,
                    &change,
                    &NoPublic
                )
                .await,
                Err(AppError::Conflict(_))
            ),
            "expired without refresh cannot resume"
        );
        keys.update_one(
            doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()},
            doc! {"$set":{"refresh_token_encrypted":refresh}},
        )
        .await
        .unwrap();
        let resumed = update(
            &fixture.state.db,
            &actor,
            b"native-review-test-audit",
            &profile.id,
            &change,
            &NoPublic,
        )
        .await
        .unwrap();
        assert!(
            !resumed.emergency_stopped && resumed.public_enabled && resumed.personal_enabled,
            "expired refreshable credentials must allow one atomic Resume"
        );
        assert_eq!(
            super::super::platform_bot_status_service::view(&fixture.state.db, Some(&resumed))
                .await
                .unwrap()
                .state,
            "active"
        );
        session.start_transaction().await.unwrap();
        assert!(
            !snapshot
                .fence_in_session(&fixture.state.db, &mut session)
                .await
                .unwrap(),
            "runtime configuration cannot authorize expired message admission or delivery"
        );
        session.abort_transaction().await.unwrap();
        assert!(
            snapshot
                .send_token(&fixture.state.encryption_keys)
                .await
                .is_err()
        );
        assert_eq!(
            fixture.server.received_requests().await.unwrap().len(),
            requests_before_status,
            "status, inventory and atomic Resume never perform provider refresh"
        );
        keys.delete_one(doc! {"_id":fixture.bot.connection_id.as_ref().unwrap()})
            .await
            .unwrap();
        assert_eq!(
            super::super::platform_bot_status_service::view(&fixture.state.db, Some(&runtime))
                .await
                .unwrap()
                .state,
            "connection_repair_required"
        );
        keys.insert_one(original).await.unwrap();
        runtime_rows
            .update_one(
                doc! {"platform_config_id":&profile.id},
                doc! {"$inc":{"manager_authority_epoch":1_i64}},
            )
            .await
            .unwrap();
        assert_eq!(
            super::super::platform_personal_binding_service::inventory_status(
                &fixture.state.db,
                &binding
            )
            .await
            .unwrap()
            .state,
            crate::models::platform_personal_binding::PersonalBindingState::Revoked
        );
        fixture.state.db.drop().await.unwrap();
    });
}

#[test]
fn polling_only_x_cannot_advertise_a_native_shared_entry() {
    crate::test_utils::run_native_journey_test("native_x_needs_signed_webhook", || async {
        let fixture = Fixture::new().await;
        fixture
            .state
            .db
            .collection::<Document>(BOTS)
            .update_one(
                doc! {"_id":&fixture.bot.id},
                doc! {"$set":{"webhook_registered":false}},
            )
            .await
            .unwrap();
        let error = fixture
            .convert_as(
                &fixture.bot.user_id,
                &fixture.bot.user_id,
                NativeTransportChoice::Direct,
            )
            .await
            .err()
            .unwrap();
        assert!(
            matches!(error,AppError::Conflict(message) if message.contains("activity webhooks"))
        );
        assert!(fixture.server.received_requests().await.unwrap().is_empty());
        assert!(
            fixture
                .state
                .db
                .collection::<Document>(BOTS)
                .find_one(doc! {"_id":&fixture.bot.id,"platform_entry_pending":{"$exists":true}})
                .await
                .unwrap()
                .is_none()
        );
        fixture.state.db.drop().await.unwrap();
    });
}
