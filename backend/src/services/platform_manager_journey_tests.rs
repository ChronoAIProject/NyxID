//! Local Mongo transactions and a local Telegram HTTP server; never live accounts.
#[path = "platform_manager_journey_tests/account_parity.rs"]
mod account_parity;
#[path = "platform_manager_journey_tests/faq.rs"]
mod faq;
#[path = "platform_manager_journey_tests/legacy_compatibility.rs"]
mod legacy_compatibility;
#[path = "platform_manager_journey_tests/management.rs"]
mod management;
#[path = "platform_manager_journey_tests/native_personal.rs"]
mod native_personal;
use super::*;
use crate::{
    models::{
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        platform_bot_config::{PlatformBotConfig, PlatformExecutionProfile},
        platform_bot_runtime_controls::PlatformBotRuntimeControls,
        platform_manager_delivery::{DeliveryState, PlatformManagerDelivery},
        platform_manager_ingress::{IngressAudience, PlatformManagerIngress},
        platform_personal_binding::{PersonalBindingState, PlatformPersonalBinding},
    },
    services::{assistant_nyxagent as engine, telegram_new_tests as telegram},
};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct SavedReplyEgress(AtomicUsize, tokio::sync::Mutex<Vec<Value>>);
#[async_trait]
impl NyxAgentEgress for SavedReplyEgress {
    async fn request(
        &self,
        context: &NyxAgentExecutionContext,
        _: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        assert!(matches!(
            context,
            NyxAgentExecutionContext::Owner {
                platform_origin: Some(_),
                ..
            }
        ));
        assert_eq!(request.relative_path, "v1/responses");
        self.1.lock().await.push(request.body.clone().unwrap());
        self.0.fetch_add(1, Ordering::SeqCst);
        use axum::response::IntoResponse;
        let session = "1234567890abcdef1234567890abcdef";
        let frame = json!({"type":"response.completed","sequence_number":0,"response":{"id":format!("resp_{session}_1234567890abcdef1234567890abcdef"),"conversation":{"id":format!("conv_{session}")},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Retained personal answer"}]}]}});
        Ok((
            [("content-type", "text/event-stream")],
            format!("data: {frame}\n\n"),
        )
            .into_response())
    }
}

struct TestSettledHook;
#[async_trait]
impl crate::services::assistant_nyxagent_runtime::OwnerTurnSettledHook for TestSettledHook {
    async fn settled(
        &self,
        _: crate::models::assistant_conversation::AssistantConversation,
        _: ReplyRef,
    ) {
    }
}

struct PersonalDeliveryGate {
    capacity: Arc<tokio::sync::Semaphore>,
    arrived: tokio::sync::Notify,
    release: tokio::sync::Notify,
    finished: tokio::sync::Notify,
    outcome: tokio::sync::Mutex<Option<(String, DeliveryOutcome)>>,
}
impl PersonalDeliveryGate {
    fn new() -> Self {
        Self {
            capacity: Arc::new(tokio::sync::Semaphore::new(0)),
            arrived: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            finished: tokio::sync::Notify::new(),
            outcome: tokio::sync::Mutex::new(None),
        }
    }
}

struct LocalTransport {
    state: AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    base: String,
    personal_gate: Option<Arc<PersonalDeliveryGate>>,
}
#[async_trait]
impl ManagerTransportProducer for LocalTransport {
    async fn admit_manager_update(
        &self,
        db: &Database,
        update: &VerifiedManagerUpdate,
    ) -> AppResult<IngressAdmission> {
        dispatch::admit_manager_update(db, self.identity.as_ref(), update).await
    }
    async fn deliver_platform_reply(
        &self,
        _: &Database,
        destination: &ReplyDestination,
        reference: &DeliveryRef,
        content: ReplyContent,
    ) -> AppResult<DeliveryOutcome> {
        let api = super::super::telegram_new_api::TelegramApi {
            http: &self.state.http_client,
            base_url: &self.base,
        };
        let send = delivery::deliver(
            &self.state,
            self.identity.clone(),
            destination,
            reference,
            content,
            &api,
        );
        if let Some(gate) = &self.personal_gate
            && let DeliveryRef::Reply(reply) = reference
        {
            gate.arrived.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(20), gate.release.notified())
                .await
                .expect("first personal delivery released");
            let outcome =
                delivery::with_control_capacity_for_test(gate.capacity.clone(), send).await?;
            *gate.outcome.lock().await = Some((reply.reply_id.clone(), outcome.clone()));
            gate.finished.notify_one();
            Ok(outcome)
        } else {
            send.await
        }
    }
}

struct Journey {
    state: AppState,
    owner: String,
    server: MockServer,
    producers: Arc<PlatformManagerProducers>,
    egress: Arc<SavedReplyEgress>,
    profile: PlatformBotConfig,
}
impl Journey {
    async fn new() -> Self {
        let (state, owner, server) = telegram::fixture().await;
        let bot = telegram::register_manager_channel(&state, &owner, &server).await;
        for result in [
            super::super::platform_personal_binding_service::ensure_indexes(&state.db).await,
            super::super::platform_link_intent_service::ensure_indexes(&state.db).await,
            super::super::platform_bot_runtime_control_service::ensure_indexes(&state.db).await,
            super::super::platform_manager_ingress_service::ensure_indexes(&state.db).await,
            delivery::ensure_indexes(&state.db).await,
            engine::ensure_indexes(&state.db).await,
        ] {
            result.unwrap();
        }
        super::super::feature_flag_service::set_platform_override(
            &state.db,
            super::super::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
            &super::super::feature_flag_service::FlagTarget::Global,
            true,
            "admin",
        )
        .await
        .unwrap();
        let now = Utc::now();
        let profile = PlatformBotConfig {
            identity_version: None,
            native_entry: None,
            id: uuid::Uuid::new_v4().to_string(),
            channel_bot_id: bot.id.clone(),
            manager_bot_id: "100".into(),
            observation_id: "test-observation".into(),
            execution_profile: PlatformExecutionProfile::PlatformOnboarding,
            generation: 1,
            enabled: false,
            guide: None,
            funding_owner_user_id: uuid::Uuid::new_v4().to_string(),
            configured_by: owner.clone(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        state
            .db
            .collection::<PlatformBotConfig>(PROFILES)
            .insert_one(&profile)
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id":bot.id},
                doc! {"$set":{"platform_profile_id":&profile.id}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<PlatformBotRuntimeControls>(
                crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
            )
            .insert_one(PlatformBotRuntimeControls {
                id: uuid::Uuid::new_v4().to_string(),
                contract_version: 1,
                platform_config_id: profile.id.clone(),
                manager_bot_id: "100".into(),
                entry: crate::models::platform_entry_projection::StoredEntryIdentity::TelegramV1 {
                    telegram_bot_id: 100,
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
            })
            .await
            .unwrap();
        let egress = Arc::new(SavedReplyEgress(
            AtomicUsize::new(0),
            tokio::sync::Mutex::new(Vec::new()),
        ));
        let public_runtime =
            super::super::platform_public_credential_service::producer(state.clone());
        let execution = super::super::assistant_nyxagent_runtime::producer(
            state.clone(),
            egress.clone(),
            public_runtime.clone(),
        );
        let mut producers = assemble(
            state.clone(),
            execution,
            egress.clone(),
            Arc::new(TestSettledHook),
        );
        let identity = producers.identity.clone();
        Arc::get_mut(&mut producers).unwrap().transport = Arc::new(LocalTransport {
            state: state.clone(),
            identity,
            base: server.uri(),
            personal_gate: None,
        });
        Self {
            state,
            owner,
            server,
            producers,
            egress,
            profile,
        }
    }
    fn update(&self, id: i64, text: &str, forwarded: bool) -> Value {
        let mut update = telegram::message(json!({"text":text,"message_id":id}));
        update["update_id"] = json!(id);
        if forwarded {
            update["message"]["forward_origin"] = json!({"type":"hidden_user","sender_user_name":"Someone","date":Utc::now().timestamp()});
        }
        update
    }
    async fn webhook(&self, update: &Value) {
        let base = self.server.uri();
        let status = crate::handlers::telegram_new::webhook_with_producers(
            &self.state,
            &telegram::service(&self.state, &base),
            &telegram::headers(&self.state).await,
            &serde_json::to_vec(update).unwrap(),
            Some(self.producers.clone()),
        )
        .await
        .unwrap();
        assert_eq!(status, axum::http::StatusCode::OK);
    }
    async fn sends(&self) -> Vec<Value> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/sendMessage"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    }
    /// Local injected state exercises only the delivery boundary, never runtime
    /// readiness or authorization from a supported public upstream.
    async fn public_signup(&self) -> (PublicExecutionContext, zeroize::Zeroizing<String>) {
        use crate::models::{
            credits::Credits,
            platform_bot_runtime_controls::PublicRuntimePolicy,
            platform_manager_ingress::IngressExecution,
            platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
        };
        super::super::platform_public_agent_service::ensure_indexes(&self.state.db)
            .await
            .unwrap();
        let update = dispatch::verified_message(
            &self.profile.id,
            &self.profile.manager_bot_id,
            100,
            &self.profile.observation_id,
            &self.update(900, "Guide me", false),
        )
        .unwrap();
        let mut public = super::super::platform_public_agent_service::tests::fixture(
            &self.state.db,
            crate::services::platform_entry_scope::legacy_sender_id(&update.sender),
            false,
        )
        .await;
        public.profile_id = self.profile.id.clone();
        public.manager_bot_id = "100".into();
        public.entry = EntryIdentity::TelegramV1 {
            telegram_bot_id: 100,
        };
        public.manager_authority_epoch = 1;
        self.state
            .db
            .collection::<PlatformPublicSession>(SESSIONS)
            .replace_one(doc! {"_id":&public.id}, &public)
            .await
            .unwrap();
        self.state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .insert_one(
                doc! {"_id":&public.sponsor_owner_user_id,"user_type":"org","is_active":true},
            )
            .await
            .unwrap();
        self.state.db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(doc! {"platform_config_id":&self.profile.id},doc! {"$set":{
                "public_enabled":true,"public_policy":bson::to_bson(&PublicRuntimePolicy {
                    sponsor_owner_user_id:public.sponsor_owner_user_id.clone(),
                    nyxagent_profile:public.nyxagent_profile.clone(),
                    total_daily_credits:Credits::from_whole(10),sender_daily_credits:Credits::from_whole(1),
                }).unwrap(),
            }}).await.unwrap();
        let active = public.active_turn.as_ref().unwrap();
        let context = PublicExecutionContext {
            proof_record_id: "test-proof".into(),
            ingress: PublicIngress {
                ingress: IngressClaim {
                    receipt_id: active.receipt_id.clone(),
                    claim_id: active.claim_id.clone(),
                },
                manager: ManagerAuthority {
                    profile_id: public.profile_id.clone(),
                    manager_bot_id: public.manager_bot_id.clone(),
                    entry:
                        crate::models::platform_entry_projection::StoredEntryIdentity::TelegramV1 {
                            telegram_bot_id: 100,
                        },
                    manager_authority_epoch: 1,
                },
                sender: public.sender.clone(),
                public_policy_revision: public.public_policy_revision,
            },
            public_session_id: public.id.clone(),
            turn: TurnRef {
                conversation_id: active.conversation_id.clone(),
                turn_id: active.turn_id.clone(),
            },
            sponsor_owner_user_id: public.sponsor_owner_user_id.clone(),
            policy_digest: public.policy_digest.clone(),
            nyxagent_profile: public.nyxagent_profile.clone(),
            model_operations: vec![],
            limits: PublicExecutionLimits {
                max_duration_secs: 60,
                max_model_calls: 2,
                max_tool_calls: 3,
                max_continuations: 0,
                max_input_tokens: 20,
                max_output_tokens: 10,
                max_input_bytes: 100,
                max_context_bytes: 100,
                max_output_bytes: 100,
            },
            expires_at: active.expires_at,
        };
        let mut session = self.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        super::super::platform_manager_ingress_service::record_admission_in_session(
            &self.state.db,
            &mut session,
            &update,
            &context.ingress.manager,
            &context.ingress.ingress,
            &ManagerAudience::Public(context.ingress.clone()),
        )
        .await
        .unwrap();
        self.state.db.collection::<Document>(INGRESS).update_one(doc! {"_id":&active.receipt_id},doc! {"$set":{
            "phase":"executing","execution":bson::to_bson(&IngressExecution {conversation_id:active.conversation_id.clone(),turn_id:active.turn_id.clone(),public_session_id:Some(public.id.clone()),payer_user_id:public.sponsor_owner_user_id.clone(),public_policy_digest:Some(public.policy_digest.clone())}).unwrap(),
        }}).session(&mut session).await.unwrap();
        let issued = super::super::platform_link_intent_service::begin_public_in_session(
            &self.state.db,
            &mut session,
            self.state.auth_device_hmac_key.as_slice(),
            &context,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        (context, issued.token.unwrap())
    }
    async fn signup_send(
        &self,
        context: &PublicExecutionContext,
        token: zeroize::Zeroizing<String>,
    ) -> delivery::PublicSignupDeliveryOutcome {
        delivery::deliver_public_signup_with_api(
            &self.state,
            context,
            token,
            &super::super::telegram_new_api::TelegramApi {
                http: &self.state.http_client,
                base_url: &self.server.uri(),
            },
        )
        .await
        .unwrap()
    }
    async fn link(&self) {
        self.webhook(&self.update(10, "/link", false)).await;
        let sends = self.sends().await;
        let text = sends.last().unwrap()["text"].as_str().unwrap();
        let url = text
            .split_whitespace()
            .find(|word| word.starts_with("https://") || word.starts_with("http://"))
            .unwrap();
        let url = reqwest::Url::parse(url).unwrap();
        let token = url
            .fragment()
            .unwrap()
            .strip_prefix("intent_token=")
            .unwrap();
        let review = super::super::platform_link_intent_service::review(
            &self.state.db,
            self.state.auth_device_hmac_key.as_slice(),
            &self.owner,
            token,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let approved = super::super::platform_link_intent_service::approve(
            &self.state.db,
            self.state.auth_device_hmac_key.as_slice(),
            &self.owner,
            &review.id,
            &review.review_digest,
            false,
        )
        .await
        .unwrap();
        let command = approved.confirmation_command.unwrap();
        self.webhook(&self.update(11, command.as_str(), false))
            .await;
        assert!(
            self.sends().await.last().unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("is linked")
        );
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }

    async fn control(
        &self,
        id: i64,
        text: &str,
    ) -> Option<super::super::platform_manager_command_service::CommittedCommand> {
        self.control_as(id, text, 700).await
    }
    async fn control_as(
        &self,
        id: i64,
        text: &str,
        sender: i64,
    ) -> Option<super::super::platform_manager_command_service::CommittedCommand> {
        let mut payload = self.update(id, text, false);
        payload["message"]["from"]["id"] = json!(sender);
        payload["message"]["chat"]["id"] = json!(sender);
        let update = dispatch::verified_message(
            &self.profile.id,
            "100",
            100,
            &self.profile.observation_id,
            &payload,
        )
        .unwrap();
        let command =
            OriginalManagerCommand::from_verified(update, TurnInput { text: text.into() }).unwrap();
        commands::execute(
            &self.state.db,
            self.state.auth_device_hmac_key.as_slice(),
            self.state.audit_chain_hmac_key.as_slice(),
            &command,
            super::super::platform_manager_management::parse(text, "manager_bot")
                .map(commands::PersonalCommandKind::Management)
                .or_else(|| commands::personal_command(text, "manager_bot"))
                .unwrap(),
            &bindings::LifecycleProducer {
                personal: self.producers.personal.clone(),
                public: self.producers.public.clone(),
            },
        )
        .await
        .unwrap()
    }
    async fn confirm_login(
        &self,
        id: i64,
    ) -> super::super::platform_manager_command_service::CommittedCommand {
        let login = self.control(id, "/login").await.unwrap();
        let commands::PersonalCommandReply::Link { token } = login.reply else {
            panic!("Expected a fresh login review");
        };
        let review = super::super::platform_link_intent_service::review(
            &self.state.db,
            self.state.auth_device_hmac_key.as_slice(),
            &self.owner,
            &token,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let approved = super::super::platform_link_intent_service::approve(
            &self.state.db,
            self.state.auth_device_hmac_key.as_slice(),
            &self.owner,
            &review.id,
            &review.review_digest,
            false,
        )
        .await
        .unwrap();
        self.control(id + 1, approved.confirmation_command.as_ref().unwrap())
            .await
            .unwrap()
    }
    async fn send_control(
        &self,
        command: super::super::platform_manager_command_service::CommittedCommand,
    ) -> DeliveryOutcome {
        delivery::deliver_command(
            &self.state,
            self.producers.identity.clone(),
            command,
            &super::super::telegram_new_api::TelegramApi {
                http: &self.state.http_client,
                base_url: &self.server.uri(),
            },
        )
        .await
        .unwrap()
    }
    async fn saved(&self) -> (ReplyDestination, ReplyRef, AssistantMessage) {
        let message = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if let Some(message) = self
                    .state
                    .db
                    .collection::<AssistantMessage>(MESSAGES)
                    .find_one(doc! {"user_id":&self.owner,"role":"assistant"})
                    .await
                    .unwrap()
                {
                    break message;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let origin = super::super::platform_personal_turn_service::contract_origin(
            message.platform_origin.as_ref().unwrap(),
        )
        .unwrap();
        let turn = TurnRef {
            conversation_id: message.conversation_id.clone(),
            turn_id: message.turn_id.clone(),
        };
        let destination = ReplyDestination {
            manager: origin.authority.manager.clone(),
            sender: origin.authority.sender.clone(),
            authority: ReplyAuthority::Personal {
                origin,
                turn: turn.clone(),
            },
            expires_at: Utc::now() + Duration::minutes(10),
        };
        let reply = ReplyRef {
            turn,
            reply_id: message.id.clone(),
        };
        (destination, reply, message)
    }
}

#[tokio::test]
async fn committed_control_effects_reject_confirm_logout_and_bearer_delivery_races() {
    // Each scenario has a fresh transport budget, so authority refusal cannot
    // be accidentally masked by a pacing refusal from the preceding scenario.
    for scenario in [
        "confirm-logout",
        "logout-login",
        "logout-confirm",
        "link-logout",
    ] {
        let j = Journey::new().await;
        let stale = match scenario {
            "confirm-logout" => {
                let confirmed = j.confirm_login(300).await;
                j.control(302, "/logout").await.unwrap();
                confirmed
            }
            "logout-login" => {
                let logout = j.control(310, "/logout").await.unwrap();
                j.control(311, "/login").await.unwrap();
                logout
            }
            "logout-confirm" => {
                let logout = j.control(320, "/logout").await.unwrap();
                j.confirm_login(321).await;
                logout
            }
            "link-logout" => {
                let login = j.control(330, "/login").await.unwrap();
                j.control(331, "/logout").await.unwrap();
                login
            }
            _ => unreachable!(),
        };
        let receipt = j
            .state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .find_one(doc! {"_id": &stale.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.control_effect.as_ref(), Some(&stale.effect));
        assert!(
            matches!(
                j.send_control(stale).await,
                DeliveryOutcome::Refused(UnavailableReason::OriginRevoked)
            ),
            "{scenario}"
        );
        assert!(j.sends().await.is_empty(), "{scenario}");
        let stored = j
            .state
            .db
            .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME)
            .find_one(doc! {"reference.kind":"command"})
            .await
            .unwrap()
            .unwrap();
        assert!(
            stored
                .get_document("reference")
                .unwrap()
                .contains_key("effect")
        );
        assert!(!stored.contains_key("command_status"));
    }
}

#[tokio::test]
async fn public_control_capacity_does_not_delay_logout_effects_or_personal_delivery() {
    let mut j = Journey::new().await;
    let gate = Arc::new(PersonalDeliveryGate::new());
    let transport = Arc::new(LocalTransport {
        state: j.state.clone(),
        identity: j.producers.identity.clone(),
        base: j.server.uri(),
        personal_gate: Some(gate.clone()),
    });
    Arc::get_mut(&mut j.producers).unwrap().transport = transport;
    j.confirm_login(400).await;
    delivery::with_control_capacity_for_test(gate.capacity.clone(), async {
        j.webhook(&j.update(402, "Save a personal reply", false))
            .await;
        let (_, reply, _) = j.saved().await;
        tokio::time::timeout(std::time::Duration::from_secs(20), gate.arrived.notified())
            .await
            .expect("actual first personal send reaches transport gate");
        let before = j.sends().await.len();
        // Real commands commit under saturated delivery capacity. A visitor's
        // pending proof is cancelled even when its acknowledgement cannot be sent.
        j.control_as(403, "/login", 701).await.unwrap();
        let logout = j.control_as(404, "/logout", 701).await.unwrap();
        assert!(matches!(
            j.send_control(logout).await,
            DeliveryOutcome::Refused(UnavailableReason::CapacityLimited)
        ));
        assert_eq!(
            j.state
                .db
                .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
                .count_documents(doc! {"telegram_sender_id":701_i64,"state":"pending"})
                .await
                .unwrap(),
            0
        );
        for id in 405..413 {
            let command = j
                .control_as(id, if id % 2 == 0 { "/status" } else { "/help" }, 700 + id)
                .await
                .unwrap();
            assert!(matches!(
                j.send_control(command).await,
                DeliveryOutcome::Refused(UnavailableReason::CapacityLimited)
            ));
        }
        assert_eq!(
            j.sends().await.len(),
            before,
            "control acknowledgements remain unsent while saturated"
        );
        gate.release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(30), gate.finished.notified())
            .await
            .expect("actual first personal send completes while controls remain saturated");
        let outcome = gate.outcome.lock().await;
        let (reply_id, sent) = outcome.as_ref().expect("captured first-send outcome");
        assert_eq!(reply_id, &reply.reply_id);
        assert!(matches!(sent, DeliveryOutcome::Sent { .. }), "{sent:?}");
        assert_eq!(
            j.sends().await.len(),
            before + 1,
            "exactly the original personal reply was sent"
        );
        assert!(j.sends().await.iter().any(|s| {
            s["text"]
                .as_str()
                .unwrap()
                .contains("Retained personal answer")
        }));
    })
    .await;
}

#[tokio::test]
async fn login_status_logout_use_live_sender_authority_and_cancel_pending_proofs() {
    use commands::{ChatLoginStatus as Status, PersonalCommandReply as Reply};
    let j = Journey::new().await;
    assert!(matches!(
        j.control(101, "/status").await.unwrap().reply,
        Reply::Status(Status::LoggedOut)
    ));
    assert!(matches!(
        j.control(102, "/login").await.unwrap().reply,
        Reply::Link { .. }
    ));
    assert!(j.control(102, "/login").await.is_none());
    assert!(matches!(
        j.control(103, "/status").await.unwrap().reply,
        Reply::Status(Status::Pending)
    ));
    j.control(104, "/logout").await.unwrap();
    assert!(matches!(
        j.control(105, "/status").await.unwrap().reply,
        Reply::Status(Status::LoggedOut)
    ));
    let pending = j
        .state
        .db
        .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
        .count_documents(doc! {"platform_config_id": &j.profile.id, "state": "pending"})
        .await
        .unwrap();
    assert_eq!(pending, 0);
    j.link().await;
    assert!(matches!(
        j.control(106, "/status").await.unwrap().reply,
        Reply::Status(Status::LoggedIn)
    ));
    assert!(matches!(
        j.control(107, "/login").await.unwrap().reply,
        Reply::Status(Status::LoggedIn)
    ));
    j.state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": &j.owner}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert!(matches!(
        j.control(108, "/status").await.unwrap().reply,
        Reply::Status(Status::Unavailable)
    ));
    j.control(109, "/logout").await.unwrap();
    assert!(matches!(
        j.control(110, "/status").await.unwrap().reply,
        Reply::Status(Status::LoggedOut)
    ));
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn status_changed_before_send_is_refused_and_other_sender_stays_logged_out() {
    let j = Journey::new().await;
    j.link().await;
    let status = j.control(201, "/status").await.unwrap();
    let mut other = j.update(202, "/status", false);
    other["message"]["from"]["id"] = json!(701);
    other["message"]["chat"]["id"] = json!(701);
    j.webhook(&other).await;
    let sent = j.sends().await;
    let other_status = sent.iter().find(|send| send["chat_id"] == 701).unwrap();
    assert!(
        other_status["text"]
            .as_str()
            .unwrap()
            .contains("logged out")
    );
    j.control(203, "/logout").await.unwrap();
    let outcome = delivery::deliver_command(
        &j.state,
        j.producers.identity.clone(),
        status,
        &super::super::telegram_new_api::TelegramApi {
            http: &j.state.http_client,
            base_url: &j.server.uri(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(outcome, DeliveryOutcome::Refused(_)));
    for (id, command) in [
        (204, "/login"),
        (205, "/logout"),
        (206, "/status"),
        (207, "/help"),
    ] {
        j.webhook(&j.update(id, command, true)).await;
    }
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn telegram_personal_link_saved_reply_web_history_and_unlink_journey() {
    let j = Journey::new().await;
    // An unlinked update keeps its original fixed audience after subsequent consent.
    let old = j.update(2, "before linking", false);
    j.webhook(&old).await;
    j.link().await;
    j.webhook(&old).await;
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
    let before_link = j
        .state
        .db
        .collection::<PlatformManagerIngress>(INGRESS)
        .find_one(doc! {"update_id":2_i64})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        before_link.audience,
        IngressAudience::Fixed { .. }
    ));
    let update = j.update(
        12,
        "Please use this deliberately forwarded ordinary text",
        true,
    );
    j.webhook(&update).await;
    j.webhook(&update).await;
    let (destination, reply, saved) = j.saved().await;
    assert_eq!(saved.text, "Retained personal answer");
    let web = engine::messages(&j.state.db, &j.owner, &reply.turn.conversation_id, 50, None)
        .await
        .unwrap();
    assert!(web.iter().any(|m| m.id == saved.id));
    assert!(
        web.iter()
            .any(|m| m.role == "user" && m.text.contains("forwarded ordinary text"))
    );
    assert!(
        engine::list(&j.state.db, &j.owner, 50, None, None)
            .await
            .unwrap()
            .iter()
            .any(|thread| thread.id == reply.turn.conversation_id)
    );
    assert!(
        engine::messages(
            &j.state.db,
            &uuid::Uuid::new_v4().to_string(),
            &reply.turn.conversation_id,
            50,
            None
        )
        .await
        .is_err()
    );
    let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
    let reference = DeliveryRef::Reply(reply.clone());
    let (a, b) = tokio::join!(
        j.producers.transport.deliver_platform_reply(
            &j.state.db,
            &destination,
            &reference,
            ReplyContent::Text(body.clone())
        ),
        j.producers.transport.deliver_platform_reply(
            &j.state.db,
            &destination,
            &reference,
            ReplyContent::Text(body.clone())
        )
    );
    assert!(a.is_ok() && b.is_ok());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if j.sends().await.iter().any(|s| {
                s["text"]
                    .as_str()
                    .unwrap()
                    .contains("Retained personal answer")
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let sends = j.sends().await;
    let private: Vec<_> = sends
        .iter()
        .filter(|s| {
            s["text"]
                .as_str()
                .unwrap()
                .contains("Retained personal answer")
        })
        .collect();
    assert_eq!(private.len(), 1);
    assert_eq!(private[0]["chat_id"], 700);
    assert_eq!(private[0]["protect_content"], true);
    assert!(
        private[0]["text"]
            .as_str()
            .unwrap()
            .contains(&format!("?c={}", reply.turn.conversation_id))
    );
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 1);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    j.webhook(&j.update(13, "/unlink", false)).await;
    let binding = j
        .state
        .db
        .collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .find_one(doc! {"platform_config_id":&j.profile.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.state, PersonalBindingState::Revoked);
    assert!(
        engine::messages(&j.state.db, &j.owner, &reply.turn.conversation_id, 50, None)
            .await
            .unwrap()
            .iter()
            .any(|m| m.id == saved.id)
    );
    j.webhook(&j.update(14, "after unlink", false)).await;
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 1);
    let metadata = j
        .state
        .db
        .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME)
        .find_one(doc! {"reference.reply_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !metadata.contains_key("body")
            && !metadata.contains_key("text")
            && !metadata.contains_key("token")
    );
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn malformed_and_forwarded_personal_controls_never_fall_through_or_issue_proof() {
    let j = Journey::new().await;
    for (id, text, forwarded) in [
        (20, "/link extra", false),
        (21, "/unlink extra", false),
        (22, "/confirm P-BAD extra", false),
        (23, "/link", true),
        (24, "/confirm P-1234-5678", true),
        (25, "/link@other_bot", false),
    ] {
        j.webhook(&j.update(id, text, forwarded)).await;
    }
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_binding::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        j.state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        6
    );
    assert!(
        j.sends()
            .await
            .iter()
            .all(|s| !s["text"].as_str().unwrap().contains("nyx_pli_"))
    );
    j.state.db.drop().await.unwrap();
}

impl Journey {
    // Establish events before starting, but leave transport delivery unconsumed so
    // the race fixtures control the exact point between settlement and send.
    async fn prepared_reply(&self, id: i64) -> (ReplyDestination, ReplyRef, AssistantMessage) {
        let update = self.update(id, "save before delivery", false);
        let verified =
            dispatch::verified_message(&self.profile.id, "100", 100, "test-observation", &update)
                .unwrap();
        let admission = self
            .producers
            .transport
            .admit_manager_update(&self.state.db, &verified)
            .await
            .unwrap();
        let IngressAdmission::Claimed {
            audience: ManagerAudience::Personal(admitted),
            ..
        } = admission
        else {
            panic!("personal admission required");
        };
        let (events, mut receiver) = tokio::sync::mpsc::channel(4);
        let PersonalTurnStart::Started { destination, .. } = self
            .producers
            .personal
            .start_personal_turn(
                &self.state.db,
                &admitted,
                TurnInput {
                    text: "save before delivery".into(),
                },
                events,
            )
            .await
            .unwrap()
        else {
            panic!("start required");
        };
        let event = tokio::time::timeout(std::time::Duration::from_secs(15), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let ExecutionEvent::ReplyReady(reply) = event else {
            panic!("Saved reference required");
        };
        let saved = self
            .state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"_id":&reply.reply_id})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(ExecutionEvent::Settled { .. })
        ));
        (destination, reply, saved)
    }
    async fn deliver(
        &self,
        destination: &ReplyDestination,
        reply: &ReplyRef,
        body: String,
    ) -> AppResult<DeliveryOutcome> {
        self.producers
            .transport
            .deliver_platform_reply(
                &self.state.db,
                destination,
                &DeliveryRef::Reply(reply.clone()),
                ReplyContent::Text(body),
            )
            .await
    }
}

#[tokio::test]
async fn exact_saved_body_destination_and_revoke_before_send_fail_closed() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, saved) = j.prepared_reply(30).await;
    assert!(
        j.deliver(&destination, &reply, "forged private body".into())
            .await
            .is_err()
    );
    let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
    let mut wrong = destination.clone();
    if let SenderIdentity::TelegramV1 {
        telegram_chat_id, ..
    } = &mut wrong.sender
    {
        *telegram_chat_id = 701;
    } else {
        panic!("legacy fixture");
    }
    assert!(j.deliver(&wrong, &reply, body.clone()).await.is_err());
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME)
            .count_documents(doc! {"reference.reply_id":&saved.id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(j.sends().await.len(), 2);
    let (destination2, reply2, saved2) = j.prepared_reply(31).await;
    j.webhook(&j.update(32, "/unlink", false)).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(matches!(
        j.deliver(
            &destination2,
            &reply2,
            delivery::render_saved(&saved2, &j.state.config.frontend_url)
        )
        .await
        .unwrap(),
        DeliveryOutcome::Refused(_)
    ));
    assert_eq!(j.sends().await.len(), 3);
    assert!(
        engine::messages(
            &j.state.db,
            &j.owner,
            &reply2.turn.conversation_id,
            50,
            None
        )
        .await
        .unwrap()
        .iter()
        .any(|message| message.id == saved2.id)
    );
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn ambiguous_send_is_terminal_and_same_identity_current_observation_is_used() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, saved) = j.prepared_reply(40).await;
    // The admitted receipt has the previous observation. Same numeric bot/epoch
    // must revalidate and capture the current ready credential for this send.
    j.state
        .db
        .collection::<Document>(PROFILES)
        .update_one(
            doc! {"_id":&j.profile.id},
            doc! {"$set":{"observation_id":"same-identity-new-observation"}},
        )
        .await
        .unwrap();
    j.state
        .db
        .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
        .update_one(
            doc! {"provider":"telegram-new"},
            doc! {"$set":{"fields.observation_id":"same-identity-new-observation"}},
        )
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/bot100:manager-test-secret/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ambiguous provider response"))
        .with_priority(1)
        .mount(&j.server)
        .await;
    let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
    assert!(matches!(
        j.deliver(&destination, &reply, body.clone()).await.unwrap(),
        DeliveryOutcome::Uncertain
    ));
    assert!(matches!(
        j.deliver(&destination, &reply, body).await.unwrap(),
        DeliveryOutcome::Duplicate
    ));
    delivery::sweep(
        &j.state,
        j.producers.identity.clone(),
        j.producers.public.clone(),
        100,
    )
    .await
    .unwrap();
    assert_eq!(j.sends().await.len(), 3);
    let row = j
        .state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {"reference.reply_id":saved.id})
        .await
        .unwrap()
        .unwrap();
    assert!(row.state == DeliveryState::Uncertain);
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stopped_reply_and_mismatched_current_token_never_send() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, mut saved) = j.prepared_reply(50).await;
    j.state
        .db
        .collection::<Document>(MESSAGES)
        .update_one(
            doc! {"_id":&saved.id},
            doc! {"$set":{"error_code":"cancelled","status":"stopped"}},
        )
        .await
        .unwrap();
    saved.error_code = Some("cancelled".into());
    saved.status = "stopped".into();
    assert!(matches!(
        j.deliver(
            &destination,
            &reply,
            delivery::render_saved(&saved, &j.state.config.frontend_url)
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(j.sends().await.len(), 2);
    // Numeric identity or epoch changes also fail; an ordinary readiness flag
    // cannot turn an admitted proof into authority for a different manager.
    let (destination, reply, saved) = j.prepared_reply(51).await;
    j.state
        .db
        .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
        .update_one(
            doc! {"provider":"telegram-new"},
            doc! {"$set":{"fields.manager_bot_id":"101"}},
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(matches!(
        j.deliver(
            &destination,
            &reply,
            delivery::render_saved(&saved, &j.state.config.frontend_url)
        )
        .await
        .unwrap(),
        DeliveryOutcome::Refused(_)
    ));
    assert_eq!(j.sends().await.len(), 2);
    j.state.db.drop().await.unwrap();
}

#[test]
fn expected_personal_admission_errors_are_bounded_and_infrastructure_stays_error() {
    assert_eq!(
        public_failure(&AppError::RateLimited),
        Some(UnavailableReason::PublicQuotaLimited)
    );
    assert_eq!(
        public_failure(&AppError::InsufficientCredits),
        Some(UnavailableReason::SponsorUnavailable)
    );
    assert_eq!(
        public_failure(&AppError::Internal("persistence failed".into())),
        None
    );
    assert_eq!(
        expected_failure(&AppError::AssistantTurnActive),
        Some(UnavailableReason::CapacityLimited)
    );
    assert_eq!(
        expected_failure(&AppError::InsufficientCredits),
        Some(UnavailableReason::InsufficientFunds)
    );
    assert_eq!(
        expected_failure(&AppError::Forbidden("engine disabled".into())),
        Some(UnavailableReason::OwnerUnavailable)
    );
    assert_eq!(
        expected_failure(&AppError::Internal("database or persistence failed".into())),
        None
    );
}

#[test]
fn bounded_reply_renderer_links_exact_thread_and_strips_unrelated_query_secrets() {
    let message:AssistantMessage=mongodb::bson::from_document(doc! {"_id":"reply","conversation_id":"nyxa-0123456789abcdef0123456789abcdef","user_id":"owner","seq":2_i64,"turn_id":"turn","role":"assistant","text":"Review the pending action in NyxID.","status":"completed","error_code":bson::Bson::Null,"created_at":bson::DateTime::now()}).unwrap();
    let rendered = delivery::render_saved(
        &message,
        "https://app.nyxid.test/old?secret=discard#discard",
    );
    assert!(rendered.contains("/assistant?c=nyxa-0123456789abcdef0123456789abcdef"));
    assert!(!rendered.contains("discard"));
    let enormous_origin = format!("https://{}.example.test", "a".repeat(4096));
    assert!(
        delivery::render_saved(&message, &enormous_origin)
            .encode_utf16()
            .count()
            <= 4096
    );
}

#[tokio::test]
async fn public_signup_sends_one_protected_ephemeral_bearer_without_retaining_it() {
    let j = Journey::new().await;
    let (context, token) = j.public_signup().await;
    let replay = token.clone();
    assert_eq!(
        j.signup_send(&context, token).await,
        delivery::PublicSignupDeliveryOutcome::Sent
    );
    assert_eq!(
        j.signup_send(&context, replay).await,
        delivery::PublicSignupDeliveryOutcome::Unavailable
    );
    delivery::sweep(
        &j.state,
        j.producers.identity.clone(),
        j.producers.public.clone(),
        100,
    )
    .await
    .unwrap();
    let sends = j.sends().await;
    assert_eq!(sends.len(), 1);
    assert!(
        sends[0]["text"]
            .as_str()
            .unwrap()
            .contains("#intent_token=nyx_pli_")
    );
    assert_eq!(sends[0]["protect_content"], true);
    assert_eq!(sends[0]["link_preview_options"]["is_disabled"], true);
    let stored = j
        .state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        stored.reference,
        crate::models::platform_manager_delivery::DeliveryReference::PublicSignup { .. }
    ));
    for name in j.state.db.list_collection_names().await.unwrap() {
        let mut documents = j
            .state
            .db
            .collection::<Document>(&name)
            .find(doc! {})
            .await
            .unwrap();
        while let Some(document) = documents.try_next().await.unwrap() {
            assert!(!format!("{document:?}").contains("nyx_pli_"));
        }
    }
    assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn public_signup_429_and_ambiguous_outcomes_are_terminal_and_never_replayed() {
    for (body, expected, state) in [
        (
            json!({"ok":false,"error_code":429,"parameters":{"retry_after":1}}),
            delivery::PublicSignupDeliveryOutcome::Unavailable,
            DeliveryState::Refused,
        ),
        (
            json!({"unrecognized":"ambiguous response"}),
            delivery::PublicSignupDeliveryOutcome::Uncertain,
            DeliveryState::Uncertain,
        ),
    ] {
        let j = Journey::new().await;
        let (context, token) = j.public_signup().await;
        j.server.reset().await;
        Mock::given(method("POST"))
            .and(path("/bot100:manager-test-secret/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&j.server)
            .await;
        let replay = token.clone();
        assert_eq!(j.signup_send(&context, token).await, expected);
        assert_eq!(
            j.signup_send(&context, replay).await,
            delivery::PublicSignupDeliveryOutcome::Unavailable
        );
        delivery::sweep(
            &j.state,
            j.producers.identity.clone(),
            j.producers.public.clone(),
            100,
        )
        .await
        .unwrap();
        assert_eq!(j.sends().await.len(), 1);
        let row = j
            .state
            .db
            .collection::<PlatformManagerDelivery>(
                crate::models::platform_manager_delivery::COLLECTION_NAME,
            )
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert!(row.state == state);
        j.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn public_signup_refuses_changed_turn_consent_intent_and_transport_before_send() {
    for mutation in [
        "stop",
        "revoke",
        "personal_pause",
        "intent_cancel",
        "observation",
        "altered_turn",
    ] {
        let j = Journey::new().await;
        let (mut context, token) = j.public_signup().await;
        let sessions = crate::models::platform_public_session::COLLECTION_NAME;
        let (collection, filter, change) = match mutation {
            "stop" => (
                sessions,
                doc! {"_id":&context.public_session_id},
                doc! {"active_turn.stop_requested":true},
            ),
            "revoke" => (
                sessions,
                doc! {"_id":&context.public_session_id},
                doc! {"revoked_at":bson::DateTime::now()},
            ),
            "personal_pause" => (
                crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
                doc! {"platform_config_id":&j.profile.id},
                doc! {"personal_enabled":false},
            ),
            "intent_cancel" => (
                crate::models::platform_link_intent::COLLECTION_NAME,
                doc! {},
                doc! {"state":"cancelled"},
            ),
            "observation" => (
                crate::models::platform_credential::COLLECTION_NAME,
                doc! {"provider":"telegram-new"},
                doc! {"fields.observation_id":"rotating-current-observation"},
            ),
            "altered_turn" => {
                context.turn.turn_id = uuid::Uuid::new_v4().to_string();
                (
                    sessions,
                    doc! {"_id":&context.public_session_id},
                    doc! {"effect_revision":1_i64},
                )
            }
            _ => unreachable!(),
        };
        j.state
            .db
            .collection::<Document>(collection)
            .update_one(filter, doc! {"$set":change})
            .await
            .unwrap();
        assert_eq!(
            j.signup_send(&context, token).await,
            delivery::PublicSignupDeliveryOutcome::Unavailable
        );
        delivery::sweep(
            &j.state,
            j.producers.identity.clone(),
            j.producers.public.clone(),
            100,
        )
        .await
        .unwrap();
        assert!(j.sends().await.is_empty());
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
        j.state.db.drop().await.unwrap();
    }
}

async fn notify_local(j: &Journey, reply: &ReplyRef) -> AppResult<Option<DeliveryOutcome>> {
    let row = engine::get(&j.state.db, &j.owner, &reply.turn.conversation_id).await?;
    delivery::saved_personal_reply_with_api(
        &j.state,
        &row,
        reply,
        &super::super::telegram_new_api::TelegramApi {
            http: &j.state.http_client,
            base_url: &j.server.uri(),
        },
    )
    .await
}

#[tokio::test]
async fn saved_notification_honors_web_only_absent_and_expired_routes() {
    use crate::models::assistant_conversation::PlatformReplyRouting;
    let j = Journey::new().await;
    j.link().await;
    let (_, reply, mut saved) = j.prepared_reply(1200).await;
    let messages = j.state.db.collection::<AssistantMessage>(MESSAGES);
    for route in [None, Some(PlatformReplyRouting::WebOnly)] {
        saved.platform_reply = route;
        messages
            .replace_one(doc! {"_id":&saved.id}, &saved)
            .await
            .unwrap();
        assert!(notify_local(&j, &reply).await.unwrap().is_none());
    }
    let origin = saved.platform_origin.clone().unwrap();
    saved.platform_reply = Some(PlatformReplyRouting::Captured {
        origin: Box::new(origin),
        turn_id: reply.turn.turn_id.clone(),
        expires_at: Utc::now() - Duration::seconds(1),
    });
    messages
        .replace_one(doc! {"_id":&saved.id}, &saved)
        .await
        .unwrap();
    assert!(notify_local(&j, &reply).await.is_err());
    let batch = recover_saved_replies(&j.state, 100).await.unwrap();
    assert_eq!(batch.processed, 1);
    let retained = messages
        .find_one(doc! {"_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert!(retained.platform_delivery_recovery_queued);
    assert_eq!(retained.text, "Retained personal answer");
    assert_eq!(j.sends().await.len(), 2);
    assert!(
        engine::messages(&j.state.db, &j.owner, &reply.turn.conversation_id, 50, None)
            .await
            .unwrap()
            .iter()
            .any(|m| m.id == reply.reply_id)
    );
}

#[tokio::test]
async fn delayed_saved_result_uses_original_receipt_without_replacing_its_reply() {
    let j = Journey::new().await;
    j.link().await;
    let (_, original, mut delayed) = j.prepared_reply(1201).await;
    let original_route = delayed.platform_reply.clone();
    delayed.id = uuid::Uuid::new_v4().to_string();
    delayed.turn_id = uuid::Uuid::new_v4().to_string();
    delayed.seq += 2;
    delayed.text = "A delayed persisted result".into();
    delayed.platform_origin.as_mut().unwrap().source =
        crate::models::assistant_conversation::PlatformOriginSource::ThreadEvent {
            admission_id: delayed.turn_id.clone(),
        };
    assert_eq!(delayed.platform_reply, original_route);
    j.state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .insert_one(&delayed)
        .await
        .unwrap();
    let reply = ReplyRef {
        turn: TurnRef {
            conversation_id: delayed.conversation_id.clone(),
            turn_id: delayed.turn_id.clone(),
        },
        reply_id: delayed.id.clone(),
    };
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(matches!(
        notify_local(&j, &reply).await.unwrap(),
        Some(DeliveryOutcome::Sent { .. })
    ));
    assert!(matches!(
        notify_local(&j, &reply).await.unwrap(),
        Some(DeliveryOutcome::Duplicate)
    ));
    let origin = super::super::platform_personal_turn_service::contract_origin(
        delayed.platform_origin.as_ref().unwrap(),
    )
    .unwrap();
    let original_saved = j
        .state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id":&original.reply_id})
        .await
        .unwrap()
        .unwrap();
    let captured = super::super::platform_personal_turn_service::contract_origin(
        original_saved.platform_origin.as_ref().unwrap(),
    )
    .unwrap();
    assert_ne!(origin.source, captured.source);
    let PlatformOriginSource::Ingress(claim) = captured.source else {
        panic!("ingress required");
    };
    let receipt = j
        .state
        .db
        .collection::<PlatformManagerIngress>(INGRESS)
        .find_one(doc! {"_id":claim.receipt_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.reply_id.as_ref(), Some(&original.reply_id));
    let queued = j
        .state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {"reference.reply_id":&delayed.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.origin_turn_id.as_ref(), Some(&original.turn.turn_id));
    assert_eq!(j.sends().await.len(), 3);
}

#[tokio::test]
async fn genuine_thread_event_saved_delivery_needs_no_ingress_and_keeps_single_claim() {
    use crate::models::assistant_conversation::{
        PlatformOriginSource as ModelSource, PlatformReplyRouting,
    };
    let j = Journey::new().await;
    j.link().await;
    let (_, reply, mut saved) = j.prepared_reply(1202).await;
    let mut source=j.state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {"conversation_id":&reply.turn.conversation_id,"turn_id":&reply.turn.turn_id,"role":"user"}).await.unwrap().unwrap();
    let event_id = uuid::Uuid::new_v4().to_string();
    let mut origin = saved.platform_origin.clone().unwrap();
    origin.source = ModelSource::ThreadEvent {
        admission_id: event_id.clone(),
    };
    let route = PlatformReplyRouting::Captured {
        origin: Box::new(origin.clone()),
        turn_id: event_id.clone(),
        expires_at: mongodb::bson::DateTime::from_chrono(Utc::now() + Duration::minutes(20))
            .to_chrono(),
    };
    source.id = uuid::Uuid::new_v4().to_string();
    source.turn_id = event_id.clone();
    source.role = "event".into();
    source.seq += 2;
    source.platform_origin = Some(origin.clone());
    source.platform_reply = Some(route.clone());
    saved.id = uuid::Uuid::new_v4().to_string();
    saved.turn_id = event_id.clone();
    saved.seq += 2;
    saved.platform_origin = Some(origin);
    saved.platform_reply = Some(route);
    j.state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .insert_many([source, saved.clone()])
        .await
        .unwrap();
    let event_reply = ReplyRef {
        turn: TurnRef {
            conversation_id: saved.conversation_id.clone(),
            turn_id: event_id,
        },
        reply_id: saved.id.clone(),
    };
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (first, second) = tokio::join!(
        notify_local(&j, &event_reply),
        notify_local(&j, &event_reply)
    );
    assert!(first.is_ok() && second.is_ok());
    let queued = j
        .state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {"reference.reply_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert!(queued.ingress_id.is_none() && queued.ingress_claim_id.is_none());
    assert_eq!(
        queued.origin_turn_id.as_ref(),
        Some(&event_reply.turn.turn_id)
    );
    assert_eq!(
        j.state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .count_documents(doc! {"execution.turn_id":&event_reply.turn.turn_id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(j.sends().await.len(), 3);
}

#[tokio::test]
async fn delayed_original_route_is_not_promoted_by_new_personal_control_revision() {
    let j = Journey::new().await;
    j.link().await;
    let (_, reply, saved) = j.prepared_reply(1203).await;
    j.state
        .db
        .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&j.profile.id},
            doc! {"$inc":{"personal_control_revision":1_i64}},
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(matches!(
        notify_local(&j, &reply).await.unwrap(),
        Some(DeliveryOutcome::Refused(UnavailableReason::OriginRevoked))
    ));
    assert_eq!(j.sends().await.len(), 2);
    assert_eq!(
        j.state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"_id":saved.id})
            .await
            .unwrap()
            .unwrap()
            .text,
        "Retained personal answer"
    );
}

#[tokio::test]
async fn public_reply_branch_refuses_missing_retained_public_turn_before_owner_history_lookup() {
    let j = Journey::new().await;
    let (context, _) = j.public_signup().await;
    let destination = ReplyDestination {
        manager: context.ingress.manager.clone(),
        sender: context.ingress.sender.clone(),
        authority: ReplyAuthority::Public {
            ingress: context.ingress.clone(),
            public_session_id: context.public_session_id.clone(),
            turn: context.turn.clone(),
        },
        expires_at: context.expires_at,
    };
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_public_session::COLLECTION_NAME)
            .count_documents(doc! {"_id": &context.public_session_id})
            .await
            .unwrap(),
        1,
        "the session exists; the missing retained public turn must reject delivery"
    );
    assert_eq!(
        j.state
            .db
            .collection::<Document>(crate::models::platform_public_turn::COLLECTION_NAME)
            .count_documents(doc! {"_id": &context.turn.turn_id})
            .await
            .unwrap(),
        0
    );
    let error = deliver_saved(
        &j.state,
        &j.producers,
        &destination,
        &ReplyRef {
            turn: context.turn,
            reply_id: "public-only-reply".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(message) if message == "Public session authority changed")
    );
    assert_eq!(
        j.state
            .db
            .collection::<AssistantMessage>(MESSAGES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(j.sends().await.is_empty());
}

#[tokio::test]
async fn asynchronous_public_failure_sends_only_fixed_notice_and_live_revocations_suppress_it() {
    use crate::models::{
        platform_public_contract::{ACTIVATIONS, PublicDeploymentActivation},
        platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
        platform_public_turn::{COLLECTION_NAME as TURNS, PlatformPublicTurn},
    };
    for change in [
        "unchanged",
        "execution_timeout",
        "manager_stop",
        "sponsor_revoke",
        "sender_revoke",
        "receipt_change",
        "notice_expired",
    ] {
        let j = Journey::new().await;
        // Inject local admitted state, then use real settlement, event consumption
        // and Telegram delivery. This is not a deployed runtime readiness proof.
        let (mut context, _) = j.public_signup().await;
        let session = j
            .state
            .db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id": &context.public_session_id})
            .await
            .unwrap()
            .unwrap();
        let controls =
            super::super::platform_bot_runtime_control_service::get(&j.state.db, &j.profile.id)
                .await
                .unwrap()
                .unwrap();
        j.state
            .db
            .collection::<PublicDeploymentActivation>(ACTIVATIONS)
            .insert_one(PublicDeploymentActivation {
                profile_id: j.profile.id.clone(),
                binding: super::super::platform_public_policy::binding(
                    &super::super::platform_bot_runtime_control_service::policy(&controls),
                )
                .unwrap(),
                policy_digest: context.policy_digest.clone(),
                activated_by: "local-fixture-admin".into(),
                effect_revision: 0,
                activated_at: Utc::now(),
            })
            .await
            .unwrap();
        j.state
            .db
            .collection::<PlatformPublicTurn>(TURNS)
            .insert_one(PlatformPublicTurn {
                secret_context_versions: Default::default(),
                proof_record_id: context.proof_record_id.clone(),
                id: context.turn.turn_id.clone(),
                public_session_id: session.id.clone(),
                conversation_id: context.turn.conversation_id.clone(),
                receipt_id: context.ingress.ingress.receipt_id.clone(),
                claim_id: context.ingress.ingress.claim_id.clone(),
                sponsor_owner_user_id: context.sponsor_owner_user_id.clone(),
                policy_digest: context.policy_digest.clone(),
                credential_api_key_id: session.credential_api_key_id.clone(),
                input_ciphertext: vec![],
                reply_ciphertext: vec![],
                reply_digest: String::new(),
                reply_id: None,
                outcome: None,
                max_output_bytes: context.limits.max_output_bytes,
                nyxagent_service_id: "local-fixture-outer".into(),
                nyxagent_catalog_digest: "local-fixture-pin".into(),
                outer_dispatch_admitted: true,
                expires_at: session.expires_at,
            })
            .await
            .unwrap();
        j.state
            .db
            .collection::<PlatformPublicSession>(SESSIONS)
            .update_one(
                doc! {"_id": &session.id},
                doc! {"$set": {"upstream_may_exist": true}},
            )
            .await
            .unwrap();
        if change == "execution_timeout" {
            context.expires_at = Utc::now() - Duration::seconds(1);
        }
        let failure = engine::TurnResult {
            text: "Model-authored content must never be delivered after revocation".into(),
            session_id: None,
            response_id: None,
            error: Some(engine::TurnError::internal(
                "assistant_unavailable",
                "public_execution",
            )),
        };
        let SettledExecution::Saved(reply) =
            super::super::platform_public_credential_service::producer(j.state.clone())
                .settle(
                    &j.state.db,
                    &context,
                    &uuid::Uuid::new_v4().to_string(),
                    &failure,
                )
                .await
                .unwrap();
        let revoked = j
            .state
            .db
            .collection::<PlatformPublicSession>(SESSIONS)
            .find_one(doc! {"_id": &session.id})
            .await
            .unwrap()
            .unwrap();
        assert!(revoked.revoked_at.is_some() && revoked.credential_ciphertext.is_empty());
        assert_eq!(revoked.expires_at, session.expires_at);
        match change {
            "manager_stop" => {
                let mut txn = j.state.db.client().start_session().await.unwrap();
                txn.start_transaction().await.unwrap();
                super::super::platform_bot_runtime_control_service::emergency_stop_in_session(
                    &j.state.db,
                    &mut txn,
                    &j.profile.id,
                    false,
                )
                .await
                .unwrap();
                txn.commit_transaction().await.unwrap();
            }
            "sponsor_revoke" | "sender_revoke" => {
                let scope = if change == "sponsor_revoke" {
                    PublicRevocationScope::Sponsor {
                        sponsor_owner_user_id: context.sponsor_owner_user_id.clone(),
                    }
                } else {
                    PublicRevocationScope::Sender {
                        manager: context.ingress.manager.clone(),
                        sender: context.ingress.sender.clone(),
                    }
                };
                let mut txn = j.state.db.client().start_session().await.unwrap();
                txn.start_transaction().await.unwrap();
                super::super::platform_public_agent_service::revoke_in_session(
                    &j.state.db,
                    &mut txn,
                    &scope,
                    PublicRevocationReason::PersonalLinkConfirmed,
                )
                .await
                .unwrap();
                txn.commit_transaction().await.unwrap();
            }
            "receipt_change" => {
                j.state
                    .db
                    .collection::<Document>(INGRESS)
                    .update_one(
                        doc! {"_id": &context.ingress.ingress.receipt_id},
                        doc! {"$set": {"claim_id": "replaced-claim"}},
                    )
                    .await
                    .unwrap();
            }
            "notice_expired" => {
                j.state.db.collection::<Document>(INGRESS).update_one(doc! {"_id": &context.ingress.ingress.receipt_id},
                    doc! {"$set": {"settled_at": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(3))}}).await.unwrap();
            }
            _ => {}
        }
        let destination = ReplyDestination {
            manager: context.ingress.manager.clone(),
            sender: context.ingress.sender.clone(),
            authority: ReplyAuthority::Public {
                ingress: context.ingress.clone(),
                public_session_id: session.id,
                turn: context.turn.clone(),
            },
            expires_at: session.expires_at,
        };
        let (events, receiver) = tokio::sync::mpsc::channel(2);
        events
            .send(ExecutionEvent::ReplyReady(reply.clone()))
            .await
            .unwrap();
        events
            .send(ExecutionEvent::Settled {
                turn: context.turn.clone(),
            })
            .await
            .unwrap();
        drop(events);
        consume_events(
            j.state.clone(),
            j.producers.clone(),
            destination.clone(),
            receiver,
        )
        .await;
        let sends = j.sends().await;
        if matches!(change, "unchanged" | "execution_timeout") {
            assert_eq!(sends.len(), 1);
            assert_eq!(
                sends[0]["text"],
                delivery::fixed_text(UnavailableReason::PublicTurnFailed)
            );
            // The saved failure reference is idempotent and cannot emit another notice.
            let _ = deliver_saved(&j.state, &j.producers, &destination, &reply).await;
            assert_eq!(j.sends().await.len(), 1);
            let receipt = j
                .state
                .db
                .collection::<PlatformManagerIngress>(INGRESS)
                .find_one(doc! {"_id": &context.ingress.ingress.receipt_id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                receipt.delivery_recovery_queued,
                "durable fixed delivery owns recovery"
            );
        } else {
            assert!(
                sends.is_empty(),
                "{change} must suppress the fixed failure notice"
            );
        }
        assert_eq!(
            j.state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert!(
            !j.state
                .db
                .collection::<crate::models::api_key::ApiKey>(
                    crate::models::api_key::COLLECTION_NAME
                )
                .find_one(doc! {"_id": &revoked.credential_api_key_id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
    }
}

#[tokio::test]
async fn pending_action_renderer_uses_exact_turn_key_routing_and_unique_codes() {
    use crate::models::assistant_acknowledgement::{
        AssistantAcknowledgement, COLLECTION_NAME as ACKS,
    };
    let j = Journey::new().await;
    j.link().await;
    let (_, reply, saved) = j.prepared_reply(1204).await;
    let row = engine::get(&j.state.db, &j.owner, &reply.turn.conversation_id)
        .await
        .unwrap();
    let card = |id: &str, turn: &str, key: &str, routing: bson::Bson| -> AssistantAcknowledgement {
        bson::from_document(doc! {"_id":id,"conversation_id":&row.id,"user_id":&j.owner,"api_key_id":key,
            "kind":"action","summary":"Create the requested report","status":"pending","decider":"user",
            "requested_turn_id":turn,"created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10)),"routing":routing}).unwrap()
    };
    let exact = card(
        "00000100-0000-4000-8000-000000000001",
        &reply.turn.turn_id,
        &row.credential_api_key_id,
        bson::Bson::Document(doc! {"kind":"not_involved"}),
    );
    j.state
        .db
        .collection::<AssistantAcknowledgement>(ACKS)
        .insert_one(&exact)
        .await
        .unwrap();
    let text = delivery::render_personal(&j.state, &saved, None)
        .await
        .unwrap();
    let code = super::super::assistant_acknowledgement_service::confirm_code(&exact.id);
    assert!(text.contains(&format!("yes {code}")) && text.contains(&format!("no {code}")));
    assert!(text.contains("Create the requested report"));
    assert!(text.contains(&format!("c={}", row.id)));
    for (field, value) in [
        ("routing", bson::Bson::Null),
        ("api_key_id", bson::Bson::String("different-key".into())),
        ("requested_turn_id", bson::Bson::String("other-turn".into())),
    ] {
        let original = bson::to_document(&exact).unwrap();
        let mut modified = original.clone();
        modified.insert(field, value);
        j.state
            .db
            .collection::<Document>(ACKS)
            .replace_one(doc! {"_id":&exact.id}, modified)
            .await
            .unwrap();
        let text = delivery::render_personal(&j.state, &saved, None)
            .await
            .unwrap();
        assert!(!text.contains(&format!("yes {code}")));
        assert!(text.contains("Pending actions require review"));
        j.state
            .db
            .collection::<Document>(ACKS)
            .replace_one(doc! {"_id":&exact.id}, original)
            .await
            .unwrap();
    }
    let colliding = "00271100-0000-4000-8000-000000000002";
    assert_eq!(
        super::super::assistant_acknowledgement_service::confirm_code(colliding),
        code
    );
    let collision = card(
        colliding,
        &reply.turn.turn_id,
        &row.credential_api_key_id,
        bson::Bson::Document(doc! {"kind":"not_involved"}),
    );
    j.state
        .db
        .collection::<AssistantAcknowledgement>(ACKS)
        .insert_one(collision)
        .await
        .unwrap();
    assert!(
        !delivery::render_personal(&j.state, &saved, None)
            .await
            .unwrap()
            .contains(&format!("yes {code}"))
    );
    // Only the canonical renderer may be sent; old text without pending card
    // notices cannot bypass final card/body validation.
    assert!(
        j.deliver(
            &delivery::captured_destination(&saved).unwrap().unwrap(),
            &reply,
            delivery::render_saved(&saved, &j.state.config.frontend_url)
        )
        .await
        .is_err()
    );
    assert_eq!(j.sends().await.len(), 2);
}

#[tokio::test]
async fn retired_owner_or_control_authority_can_logout_to_public_without_revival() {
    use crate::models::platform_personal_binding::PersonalRevocationReason;
    use commands::{ChatLoginStatus as Status, PersonalCommandReply as Reply};
    for reason in [
        PersonalRevocationReason::OwnerDisabled,
        PersonalRevocationReason::OwnerDeleted,
        PersonalRevocationReason::Replaced,
    ] {
        let j = Journey::new().await;
        j.confirm_login(600).await;
        let binding = j.state.db.collection::<PlatformPersonalBinding>(crate::models::platform_personal_binding::COLLECTION_NAME)
            .find_one(doc! {"platform_config_id":&j.profile.id,"telegram_sender_id":700_i64,"state":"active"}).await.unwrap().unwrap();
        if matches!(
            reason,
            PersonalRevocationReason::OwnerDisabled | PersonalRevocationReason::OwnerDeleted
        ) {
            j.state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(doc! {"_id":&j.owner}, doc! {"$set":{"is_active":false}})
                .await
                .unwrap();
            super::super::platform_personal_owner_lifecycle::retire(
                &j.state.db,
                &j.owner,
                reason == PersonalRevocationReason::OwnerDeleted,
            )
            .await
            .unwrap();
        } else {
            let effects = bindings::LifecycleProducer {
                personal: j.producers.personal.clone(),
                public: j.producers.public.clone(),
            };
            let mut session = j.state.db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            bindings::revoke_binding_in_session(
                &j.state.db,
                &mut session,
                &binding,
                reason,
                &effects,
                None,
            )
            .await
            .unwrap();
            session.commit_transaction().await.unwrap();
        }
        assert!(matches!(
            j.control(602, "/status").await.unwrap().reply,
            Reply::Status(Status::Unavailable)
        ));
        // A fresh proof may be pending even though old personal authority retired.
        let pending = j.control(603, "/login").await.unwrap();
        assert!(matches!(pending.reply, Reply::Link { .. }));
        j.control(604, "/logout").await.unwrap();
        assert!(matches!(
            j.control(605, "/status").await.unwrap().reply,
            Reply::Status(Status::LoggedOut)
        ));
        let retired = j
            .state
            .db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&binding.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retired.state, PersonalBindingState::Revoked);
        assert_eq!(retired.revocation_reason, Some(reason));
        assert!(retired.logged_out_at.is_some());
        assert_eq!(
            j.state
                .db
                .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
                .count_documents(doc! {"platform_config_id":&j.profile.id,"state":"pending"})
                .await
                .unwrap(),
            0
        );
        j.webhook(&j.update(606, "What is NyxID?", false)).await;
        let receipt = j
            .state
            .db
            .collection::<PlatformManagerIngress>(INGRESS)
            .find_one(doc! {"profile_id":&j.profile.id,"update_id":606_i64})
            .await
            .unwrap()
            .unwrap();
        // This fixture has no public runtime enabled: reaching PublicDisabled
        // proves unlinked/public classification instead of old owner authority.
        assert!(
            matches!(receipt.audience, IngressAudience::Fixed { reason } if reason == "public_disabled")
        );
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn login_delivery_rejects_substituted_bearer_for_committed_intent() {
    let j = Journey::new().await;
    let mut target = j.control(650, "/login").await.unwrap();
    let other = j.control_as(651, "/login", 701).await.unwrap();
    target.reply = other.reply;
    assert!(matches!(
        j.send_control(target).await,
        DeliveryOutcome::Refused(UnavailableReason::OriginRevoked)
    ));
    assert!(j.sends().await.is_empty());
}

/// Real auth, persisted proof/binding and trusted local Telegram ingress. No
/// real email/provider/model/send path is used by this regression.
#[tokio::test]
async fn default_and_configured_signup_login_mfa_review_and_same_chat_confirmation() {
    use crate::handlers::auth::{self, LoginRequest, RegisterRequest};
    use axum::{
        Json,
        body::{Body, to_bytes},
        extract::{ConnectInfo, State},
        http::{HeaderMap, Request, StatusCode},
    };
    use tower::ServiceExt;
    async fn api(app: &axum::Router, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1{path}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    for auto_verify in [false, true] {
        let mut j = Journey::new().await;
        j.state.config.email_auth_enabled = true;
        j.state.config.auto_verify_email = auto_verify;
        let login = j.control(3000, "/login").await.unwrap();
        let commands::PersonalCommandReply::Link { token: intent } = login.reply else {
            panic!("login must issue intent");
        };
        let email = format!("{}@signup.test", uuid::Uuid::new_v4());
        let password = zeroize::Zeroizing::new(format!("Signup-1!{}", uuid::Uuid::new_v4()));
        let invite = super::super::invite_code_service::create_invite_code(
            &j.state.db,
            &j.owner,
            1,
            Some("Local personal signup journey"),
        )
        .await
        .unwrap();
        let peer = ConnectInfo("127.0.0.1:45678".parse().unwrap());
        let Json(registered) = auth::register(
            State(j.state.clone()),
            peer,
            HeaderMap::new(),
            Json(RegisterRequest {
                email: email.clone(),
                password: password.to_string(),
                invite_code: Some(invite.code),
                display_name: Some("Signup visitor".into()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            registered.message,
            "Registration processed. You can now sign in."
        );
        let user = j
            .state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&registered.user_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.email_verified, auto_verify);
        assert_eq!(
            j.state
                .db
                .collection::<PlatformPersonalBinding>(
                    crate::models::platform_personal_binding::COLLECTION_NAME
                )
                .count_documents(doc! {"state":"active"})
                .await
                .unwrap(),
            0,
            "signup alone is not chat login"
        );
        let (_, Json(logged_in)) = auth::login(
            State(j.state.clone()),
            peer,
            HeaderMap::new(),
            Json(LoginRequest {
                email: email.clone(),
                password: password.to_string(),
                mfa_code: None,
                client: Some("token".into()),
            }),
        )
        .await
        .unwrap();
        let mut access = zeroize::Zeroizing::new(logged_in.access_token.unwrap());
        let (_, private) = crate::routes::build_router_with_state(j.state.clone());
        let app = private.with_state(j.state.clone());
        if auto_verify {
            let (status, setup) = api(&app, "/auth/mfa/setup", &access, json!({})).await;
            assert_eq!(status, StatusCode::OK);
            let totp = totp_rs::TOTP::new(
                totp_rs::Algorithm::SHA1,
                6,
                1,
                30,
                totp_rs::Secret::Encoded(setup["secret"].as_str().unwrap().to_owned())
                    .to_bytes()
                    .unwrap(),
                Some("NyxID".into()),
                registered.user_id.clone(),
            )
            .unwrap();
            assert_eq!(
                api(
                    &app,
                    "/auth/mfa/confirm",
                    &access,
                    json!({"code":totp.generate_current().unwrap()})
                )
                .await
                .0,
                StatusCode::OK
            );
            let pending = auth::login(
                State(j.state.clone()),
                peer,
                HeaderMap::new(),
                Json(LoginRequest {
                    email: email.clone(),
                    password: password.to_string(),
                    mfa_code: None,
                    client: Some("token".into()),
                }),
            )
            .await
            .unwrap_err();
            let AppError::MfaRequired { session_token } = pending else {
                panic!("configured MFA must challenge");
            };
            let pending = zeroize::Zeroizing::new(session_token);
            assert_eq!(
                api(
                    &app,
                    "/platform-personal-links/review",
                    &pending,
                    json!({"intent_token":intent.as_str()})
                )
                .await
                .0,
                StatusCode::UNAUTHORIZED
            );
            // Exclude every accepted skew code, plus adjacent time steps so a
            // boundary crossed during the handler cannot turn this into a pass.
            let step = Utc::now().timestamp() as u64 / 30;
            let excluded = (step.saturating_sub(2)..=step + 2)
                .map(|value| totp.generate(value * 30))
                .collect::<std::collections::HashSet<_>>();
            let wrong = (0..1_000_000)
                .map(|value| format!("{value:06}"))
                .find(|code| !excluded.contains(code))
                .unwrap();
            assert!(!totp.check_current(&wrong).unwrap());
            let sessions = j
                .state
                .db
                .collection::<bson::Document>(crate::models::session::COLLECTION_NAME);
            let authenticated_filter = crate::services::session_authority::authenticated(
                doc! {"user_id":&registered.user_id,"revoked":false},
            );
            let before = sessions
                .count_documents(authenticated_filter.clone())
                .await
                .unwrap();
            let rejected = crate::handlers::mfa::verify(
                State(j.state.clone()),
                peer,
                HeaderMap::new(),
                Json(crate::handlers::mfa::MfaLoginVerifyRequest {
                    code: wrong.into(),
                    mfa_token: Some(pending.to_string().into()),
                    client: Some("token".into()),
                }),
            )
            .await;
            assert!(
                matches!(rejected, Err(AppError::AuthenticationFailed(message)) if message == "Invalid MFA code")
            );
            assert_eq!(
                sessions
                    .count_documents(authenticated_filter)
                    .await
                    .unwrap(),
                before
            );
            let pending_hash = crate::crypto::token::hash_token(&pending);
            assert!(
                j.state
                    .db
                    .collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
                    .find_one(crate::services::session_authority::pending(
                        doc! {"user_id":&registered.user_id,"token_hash":&pending_hash,"revoked":false,
                            "expires_at":{"$gt":bson::DateTime::now()}},
                        false,
                    ))
                    .await
                    .unwrap()
                    .is_some(),
                "an invalid factor must leave the dedicated MFA challenge redeemable"
            );
            assert!(
                sessions
                    .find_one(doc! {"token_hash":&pending_hash})
                    .await
                    .unwrap()
                    .is_none(),
                "a pending MFA challenge must never be an ordinary session"
            );
            assert_eq!(
                j.state
                    .db
                    .collection::<PlatformPersonalBinding>(
                        crate::models::platform_personal_binding::COLLECTION_NAME
                    )
                    .count_documents(doc! {"state":"active"})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                api(
                    &app,
                    "/platform-personal-links/review",
                    &pending,
                    json!({"intent_token":intent.as_str()})
                )
                .await
                .0,
                StatusCode::UNAUTHORIZED
            );
            // Enrollment consumed the current step and TOTP counters are single
            // use, so log in with the next step (inside the one-step skew).
            let code = totp.generate((Utc::now().timestamp() as u64 / 30 + 1) * 30);
            let (_, Json(verified)) = crate::handlers::mfa::verify(
                State(j.state.clone()),
                peer,
                HeaderMap::new(),
                Json(crate::handlers::mfa::MfaLoginVerifyRequest {
                    code: code.into(),
                    mfa_token: Some(pending.to_string().into()),
                    client: Some("token".into()),
                }),
            )
            .await
            .unwrap();
            access = zeroize::Zeroizing::new(verified.access_token.unwrap());
        }
        let (status, review) = api(
            &app,
            "/platform-personal-links/review",
            &access,
            json!({"intent_token":intent.as_str()}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(review["account"]["id"], registered.user_id);
        assert_eq!(review["billing"], "personal");
        assert_eq!(
            j.control(3001, "/status").await.unwrap().effect,
            commands::ManagerCommandEffect::Status {
                state: commands::ChatLoginStatus::Pending
            }
        );
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let (status,approved) = api(&app,"/platform-personal-links/approve",&access,json!({"challenge_id":review["challenge_id"],"review_digest":review["review_digest"],"replace_existing":false})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(approved["status"], "awaiting_chat_confirmation");
        assert_eq!(
            j.state
                .db
                .collection::<PlatformPersonalBinding>(
                    crate::models::platform_personal_binding::COLLECTION_NAME
                )
                .count_documents(doc! {"state":"active"})
                .await
                .unwrap(),
            0,
            "browser review alone is not chat login"
        );
        let command = approved["confirmation_command"].as_str().unwrap();
        assert!(matches!(
            j.control(3002, command).await.unwrap().effect,
            commands::ManagerCommandEffect::Confirmed { .. }
        ));
        let binding = j
            .state
            .db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"state":"active"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(binding.user_id, registered.user_id);
        assert_eq!(
            crate::services::platform_entry_scope::legacy_sender_id(&binding.sender),
            700
        );
        assert_eq!(
            j.control(3003, "/status").await.unwrap().effect,
            commands::ManagerCommandEffect::Status {
                state: commands::ChatLoginStatus::LoggedIn
            }
        );
        j.state.db.drop().await.unwrap();
    }
}
