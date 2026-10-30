//! Transport -> engine -> authenticated model/service callbacks -> settled owner billing.
//! Platform network delivery is intercepted at the existing adapter boundary.
use super::*;
use crate::models::{
    assistant_agent::AssistantAgent,
    usage_meter::{CredentialClass, UsageMeterRow, UsageStatus},
};
use crate::services::channel_platform::{
    BotCredentials, BotIdentity, InboundMessage, MediaCapabilities, OutboundCapabilities,
    OutboundReply, PlatformAdapter, PlatformVerifySecrets,
};

const GUEST_PARTITION: &str = "conv_fedcba9876543210fedcba9876543210";

#[derive(Clone, Copy, Debug)]
enum Scenario {
    OwnerPlatform,
    OwnerByok,
    OwnerByokOrgService,
    FreshPlatform,
    ColdByok,
    FailedSetup,
    EnabledVisitors,
    NoPersonalRow,
    RetainedPlatformKey,
    DeepSeek,
    Concurrent,
    RotateCredential,
    MappingChange,
    ProfileChange,
    RecoveredQueue,
    Merchant,
}
impl Scenario {
    fn platform(self) -> bool {
        matches!(
            self,
            Self::OwnerPlatform
                | Self::FreshPlatform
                | Self::EnabledVisitors
                | Self::NoPersonalRow
                | Self::RetainedPlatformKey
                | Self::DeepSeek
        )
    }
    fn owner_first(self) -> bool {
        matches!(
            self,
            Self::OwnerPlatform | Self::OwnerByok | Self::OwnerByokOrgService
        )
    }
}

macro_rules! channel_cases {
    ($($name:ident => ($platform:literal, $transport:literal, $group:literal)),+ $(,)?) => {$ (
        #[tokio::test]
        async fn $name() {
            for scenario in [Scenario::OwnerPlatform, Scenario::OwnerByok, Scenario::FreshPlatform, Scenario::ColdByok] {
                Box::pin(channel_model_turns($platform, $transport, $group, scenario)).await;
            }
        }
    )+};
}
channel_cases! {
    guest_1995_telegram_new_gateway_private => ("telegram-new", "gateway", false),
    guest_1995_telegram_new_gateway_group => ("telegram-new", "gateway", true),
    guest_1995_telegram_relay_private => ("telegram", "direct", false),
    guest_1995_telegram_relay_group => ("telegram", "direct", true),
    guest_1995_lark_gateway_private => ("lark", "gateway", false),
    guest_1995_lark_gateway_group => ("lark", "gateway", true),
    guest_1995_lark_relay_private => ("lark", "direct", false),
    guest_1995_lark_relay_group => ("lark", "direct", true),
    guest_1995_feishu_gateway_private => ("feishu", "gateway", false),
    guest_1995_feishu_gateway_group => ("feishu", "gateway", true),
    guest_1995_feishu_relay_private => ("feishu", "direct", false),
    guest_1995_feishu_relay_group => ("feishu", "direct", true),
    guest_1995_discord_gateway_private => ("discord", "gateway", false),
    guest_1995_discord_gateway_group => ("discord", "gateway", true),
    guest_1995_slack_gateway_private => ("slack", "gateway", false),
    guest_1995_slack_gateway_group => ("slack", "gateway", true),
    guest_1995_whatsapp_gateway_private => ("whatsapp", "gateway", false),
    guest_1995_x_gateway_private => ("x", "gateway", false),
    guest_1995_aurinko_gateway_private => ("aurinko", "gateway", false),
    guest_1995_discord_relay_private => ("discord", "direct", false),
    guest_1995_discord_relay_group => ("discord", "direct", true),
    guest_1995_slack_relay_private => ("slack", "direct", false),
    guest_1995_slack_relay_group => ("slack", "direct", true),
    guest_1995_whatsapp_relay_private => ("whatsapp", "direct", false),
    guest_1995_x_relay_private => ("x", "direct", false),
    guest_1995_aurinko_relay_private => ("aurinko", "direct", false),
}

#[tokio::test]
async fn guest_1995_org_bot_lark_group_bills_linked_specialist_owner() {
    for scenario in [
        Scenario::OwnerPlatform,
        Scenario::OwnerByok,
        Scenario::FreshPlatform,
        Scenario::ColdByok,
    ] {
        Box::pin(channel_model_turns_for(
            "lark", "direct", true, scenario, true,
        ))
        .await;
    }
}

#[tokio::test]
async fn guest_1995_org_byok_service_bills_agent_owner() {
    Box::pin(channel_model_turns(
        "slack",
        "direct",
        false,
        Scenario::OwnerByokOrgService,
    ))
    .await;
}

struct ReplyAdapter {
    platform: &'static str,
    sent: Calls,
}
#[async_trait::async_trait]
impl PlatformAdapter for ReplyAdapter {
    fn platform_id(&self) -> &str {
        self.platform
    }
    fn outbound_capabilities(&self) -> OutboundCapabilities {
        OutboundCapabilities {
            initiated_send: true,
            reply_to: true,
            thread: false,
            edit: false,
        }
    }
    fn media_capabilities(&self) -> MediaCapabilities {
        MediaCapabilities::NONE
    }
    async fn verify_webhook(
        &self,
        _: &crate::models::channel_bot::ChannelBot,
        _: Option<&PlatformVerifySecrets>,
        _: &HeaderMap,
        _: &[u8],
    ) -> AppResult<()> {
        unreachable!("verified relay envelope")
    }
    async fn parse_inbound(&self, _: &[u8]) -> AppResult<Vec<InboundMessage>> {
        unreachable!("verified relay envelope")
    }
    async fn register_webhook(
        &self,
        _: &reqwest::Client,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<()> {
        unreachable!()
    }
    async fn verify_bot_token(
        &self,
        _: &reqwest::Client,
        _: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        unreachable!()
    }
    async fn send_reply(
        &self,
        _: &reqwest::Client,
        _: &BotCredentials<'_>,
        chat: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        self.sent.lock().await.push(
            json!({"chat":chat,"text":reply.text,"reply_to":reply.reply_to_platform_message_id}),
        );
        Ok(Some(Uuid::new_v4().to_string()))
    }
}
struct ReplyGuard(String);
impl Drop for ReplyGuard {
    fn drop(&mut self) {
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&self.0);
        TEST_BOT_USER_IDS.lock().unwrap().remove(&self.0);
    }
}

async fn distinct_notice_bot(
    state: &AppState,
    channel: &mut NyxbotChannel,
    platform: &'static str,
    replies: &Calls,
) -> ReplyGuard {
    let bots = state.db.collection::<bson::Document>("channel_bots");
    let mut bot = bots
        .find_one(doc! {"_id": &channel.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    // Channel lists intentionally collapse attempts for the same bot. These
    // fixtures represent separate attached bots, not competing connection rows.
    channel.channel_bot_id = Uuid::new_v4().to_string();
    bot.insert("_id", &channel.channel_bot_id);
    bot.insert(
        "platform_bot_id",
        format!("fixture-{}", channel.channel_bot_id),
    );
    bots.insert_one(bot).await.unwrap();
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            channel.channel_bot_id.clone(),
            Arc::new(ReplyAdapter {
                platform,
                sent: replies.clone(),
            }),
        );
    ReplyGuard(channel.channel_bot_id.clone())
}

async fn connection(
    state: &AppState,
    owner: &str,
    provider: &str,
    slug: &str,
    platform: bool,
    inference: bool,
) -> (String, String) {
    use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = slug.into();
    catalog.base_url = provider.into();
    catalog.auth_method = "bearer".into();
    catalog.service_category = if platform { "internal" } else { "connection" }.into();
    catalog.requires_user_credential = !platform;
    catalog.credential_encrypted.clear();
    catalog.inference = inference
        .then(|| serde_json::from_value(json!({"wire_protocol":"openai_responses"})).unwrap());
    catalog.billing = Some(crate::models::service_billing::ServiceBilling {
        platform_billable: true,
        platform_charge_nyxid_credentials_only: false,
        ..Default::default()
    });
    if platform {
        catalog.platform_key =
            Some(serde_json::from_value(json!({"enabled":true,"audience":"public"})).unwrap());
        catalog.credential_encrypted = state
            .encryption_keys
            .encrypt(b"test-platform")
            .await
            .unwrap();
    }
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    let id =
        crate::services::assistant_authority_tests::connected(&state.db, owner, slug, provider)
            .await;
    let mut fields = doc! {"catalog_service_id":&catalog.id,"auth_method":"bearer","credential_binding":if platform {"platform"} else {"user"}};
    if platform {
        fields.insert("source", "auto_provision");
    } else {
        let key = create_api_key(
            &state.db,
            &state.encryption_keys,
            owner,
            CreateApiKeyParams {
                label: slug,
                credential_type: "bearer",
                credential: "test-byok",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        fields.insert("api_key_id", key.id);
    }
    state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$set":fields})
        .await
        .unwrap();
    (catalog.id, id)
}

async fn channel_model_turns(
    platform: &'static str,
    transport: &str,
    group: bool,
    scenario: Scenario,
) {
    Box::pin(channel_model_turns_for(
        platform, transport, group, scenario, false,
    ))
    .await;
}

async fn channel_model_turns_for(
    platform: &'static str,
    transport: &str,
    group: bool,
    scenario: Scenario,
    org_bot: bool,
) {
    use crate::services::assistant_team_service::{CreateRequest, GrantTargets, create_specialist};
    let (mut state, _, old_server) = setup("guest_1995_channels").await;
    old_server.abort();
    let deepseek = matches!(scenario, Scenario::DeepSeek);
    let profile = if deepseek {
        "nyxagent/channel"
    } else if matches!(scenario, Scenario::Merchant) {
        "nyxagent/chat"
    } else {
        "nyxagent/research"
    };
    let slug = if deepseek {
        "llm-deepseek"
    } else {
        "chrono-llm-public"
    };
    let model = if deepseek {
        "deepseek-flash"
    } else if matches!(scenario, Scenario::Merchant) {
        "gpt-5.6-sol"
    } else {
        "gpt-6-astra"
    };
    // Turn keys belong to OWNER even if a channel sender also has a NyxID account.
    let guest_person = "87654321-4321-4321-8321-ba0987654321";
    state
        .db
        .collection(USERS)
        .insert_one(test_user(guest_person, UserType::Person))
        .await
        .unwrap();
    crate::test_utils::set_test_approval_preference(&state.db, OWNER, false).await;
    crate::billing_integration_tests::create_usage_index(&state.db).await;
    state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        1000,
    ));
    for metric in ["platform_tokens", "platform_requests", "platform_bytes"] {
        state
            .db
            .collection("billing_rate_cache")
            .insert_one(crate::billing_integration_tests::rate(metric, Utc::now()))
            .await
            .unwrap();
    }
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any()).respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
        "id":"model-response","model":"gpt-6-astra","output":[],"usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}
    }))).mount(&provider).await;
    let (model_id, model_connection) = connection(
        &state,
        OWNER,
        &provider.uri(),
        slug,
        scenario.platform(),
        true,
    )
    .await;
    if deepseek {
        state
            .db
            .collection::<bson::Document>(SERVICES)
            .update_one(
                doc! {"_id":&model_id},
                doc! {"$set":{"inference.wire_protocol":"openai_completions"}},
            )
            .await
            .unwrap();
    }
    if matches!(scenario, Scenario::RetainedPlatformKey) {
        state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&model_connection},
                doc! {"$set":{"api_key_id":Uuid::new_v4().to_string()}},
            )
            .await
            .unwrap();
    }
    if matches!(scenario, Scenario::NoPersonalRow) {
        state
            .db
            .collection::<bson::Document>("user_services")
            .delete_one(doc! {"_id":&model_connection})
            .await
            .unwrap();
    }
    let tool_owner = if matches!(scenario, Scenario::OwnerByokOrgService) {
        use crate::models::org_membership::{OrgMembership, OrgRole};
        let org = Uuid::new_v4().to_string();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        state
            .db
            .collection::<OrgMembership>("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org,
                OWNER,
                OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        crate::test_utils::set_test_approval_preference(&state.db, &org, false).await;
        org
    } else {
        OWNER.to_owned()
    };
    // Create credentials under their real owner: encrypted fields bind that
    // owner as AAD and cannot be moved between personal/organization rows.
    let (tool_id, tool_connection) = connection(
        &state,
        &tool_owner,
        &provider.uri(),
        "guest-service",
        scenario.platform(),
        false,
    )
    .await;
    // Configure the profile before creation, including the initial home thread.
    let nyxbot = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    state
        .db
        .collection::<AssistantAgent>("assistant_agents")
        .update_one(doc! {"_id": &nyxbot.id}, doc! {"$set": {"model": profile}})
        .await
        .unwrap();
    let (specialist, _) = create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        CreateRequest {
            machines: None,
            logins: None,
            name: "channel-specialist".into(),
            description: "Customer service".into(),
            display_name: None,
            persona: None,
            targets: GrantTargets {
                service_ids: if matches!(scenario, Scenario::Merchant) {
                    vec![]
                } else {
                    vec![tool_connection.clone()]
                },
                platform_service_ids: vec![],
                slugs: vec![],
                ids_by_request: Default::default(),
            },
            account_read: false,
            specialty: None,
            created_by: "nyxbot",
        },
    )
    .await
    .unwrap()
    .unwrap();
    if matches!(scenario, Scenario::Merchant) {
        state
            .db
            .collection::<bson::Document>("assistant_agents")
            .update_one(
                doc! {"_id":&specialist.id},
                doc! {"$set":{"access_mode_override":"ask_every_time"}},
            )
            .await
            .unwrap();
    }
    let (mut channel, gateway_key) = channel(&state, transport).await;
    channel.platform = platform.into();
    if matches!(platform, "lark" | "feishu") {
        channel.gateway_bot_id = Some("ou_bot".into());
    }
    // Discord provider events must name the bound gateway channel and carry
    // snowflake actor/conversation IDs.
    let discord_gateway = transport == "gateway" && canonical_platform(platform) == "discord";
    if discord_gateway {
        channel.gateway_channel_id = Some("cmaeg1.ch.test".into());
        channel.gateway_bot_id = Some("100".into());
    }
    if org_bot {
        use crate::models::org_membership::{OrgMembership, OrgRole};
        let org = Uuid::new_v4().to_string();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        state
            .db
            .collection::<OrgMembership>("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org,
                OWNER,
                OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&channel.route_api_key_id},
                doc! {"$set":{"user_id":&org}},
            )
            .await
            .unwrap();
        channel.bot_owner_id = Some(org);
    }
    channel.owner_sender_ids = vec!["7".into()];
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&channel.id}, &channel)
        .await
        .unwrap();
    let mut bot = bot_doc(platform, "Helper bot");
    bot.insert("_id", &channel.channel_bot_id);
    bot.insert("user_id", bot_owner(&channel));
    bot.insert(
        "bot_token_encrypted",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: state
                .encryption_keys
                .encrypt(b"test-bot-token")
                .await
                .unwrap(),
        },
    );
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .replace_one(doc! {"_id":&channel.channel_bot_id}, bot)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .update_one(
            doc! {"_id":channel.route_id.as_deref().unwrap()},
            doc! {"$set":{"platform":platform,"user_id":bot_owner(&channel)}},
        )
        .await
        .unwrap();
    link(&state, OWNER, &channel.id, &specialist).await.unwrap();
    // Defer the real audience-setting write until the fake NyxAgent is serving.
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&channel.id},
            doc! {"$set":{"private_chats":"everyone"}},
        )
        .await
        .unwrap();
    channel = load_channel(&state, OWNER, &channel.id).await.unwrap();
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .insert(channel.channel_bot_id.clone(), "ou_bot".into());
    let replies: Calls = Default::default();
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            channel.channel_bot_id.clone(),
            Arc::new(ReplyAdapter {
                platform,
                sent: replies.clone(),
            }),
        );
    let _reply_guard = ReplyGuard(channel.channel_bot_id.clone());
    if transport == "gateway" {
        assert_eq!(
            put_binding(
                State(state.clone()),
                Path("bnd_chats".into()),
                bearer(&gateway_key),
                Json(binding_body(&gateway_key, OWNER))
            )
            .await
            .status(),
            StatusCode::OK
        );
        ensure_partition(&state, &gateway_key, PARTITION).await;
        ensure_partition(&state, &gateway_key, GUEST_PARTITION).await;
    }
    if matches!(scenario, Scenario::RecoveredQueue) {
        let mut gateway = crate::models::downstream_service::test_helpers::dummy_service();
        gateway.id = Uuid::new_v4().to_string();
        gateway.slug = GATEWAY_SLUG.into();
        gateway.base_url = provider.uri();
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(gateway)
            .await
            .unwrap();
        wiremock::Mock::given(wiremock::matchers::path("/events/cmaeg1.ev.sealed-reference/reply-target"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref":"cmaeg1.rt.recovered","expires_at":(Utc::now()+ChronoDuration::hours(1)).timestamp()})))
            .with_priority(1).mount(&provider).await;
        wiremock::Mock::given(wiremock::matchers::path(
            "/reply-targets/cmaeg1.rt.recovered/messages",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .with_priority(1)
        .mount(&provider)
        .await;
    }
    let group_chat = if discord_gateway { "100200" } else { "-100200" };
    if group {
        // Known chat metadata avoids live provider title lookups. Fresh-agent
        // cases have owner-enabled settings but no previous owner model turn.
        let chat = chats::record_chat(
            &state,
            &channel,
            &chats::group_partition(group_chat, None),
            &chats::ChatFacts {
                kind: "group",
                chat_id: group_chat.into(),
                thread_id: None,
                owner: false,
                title: Some("Customer support".into()),
            },
            None,
        )
        .await
        .unwrap();
        if !scenario.owner_first() {
            state
                .db
                .collection::<bson::Document>(THREADS)
                .update_one(doc! {"_id":chat.id}, doc! {"$set":{"members":"everyone"}})
                .await
                .unwrap();
        }
    }
    let callbacks = Router::new()
        .route("/api/v1/proxy/s/{slug}/{*path}", any(model_callback))
        .layer(Extension(
            crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY,
        ))
        .with_state(state.clone());
    let seen: Calls = Default::default();
    let mapping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let source_mapping = mapping.clone();
    let sink = seen.clone();
    let db = state.db.clone();
    let upstream=Router::new().route("/v1/responses",post(move |headers:HeaderMap,Json(body):Json<Value>| {
        let callbacks=callbacks.clone();let sink=sink.clone();let db=db.clone();let mapping=source_mapping.clone();
        async move {
            if matches!(scenario,Scenario::FailedSetup) {
                return (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":{"code":"model_unavailable","message":"private provider detail"}}))).into_response();
            }
            let raw=headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let (owner,key,_)=crate::services::key_service::validate_api_key(&db,raw).await.unwrap(); assert_eq!(owner,OWNER);
            let row=db.collection::<AssistantConversation>(CONVERSATIONS).find_one(doc!{"credential_api_key_id":&key.id}).await.unwrap().unwrap();
            assert_eq!(row.user_id,OWNER);
            assert!(row.model==profile || matches!(scenario,Scenario::ProfileChange) && row.model=="nyxagent/chat");
            if row.guest_model_setup {
                assert!(!row.guest_turn);
                assert_eq!(body["input"],crate::services::assistant_guest_model_setup::INSTRUCTION);
                assert_eq!(body["instructions"],crate::services::assistant_guest_model_setup::INSTRUCTION);
                for field in ["tools","tool_choice","max_output_tokens"] {assert!(body.get(field).is_none(),"NyxAgent rejects unsupported fields");}
                if matches!(scenario,Scenario::Concurrent | Scenario::RecoveredQueue) {tokio::time::sleep(Duration::from_millis(1000)).await;}
                assert!(!key.allow_all_services && key.allowed_service_ids.is_empty() && key.allowed_node_ids.is_empty());
                assert!(db.collection::<AssistantConversation>(CONVERSATIONS).count_documents(doc!{"guest_model_setup":true}).await.unwrap()<=1);
            }
            let turn=row.active_turn.as_ref().unwrap();
            for kind in ["input_admission","model_attempt"] {
                let mut filter=doc!{"conversation_id":&row.id,"turn_id":&turn.turn_id,"api_key_id":&key.id,"kind":kind};
                if kind=="model_attempt" {filter.insert("active",true);}
                assert!(db.collection::<bson::Document>("assistant_service_execution").find_one(filter).await.unwrap().is_some());
            }
            if row.guest_model_setup {
                let refusal=callbacks.clone().oneshot(Request::builder().method("GET").uri("/api/v1/proxy/s/guest-service/status")
                    .header("authorization",headers["authorization"].clone()).body(Body::empty()).unwrap()).await.unwrap();
                assert_eq!(refusal.status(),StatusCode::FORBIDDEN,"setup must never use owner tools");
            }
            let selected_model=if mapping.load(std::sync::atomic::Ordering::SeqCst) {"model-after-policy-change"}else if matches!(scenario,Scenario::ProfileChange) && row.model=="nyxagent/chat" {"gpt-5.6-sol"}else{model};
            let generation=json!({"model":selected_model,"store":false,"stream":false,"input":body["input"],"instructions":body["instructions"],"tools":[{"type":"web_search"}],
                "client_metadata":{"root_turn_id":Uuid::new_v4().to_string(),"session_id":Uuid::new_v4().to_string(),"thread_id":Uuid::new_v4().to_string(),
                    "turn_id":Uuid::new_v4().to_string(),"x-codex-installation-id":"test-installation","x-codex-turn-metadata":"{\"sandbox_mode\":\"read-only\"}","x-codex-window-id":Uuid::new_v4().to_string()}});
            let response=callbacks.clone().oneshot(Request::builder().method("POST").uri(format!("/api/v1/proxy/s/{slug}/responses"))
                .header("authorization",headers["authorization"].clone()).header("content-type","application/json").header("accept","*/*")
                .body(Body::from(generation.to_string())).unwrap()).await.unwrap();
            let status=response.status().as_u16();let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
            if row.guest_turn && status==200 && !matches!(scenario,Scenario::Merchant) {
                let response=callbacks.clone().oneshot(Request::builder().method("GET").uri("/api/v1/proxy/s/guest-service/status")
                    .header("authorization",headers["authorization"].clone()).body(Body::empty()).unwrap()).await.unwrap();
                let status=response.status();let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
                assert_eq!(status,StatusCode::OK,"guest service: {}",String::from_utf8_lossy(&bytes));
            }
            sink.lock().await.push(json!({"status":status,"body":serde_json::from_slice::<Value>(&bytes).unwrap(),"guest":row.guest_turn,"setup":row.guest_model_setup,"conversation":row.id,"key":key.id}));
            let failed=status!=200; let session=format!("conv_{}",Uuid::new_v4().simple());
            let event=json!({"type":if failed {"response.failed"}else{"response.completed"},"sequence_number":0,
                "response":{"id":format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple()),"conversation":{"id":session},"status":if failed{"failed"}else{"completed"},
                "error":if failed{json!({"code":"turn_failed","message":"never expose this secret provider detail"})}else{Value::Null},
                "output":if failed{json!([])}else{json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}])}}});
            axum::response::Sse::new(futures::stream::iter([Ok::<_,Infallible>(Event::default().data(event.to_string()))])).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{addr}")}},
        )
        .await
        .unwrap();
    if matches!(scenario, Scenario::EnabledVisitors) {
        let result = if group {
            let chat = state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"channel_id":&channel.id})
                .await
                .unwrap()
                .unwrap();
            chats::update_chat(
                &state,
                OWNER,
                &chat.id,
                &serde_json::from_value(json!({"members":"everyone"})).unwrap(),
            )
            .await
            .unwrap()
        } else {
            chats::set_private_chats(&state, OWNER, &channel.id, "everyone")
                .await
                .unwrap()
        };
        assert!(matches!(
            if group {
                result["chat"]["guest_readiness"]["status"].as_str()
            } else {
                result["guest_readiness"]["status"].as_str()
            },
            Some("setting_up" | "ready")
        ));
    }
    let send = |index: usize, owner: bool| {
        let state = &state;
        let replies = &replies;
        let channel = &channel;
        let specialist = &specialist;
        let gateway_key = &gateway_key;
        let provider = &provider;
        async move {
            let sender = if matches!(scenario, Scenario::Concurrent) && index == 1 {
                "other-visitor"
            } else if owner {
                "7"
            } else if discord_gateway {
                "9"
            } else {
                guest_person
            };
            let id = Uuid::new_v4().to_string();
            let chat = if matches!(scenario, Scenario::Concurrent) && index == 1 {
                "other-guest-chat"
            } else {
                match (discord_gateway, group, owner) {
                    (_, true, _) => group_chat,
                    (true, false, true) => "1001",
                    (true, false, false) => "1002",
                    (false, false, true) => "owner-chat",
                    (false, false, false) => "guest-chat",
                }
            };
            let text = if matches!(scenario, Scenario::Merchant) {
                "/start"
            } else {
                "@helper_bot hello"
            };
            let delivered = if transport == "gateway" {
                let mut body = if group {
                    group_event(text, sender, "Customer", &id, PARTITION, true)
                } else {
                    event(text, sender, &id)
                };
                let partition = if owner { PARTITION } else { GUEST_PARTITION };
                body["conversation"] = json!(partition);
                body["event_context"]["conversation_id"] = json!(partition);
                body["event_context"]["activity"]["source"]["platform"] =
                    json!(canonical_platform(platform));
                body["event_context"]["activity"]["conversation"]["id"] = json!(chat);
                if discord_gateway {
                    body["event_context"]["channel_id"] = json!(channel.gateway_channel_id);
                    body["event_context"]["activity"]["source"]["route_id"] =
                        json!(channel.route_id);
                }
                if canonical_platform(platform) != "telegram" {
                    body["event_context"]["activity"]
                        .as_object_mut()
                        .unwrap()
                        .remove("telegram_input");
                }
                if matches!(scenario, Scenario::RecoveredQueue) {
                    let mut request = Box::pin(respond(state, gateway_key, &body, &id));
                    let pending = async {
                        loop {
                            if let Some(row)=state.db.collection::<bson::Document>(EVENTS).find_one(doc!{"channel_id":&channel.id,"event_id":&id,"status":"queued","guest_setup_payload":{"$exists":true}}).await.unwrap() {break row;}
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    };
                    let queued = tokio::select! { _=&mut request=>panic!("request finished before simulated crash"), row=tokio::time::timeout(Duration::from_secs(10),pending)=>row.unwrap() };
                    drop(request);
                    let event_id = queued.get_str("_id").unwrap();
                    state.db.collection::<bson::Document>(EVENTS).update_one(doc!{"_id":event_id},doc!{"$set":{"guest_setup_lease":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}}).await.unwrap();
                    let delivered = tokio::time::timeout(Duration::from_secs(30), async {
                        loop {
                            guest_setup_queue::sweep(state).await.unwrap();
                            late_delivery::sweep(state).await.unwrap();
                            let requests = provider.received_requests().await.unwrap();
                            if let Some(sent) = requests.iter().find(|r| {
                                r.url.path() == "/reply-targets/cmaeg1.rt.recovered/messages"
                            }) {
                                break serde_json::from_slice::<Value>(&sent.body).unwrap()["text"]
                                    .as_str()
                                    .unwrap()
                                    .to_owned();
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    })
                    .await
                    .unwrap();
                    guest_setup_queue::sweep(state).await.unwrap();
                    late_delivery::sweep(state).await.unwrap();
                    let saved = state
                        .db
                        .collection::<bson::Document>(EVENTS)
                        .find_one(doc! {"_id":event_id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(saved.get("guest_setup_payload").is_none());
                    assert!(saved.get_str("turn_id").is_ok());
                    assert_eq!(
                        provider
                            .received_requests()
                            .await
                            .unwrap()
                            .iter()
                            .filter(
                                |r| r.url.path() == "/reply-targets/cmaeg1.rt.recovered/messages"
                            )
                            .count(),
                        1,
                        "recovery delivers exactly once to the original source"
                    );
                    delivered
                } else {
                    let response = respond(state, gateway_key, &body, &id).await;
                    assert_eq!(
                        response.status(),
                        StatusCode::OK,
                        "{platform}/{transport}/{group}/{scenario:?}"
                    );
                    body_text(response).await
                }
            } else {
                state.db.collection::<bson::Document>("channel_messages").insert_one(doc!{
                "_id":&id,"channel_bot_id":&channel.channel_bot_id,"conversation_id":channel.route_id.as_deref().unwrap(),"platform_conversation_id":chat,
                    "user_id":bot_owner(channel),"direction":"inbound","platform":platform,"platform_message_id":&id,"sender_platform_id":sender,
                "agent_api_key_id":&channel.route_api_key_id,"content_type":"text","original_message":true,"created_at":bson::DateTime::now()
            }).await.unwrap();
                let raw = match canonical_platform(platform) {
                    "telegram" => {
                        json!({"message":{"message_id":1,"from":{"is_bot":false},"text":text,"entities":[{"type":"mention","offset":0,"length":11}]}})
                    }
                    "lark" | "feishu" => {
                        json!({"event":{"sender":{"sender_type":"user"},"message":{"mentions":[{"key":"@helper_bot","id":{"open_id":"ou_bot"},"name":"Helper bot"}]}}})
                    }
                    "slack" => json!({"event":{"type":"app_mention"}}),
                    "discord" => json!({"type":2}),
                    _ => json!({}),
                };
                let body = json!({"message_id":id,"correlation_id":id,"platform":platform,"agent":{"api_key_id":channel.route_api_key_id,"name":"route"},
                "conversation":{"id":channel.route_id,"platform_id":chat,"type":if group{"group"}else{"private"}},"sender":{"platform_id":sender},
                "content":{"type":"text","text":text},"timestamp":"2026-09-30T00:00:00Z","raw_platform_data":raw});
                let bytes = serde_json::to_vec(&body).unwrap();
                let token = crate::crypto::jwt::generate_relay_callback_token(
                    &state.jwt_keys,
                    &state.config,
                    &id,
                    &channel.route_api_key_id,
                    &id,
                    platform,
                    &sha256_hex(&bytes),
                )
                .unwrap();
                let mut headers = HeaderMap::new();
                headers.insert("x-nyxid-callback-token", token.parse().unwrap());
                let (_guard, done) = watch_relay_callback(&channel.id, &id);
                assert_eq!(
                    relay_callback(
                        State(state.clone()),
                        Path(channel.id.clone()),
                        headers,
                        Bytes::from(bytes)
                    )
                    .await
                    .status(),
                    StatusCode::ACCEPTED
                );
                tokio::time::timeout(Duration::from_secs(30), done)
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "relay callback timed out: {platform}/{transport}/{group}/{scenario:?}"
                        )
                    })
                    .unwrap();
                let sent = replies.lock().await;
                let reply = sent
                    .iter()
                    .rev()
                    .find(|r| r["chat"] == chat)
                    .expect("native reply dispatched");
                assert_eq!(reply["chat"], chat);
                reply["text"].as_str().unwrap().to_owned()
            };
            if matches!(scenario, Scenario::FailedSetup) {
                assert!(
                delivered.contains("Visitor setup could not finish. The owner has been notified. Please try again later."),
                "{platform}/{transport}: {delivered}"
            );
                assert!(!delivered.contains("secret provider detail"));
            } else {
                assert!(
                    delivered.contains("Hello"),
                    "{platform}/{transport}/{group}/{scenario:?}: {delivered}"
                );
            }
            if owner {
                let agent = state
                    .db
                    .collection::<AssistantAgent>("assistant_agents")
                    .find_one(doc! {"_id":&specialist.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(agent.owner_model_route.as_ref().unwrap().profile, profile);
            }
        }
    };
    if matches!(
        scenario,
        Scenario::RotateCredential | Scenario::MappingChange | Scenario::ProfileChange
    ) {
        send(0, false).await;
        if matches!(scenario, Scenario::RotateCredential) {
            let connection = state
                .db
                .collection::<bson::Document>("user_services")
                .find_one(doc! {"_id":&model_connection})
                .await
                .unwrap()
                .unwrap();
            let key = connection.get_str("api_key_id").unwrap();
            state
                .db
                .collection::<bson::Document>("user_api_keys")
                .update_one(doc! {"_id":key}, doc! {"$inc":{"credential_epoch":1_i64}})
                .await
                .unwrap();
        } else if matches!(scenario, Scenario::ProfileChange) {
            state
                .db
                .collection::<bson::Document>("assistant_agents")
                .update_one(
                    doc! {"_id":&specialist.id},
                    doc! {"$set":{"model":"nyxagent/chat"}},
                )
                .await
                .unwrap();
            state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .update_many(
                    doc! {"agent_id":&specialist.id,"guest_model_setup":{"$ne":true}},
                    doc! {"$set":{"model":"nyxagent/chat"}},
                )
                .await
                .unwrap();
        } else {
            mapping.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        send(1, false).await;
    } else if matches!(scenario, Scenario::Concurrent) {
        tokio::join!(send(0, false), send(1, false));
    } else if matches!(scenario, Scenario::FailedSetup) {
        send(0, false).await;
        send(1, false).await;
    } else if scenario.owner_first() {
        send(0, true).await;
        send(1, false).await;
    } else {
        send(0, false).await;
    }
    let readiness = crate::services::assistant_service_execution::guest_readiness(
        &state.db,
        &state.encryption_keys,
        OWNER,
        Some(&specialist.id),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        readiness.status,
        if matches!(scenario, Scenario::FailedSetup) {
            "not_ready"
        } else {
            "ready"
        },
        "{platform}/{transport}/{scenario:?}: {}",
        serde_json::to_string(&readiness).unwrap()
    );
    let settings = list_tool(&state, OWNER).await.unwrap();
    let entry = settings["channel_agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == channel.id)
        .unwrap();
    assert_eq!(entry["guest_readiness"]["status"], readiness.status);
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    crate::services::assistant_service_execution::readiness::READS
        .scope(reads.clone(), async {
            let chat_settings = chats::list_chats(&state, OWNER, Some(&channel.id))
                .await
                .unwrap();
            assert!(!chat_settings.is_empty());
            for chat in chat_settings {
                assert!(serde_json::to_value(chat).unwrap()["guest_readiness"].is_null());
            }
            let Json(channels) = list_channels(
                State(state.clone()),
                crate::test_utils::test_auth_user(OWNER),
            )
            .await
            .unwrap();
            assert!(
                channels["channel_agents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["guest_readiness"].is_null())
            );
            if platform == "telegram-new" && !group {
                let Json(dto) = crate::handlers::assistant_team::get_agent(
                    State(state.clone()),
                    crate::test_utils::test_auth_user(OWNER),
                    Path(specialist.id.clone()),
                )
                .await
                .unwrap();
                assert!(dto["agent"]["guest_readiness"].is_null());
            }
        })
        .await;
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "list paths do no readiness reads"
    );
    let Json(dto) = crate::handlers::assistant_team::guest_readiness(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(specialist.id.clone()),
        axum::extract::Query(Default::default()),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(dto).unwrap()["status"],
        readiness.status
    );
    if matches!(scenario, Scenario::FailedSetup) {
        let notices = state
            .db
            .collection::<bson::Document>(crate::services::assistant_service_execution::COLLECTION)
            .count_documents(doc! {"kind":"guest_model_setup_notice","agent_id":&specialist.id})
            .await
            .unwrap();
        assert_eq!(
            notices, 1,
            "one metadata-only owner notification per setup reason"
        );
        assert!(readiness.fix.as_deref().unwrap().contains("automatically"));
        if transport == "direct" {
            let mut private = state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"channel_id":&channel.id})
                .await
                .unwrap()
                .unwrap();
            private.id = Uuid::new_v4().to_string();
            private.partition = format!("owner-notice-{}", private.id);
            private.relay_partition = None;
            private.owner_chat = true;
            private.kind = Some("private".into());
            private.platform_chat_id = Some("owner-notice-chat".into());
            private.platform_thread_id = None;
            private.telegram_topic = None;
            private.allow_posts = true;
            private.conversation_id = None;
            private.follow = Default::default();
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .insert_one(&private)
                .await
                .unwrap();
            let mut notice_bot_guards = Vec::new();
            // Same owner, different linked agents: none means NyxBot, not this specialist.
            if platform == "lark" && !group {
                let nyxbot =
                    crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
                        .await
                        .unwrap();
                for binding in [None, Some(nyxbot.id), Some(Uuid::new_v4().to_string())] {
                    let mut other_channel = channel.clone();
                    other_channel.id = Uuid::new_v4().to_string();
                    other_channel.agent_id = binding;
                    other_channel.private_chats = Some("owner".into());
                    notice_bot_guards.push(
                        distinct_notice_bot(&state, &mut other_channel, platform, &replies).await,
                    );
                    state
                        .db
                        .collection::<NyxbotChannel>(CHANNELS)
                        .insert_one(&other_channel)
                        .await
                        .unwrap();
                    let mut other_chat = private.clone();
                    other_chat.id = Uuid::new_v4().to_string();
                    other_chat.channel_id = other_channel.id;
                    other_chat.partition = format!("other-{}", other_chat.id);
                    other_chat.platform_chat_id = Some(format!("wrong-agent-{}", other_chat.id));
                    let other_chat_id = other_chat.id.clone();
                    state
                        .db
                        .collection::<NyxbotThread>(THREADS)
                        .insert_one(other_chat)
                        .await
                        .unwrap();
                    assert!(
                        chats::prepare_post(&state, OWNER, &other_chat_id, None)
                            .await
                            .is_ok(),
                        "unrelated-agent owner destinations must otherwise be deliverable"
                    );
                }
                // Two open channels for one specialist compute readiness once;
                // closed channels for other agents compute none.
                let mut same = channel.clone();
                same.id = Uuid::new_v4().to_string();
                notice_bot_guards
                    .push(distinct_notice_bot(&state, &mut same, platform, &replies).await);
                state
                    .db
                    .collection::<NyxbotChannel>(CHANNELS)
                    .insert_one(&same)
                    .await
                    .unwrap();
                reads.store(0, std::sync::atomic::Ordering::SeqCst);
                crate::services::assistant_service_execution::readiness::READS
                    .scope(reads.clone(), async {
                        let listed = list_tool(&state, OWNER).await.unwrap();
                        assert_eq!(listed["channel_agents"].as_array().unwrap().len(), 5);
                    })
                    .await;
                assert_eq!(
                    reads.load(std::sync::atomic::Ordering::SeqCst),
                    1,
                    "memoized per open-channel agent"
                );
                // Group audience also requests readiness when private chats are owner-only.
                state
                    .db
                    .collection::<bson::Document>(CHANNELS)
                    .update_many(
                        doc! {"user_id":OWNER},
                        doc! {"$set":{"private_chats":"owner"}},
                    )
                    .await
                    .unwrap();
                state
                    .db
                    .collection::<bson::Document>(THREADS)
                    .update_one(
                        doc! {"_id":&private.id},
                        doc! {"$set":{"kind":"group","members":"everyone"}},
                    )
                    .await
                    .unwrap();
                reads.store(0, std::sync::atomic::Ordering::SeqCst);
                crate::services::assistant_service_execution::readiness::READS
                    .scope(reads.clone(), async {
                        list_tool(&state, OWNER).await.unwrap();
                    })
                    .await;
                assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
                state
                    .db
                    .collection::<bson::Document>(THREADS)
                    .update_one(
                        doc! {"_id":&private.id},
                        doc! {"$set":{"kind":"private","members":"owner"}},
                    )
                    .await
                    .unwrap();
                reads.store(0, std::sync::atomic::Ordering::SeqCst);
                crate::services::assistant_service_execution::readiness::READS
                    .scope(reads.clone(), async {
                        list_tool(&state, OWNER).await.unwrap();
                    })
                    .await;
                assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0);
            }
            let before = replies.lock().await.len();
            guest_model_notices::sweep(&state).await.unwrap();
            guest_model_notices::sweep(&state).await.unwrap();
            let sent = replies.lock().await;
            assert_eq!(sent.len(), before + 1, "one owner setup notice");
            let note = sent.last().unwrap();
            assert_eq!(note["chat"], "owner-notice-chat");
            assert!(note["text"].as_str().unwrap().contains("automatically"));
            assert!(note["text"].as_str().unwrap().contains("/assistant?agent="));
            assert!(!note["text"].as_str().unwrap().contains("@helper_bot hello"));
        }
    }
    let seen = seen.lock().await;
    assert_eq!(
        seen.len(),
        if matches!(scenario, Scenario::FailedSetup) {
            0
        } else if matches!(scenario, Scenario::MappingChange) {
            5
        } else if matches!(
            scenario,
            Scenario::RotateCredential | Scenario::ProfileChange
        ) {
            4
        } else if matches!(scenario, Scenario::Concurrent) {
            3
        } else {
            2
        },
        "{seen:?}"
    );
    if !scenario.owner_first() && !matches!(scenario, Scenario::FailedSetup) {
        assert_eq!(
            seen.iter().filter(|r| r["setup"] == true).count(),
            if matches!(
                scenario,
                Scenario::RotateCredential | Scenario::MappingChange | Scenario::ProfileChange
            ) {
                2
            } else {
                1
            },
            "concurrent visitors share setup"
        );
        let hidden = seen.iter().find(|r| r["setup"] == true).unwrap()["conversation"]
            .as_str()
            .unwrap();
        for _ in 0..50 {
            if state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .find_one(doc! {"_id":hidden})
                .await
                .unwrap()
                .is_none()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .find_one(doc! {"_id":hidden})
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>("assistant_messages")
                .count_documents(doc! {"conversation_id":hidden})
                .await
                .unwrap(),
            0
        );
    }
    let requests = provider.received_requests().await.unwrap();
    let restricted: Vec<Value> = requests
        .iter()
        .filter_map(|request| serde_json::from_slice::<Value>(&request.body).ok())
        .filter(|body| body["max_output_tokens"] == 16)
        .collect();
    assert_eq!(
        restricted.len(),
        seen.iter().filter(|r| r["setup"] == true).count()
    );
    for body in restricted {
        assert_eq!(body["tools"], json!([]));
        assert_eq!(body["tool_choice"], "none");
    }
    let failure = json!({"guest":true,"status":403});
    let last = seen.last().unwrap_or(&failure);
    assert_eq!(last["guest"], true);
    assert_eq!(
        last["status"],
        if matches!(scenario, Scenario::FailedSetup) {
            403
        } else {
            200
        },
        "{seen:?}"
    );
    if scenario.owner_first() {
        if group {
            assert_eq!(seen[0]["conversation"], last["conversation"]);
        } else {
            assert_ne!(
                seen[0]["conversation"], last["conversation"],
                "private guests need their own thread"
            );
        }
    }
    if !scenario.owner_first() {
        let agent = state
            .db
            .collection::<AssistantAgent>("assistant_agents")
            .find_one(doc! {"_id":&specialist.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            agent.owner_model_route.is_some() != matches!(scenario, Scenario::FailedSetup),
            "only admitted system owner setup establishes the cold route"
        );
    }
    let expected = if matches!(scenario, Scenario::FailedSetup) {
        0
    } else if matches!(scenario, Scenario::Concurrent) {
        5
    } else if matches!(
        scenario,
        Scenario::RotateCredential | Scenario::MappingChange | Scenario::ProfileChange
    ) {
        6
    } else if matches!(scenario, Scenario::Merchant) {
        2
    } else {
        3
    };
    let usage: Vec<UsageMeterRow> = crate::billing_integration_tests::wait_for_charged_usage(
        &state.db,
        doc! {"service_id": {"$in": [&model_id, &tool_id]}},
        expected,
    )
    .await;
    assert_eq!(usage.len(), expected, "{platform}/{transport}/{scenario:?}");
    for row in &usage {
        assert_eq!(row.status, UsageStatus::Finalized);
        assert!(row.forwarded && row.released);
        assert_eq!(row.billing_owner_id, OWNER);
        assert_eq!(row.actor_user_id, OWNER);
        assert_eq!(
            row.credential_class,
            if scenario.platform() {
                CredentialClass::NyxidManagedMaster
            } else {
                CredentialClass::UserOwned
            }
        );
        assert_eq!(
            row.user_service_id.as_deref(),
            if row.service_id.as_deref() == Some(model_id.as_str())
                && matches!(scenario, Scenario::NoPersonalRow)
            {
                None
            } else {
                Some(if row.service_id.as_deref() == Some(model_id.as_str()) {
                    model_connection.as_str()
                } else {
                    tool_connection.as_str()
                })
            }
        );
        assert!(
            row.funding.as_ref().is_some_and(|funding| funding.settled),
            "settlement must complete"
        );
        let entry = state
            .db
            .collection::<bson::Document>("billing_ledger")
            .find_one(doc! {"reference_id":&row.id, "owner_id":OWNER})
            .await
            .unwrap();
        assert!(entry.is_some(), "owner charge must be journaled");
        assert!(
            row.wallet_id.is_some(),
            "must exercise charged settlement, not only metering"
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("usage_meter")
            .count_documents(
                doc! {"$or":[{"billing_owner_id":{"$ne":OWNER}},{"actor_user_id":{"$ne":OWNER}}]}
            )
            .await
            .unwrap(),
        0,
        "no charge or attribution to guest or unrelated person"
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("billing_wallet")
            .count_documents(doc! {"owner_id":{"$ne":OWNER}})
            .await
            .unwrap(),
        0
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_setup_2092_cold_configurations_and_immediate_enable() {
    for scenario in [
        Scenario::NoPersonalRow,
        Scenario::RetainedPlatformKey,
        Scenario::DeepSeek,
        Scenario::EnabledVisitors,
        Scenario::FailedSetup,
    ] {
        Box::pin(channel_model_turns("telegram", "direct", false, scenario)).await;
    }
}

#[tokio::test]
async fn guest_setup_2092_concurrent_first_guests_share_one_owner_turn() {
    Box::pin(channel_model_turns(
        "telegram",
        "direct",
        false,
        Scenario::Concurrent,
    ))
    .await;
}

#[tokio::test]
async fn guest_setup_2092_connection_and_nyxagent_policy_changes_reinitialize() {
    for scenario in [
        Scenario::RotateCredential,
        Scenario::MappingChange,
        Scenario::ProfileChange,
    ] {
        Box::pin(channel_model_turns("telegram", "direct", false, scenario)).await;
    }
}

#[tokio::test]
async fn guest_setup_2092_first_start_on_ask_every_time_merchant_without_tool_grants() {
    Box::pin(channel_model_turns(
        "telegram",
        "direct",
        false,
        Scenario::Merchant,
    ))
    .await;
}

#[tokio::test]
async fn guest_setup_2092_enabling_group_members_prepares_without_owner_message() {
    Box::pin(channel_model_turns(
        "telegram",
        "direct",
        true,
        Scenario::EnabledVisitors,
    ))
    .await;
}

#[tokio::test]
async fn guest_setup_2092_pending_guest_recovers_after_request_loss_once() {
    Box::pin(channel_model_turns(
        "telegram-new",
        "gateway",
        false,
        Scenario::RecoveredQueue,
    ))
    .await;
}
