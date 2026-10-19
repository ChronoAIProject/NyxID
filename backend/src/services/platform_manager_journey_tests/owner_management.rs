//! Signed private ingress and browser proof reach the real authenticated MCP
//! handlers. NyxAgent's tool choices and Discord delivery use local fixtures.
use super::*;
use crate::{
    models::{
        assistant_agent::COLLECTION_NAME as AGENTS,
        assistant_conversation::TurnOrigin,
        assistant_settings::{AccessMode, AssistantSettings},
        channel_conversation::ChannelConversation,
        nyxbot_channel::NyxbotChannel,
    },
    services::{
        assistant_acknowledgement_service as acks, assistant_authority_tests::connected,
        assistant_team_service as team, billing, key_service,
    },
};
use tokio::sync::Mutex;

async fn mcp(state: &AppState, key: &str, tool: &str, arguments: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("x-api-key", key)
        .body(Body::from(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":tool,"arguments":arguments}})
            .to_string(),
        ))
        .unwrap();
    let response = crate::handlers::mcp_transport::mcp_post(
        axum::extract::State(state.clone()),
        axum::extract::Extension(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::Mcp,
        )),
        request,
    )
    .await;
    let status = response.status();
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn allowed(state: &AppState, key: &str, tool: &str, args: Value) -> AppResult<Value> {
    let (status, response) = mcp(state, key, tool, args).await;
    if status != StatusCode::OK || response["result"]["isError"] != false {
        return Err(AppError::Internal(format!(
            "Owner journey tool {tool} refused: {status} {response}"
        )));
    }
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
        .map_err(|_| AppError::Internal(format!("Invalid owner journey result: {tool}")))
}

enum ExpectedRefusal {
    Domain(&'static str),
    NoLiveTurn,
}

async fn refused(state: &AppState, key: &str, tool: &str, args: Value, expected: ExpectedRefusal) {
    let (status, response) = mcp(state, key, tool, args).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{tool} refusal transport: {response}"
    );
    match expected {
        ExpectedRefusal::Domain(code) => {
            assert_eq!(
                response["result"]["isError"], true,
                "{tool} domain refusal: {response}"
            );
            let result: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(
                result["error"], code,
                "{tool} foreign-owner refusal: {result}"
            );
        }
        ExpectedRefusal::NoLiveTurn => {
            assert_eq!(
                response["error"]["code"], -32003,
                "{tool} expired turn: {response}"
            );
            assert_eq!(
                response["error"]["message"],
                AppError::AssistantTurnRequired.to_string(),
                "{tool} expired turn"
            );
        }
    }
}

async fn assert_public_faq(state: &AppState, profile: &str) {
    let receipt = state
        .db
        .collection::<PlatformManagerIngress>(INGRESS)
        .find_one(doc! {"profile_id":profile,"audience.kind":"public"})
        .sort(doc! {"admitted_at":-1})
        .await
        .unwrap()
        .expect("public FAQ receipt");
    assert!(matches!(receipt.audience, IngressAudience::Public { .. }));
    assert!(receipt.execution.is_none(), "FAQ cannot start a model turn");
    let delivery = state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {"ingress_id":&receipt.id,"reference.kind":"faq"})
        .await
        .unwrap()
        .expect("durable FAQ delivery");
    assert!(matches!(
        delivery.reference,
        crate::models::platform_manager_delivery::DeliveryReference::Faq { .. }
    ));
    assert_eq!(delivery.state, DeliveryState::Sent);
}

struct ManagementEgress {
    state: AppState,
    owner: String,
    service: String,
    foreign_service: String,
    owned_bot: String,
    foreign_bot: String,
    foreign_agent: String,
    foreign_owner: String,
    inner: Arc<SavedReplyEgress>,
    retained_key: Mutex<Option<Zeroizing<String>>>,
    created: Mutex<Option<(String, String)>>,
    assignment: Mutex<Option<(String, String)>>,
    failure: Mutex<Option<String>>,
}

impl ManagementEgress {
    async fn manage(&self, context: &NyxAgentExecutionContext, key: &str) -> AppResult<()> {
        let NyxAgentExecutionContext::Owner {
            user_id,
            turn,
            platform_origin: Some(origin),
        } = context
        else {
            return Err(AppError::Internal(
                "Expected private channel owner turn".into(),
            ));
        };
        assert_eq!(user_id, &self.owner);
        assert_eq!(origin.authority.user_id, self.owner);
        let (owner, credential, child) = key_service::validate_api_key(&self.state.db, key).await?;
        assert_eq!(owner, self.owner);
        assert!(child.is_none());
        let chat = acks::for_key(&self.state.db, &owner, Some(&credential.id))
            .await?
            .unwrap();
        assert!(chat.turn_live && chat.is_orchestrator() && !chat.guest);
        assert_eq!(chat.conversation_id, turn.conversation_id);
        assert!(chat.platform_origin.is_some());
        *self.retained_key.lock().await = Some(Zeroizing::new(key.to_owned()));

        let created = allowed(
            &self.state,
            key,
            "nyxid__spawn_subagent",
            json!({
                "name":"channel-helper","description":"Help with the owner's notes",
                "services":[],"account_read":false,
            }),
        )
        .await?;
        let agent = created["subagent"]["id"].as_str().unwrap();
        allowed(
            &self.state,
            key,
            "nyxid__update_subagent",
            json!({
                "subagent":agent,"display_name":"Channel helper",
                "description":"Summarize the owner's notes","persona":"Answer clearly and briefly",
            }),
        )
        .await?;
        refused(
            &self.state,
            key,
            "nyxid__update_subagent",
            json!({
                "subagent":self.foreign_agent,"description":"Must not change another owner",
            }),
            ExpectedRefusal::Domain("not_found"),
        )
        .await;
        refused(
            &self.state,
            key,
            "nyxid__grant_subagent",
            json!({
                "subagent":agent,"services":[self.foreign_service],
            }),
            ExpectedRefusal::Domain("not_granted"),
        )
        .await;
        allowed(
            &self.state,
            key,
            "nyxid__grant_subagent",
            json!({
                "subagent":agent,"services":[self.service],"account_read":true,
            }),
        )
        .await?;
        let handoff = allowed(
            &self.state,
            key,
            "nyxid__channel_bot_setup_link",
            json!({"platform":"discord","label":"Specialist setup","agent":agent}),
        )
        .await?;
        assert!(
            handoff["steps"]
                .as_str()
                .unwrap()
                .contains("never in this chat")
        );
        // The one-time URL exists only in this callback; persist/assert metadata.
        let setup = self
            .state
            .db
            .collection::<crate::models::channel_connect_link::ChannelConnectLink>(
                crate::models::channel_connect_link::COLLECTION_NAME,
            )
            .find_one(doc! {"user_id":&self.owner,"label":"Specialist setup"})
            .await?
            .unwrap();
        assert_eq!(setup.created_by_user_id, self.owner);
        assert_eq!(setup.platform, "discord");
        assert_eq!(
            setup.status,
            crate::models::channel_connect_link::LinkStatus::Pending
        );
        let watch = self
            .state
            .db
            .collection::<crate::models::nyxbot_channel::NyxbotWatch>(
                crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
            )
            .find_one(doc! {"connect_link_id":&setup.id})
            .await?
            .unwrap();
        assert_eq!(watch.user_id, self.owner);
        assert_eq!(watch.conversation_id, turn.conversation_id);
        assert_eq!(watch.agent_id.as_deref(), Some(agent));
        assert_eq!(watch.invocation_id, chat.invocation_id);
        assert_eq!(watch.report_to, chat.report_to);
        assert_eq!(
            watch.report_provenance,
            crate::models::assistant_conversation::REPORT_PROVENANCE_V1
        );
        assert_eq!(watch.status, "pending");

        let assigned = allowed(
            &self.state,
            key,
            "nyxid__message_subagent",
            json!({"subagent":agent,"text":"Summarize the notes for this channel owner"}),
        )
        .await?;
        assert_eq!(assigned["status"], "started");
        let assigned_turn = assigned["turn_id"].as_str().unwrap().to_owned();
        let specialist = team::live_specialist(&self.state.db, &self.owner, agent).await?;
        let home_id = specialist.home_conversation_id.as_ref().unwrap();
        // Wait while the originating owner turn is still live. The real team
        // settlement hook can queue its report, but cannot start a second owner turn.
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let home = engine::get(&self.state.db, &self.owner, home_id)
                    .await
                    .unwrap();
                let parent = engine::get(&self.state.db, &self.owner, &turn.conversation_id)
                    .await
                    .unwrap();
                if home.active_turn.is_none()
                    && parent.pending_events.iter().any(|event| {
                        event.kind == "subagent_settled" && event.agent_id.as_deref() == Some(agent)
                    })
                {
                    assert_eq!(
                        home.report_to.as_deref(),
                        Some(turn.conversation_id.as_str())
                    );
                    assert_eq!(parent.active_turn.as_ref().unwrap().turn_id, turn.turn_id);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("local specialist settlement reports to originating private thread");
        *self.assignment.lock().await = Some((turn.conversation_id.clone(), assigned_turn));
        refused(
            &self.state,
            key,
            "nyxid__connect_channel_bot",
            json!({
                "bot":self.foreign_bot,"agent":agent,
            }),
            ExpectedRefusal::Domain("channel_bot_not_found"),
        )
        .await;
        let attached = allowed(
            &self.state,
            key,
            "nyxid__connect_channel_bot",
            json!({
                "bot":self.owned_bot,"agent":"nyxbot",
            }),
        )
        .await?;
        let channel = attached["channel_agent"]["id"].as_str().unwrap();
        allowed(
            &self.state,
            key,
            "nyxid__link_channel_bot",
            json!({
                "channel_agent_id":channel,"agent":agent,
            }),
        )
        .await?;
        allowed(
            &self.state,
            key,
            "nyxid__update_channel_bot",
            json!({
                "bot_id":self.owned_bot,"label":"Managed from my private NyxBot",
            }),
        )
        .await?;
        refused(
            &self.state,
            key,
            "nyxid__update_channel_bot",
            json!({
                "bot_id":self.foreign_bot,"label":"Must not change another owner",
            }),
            ExpectedRefusal::Domain("channel_bot_not_found"),
        )
        .await;
        *self.created.lock().await = Some((agent.to_owned(), channel.to_owned()));
        Ok(())
    }
}

#[async_trait]
impl NyxAgentEgress for ManagementEgress {
    async fn request(
        &self,
        context: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        if let Err(error) = self.manage(context, &credential.raw_key).await {
            *self.failure.lock().await = Some(error.to_string());
            return Err(error);
        }
        self.inner.request(context, credential, request).await
    }
}

async fn management_fixture(
    state: &AppState,
    owner: &str,
    base_bot: &ChannelBot,
    provider: &MockServer,
    inner: Arc<SavedReplyEgress>,
) -> Arc<ManagementEgress> {
    state
        .db
        .collection::<AssistantSettings>(crate::models::assistant_settings::COLLECTION_NAME)
        .insert_one(AssistantSettings {
            access_mode: AccessMode::Autopilot,
            ..AssistantSettings::defaults(owner)
        })
        .await
        .unwrap();
    let provider_base = provider.uri();
    // Real specialist start/stream/key/proxy code talks only to this loopback
    // NyxAgent stand-in. This proves orchestration, not real model/provider work.
    let session = uuid::Uuid::new_v4().simple().to_string();
    let frame = json!({"type":"response.completed","sequence_number":1,"response":{
        "id":format!("resp_{session}_{session}"),"conversation":{"id":format!("conv_{session}")},
        "status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Specialist completed the assigned notes task"}]}]}});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!("data: {frame}\n\n").into_bytes(),
            "text/event-stream",
        ))
        .expect(1)
        .mount(provider)
        .await;
    let mut outer = crate::test_utils::test_auto_connected_catalog_service();
    outer.slug = engine::SERVICE_SLUG.into();
    outer.base_url = provider_base.clone();
    outer.forward_access_token = true;
    outer.streaming_supported = true;
    assert!(engine::row_contract(Some(&outer), None).valid());
    state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(outer)
        .await
        .unwrap();
    let service = connected(&state.db, owner, "journey-notes", &provider_base).await;
    let foreign_service = connected(
        &state.db,
        &base_bot.user_id,
        "foreign-notes",
        &provider_base,
    )
    .await;
    let foreign_agent = team::create_specialist(
        &state.db,
        &state.encryption_keys,
        &base_bot.user_id,
        team::CreateRequest {
            name: "foreign-helper".into(),
            description: "Foreign private specialist".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            machines: None,
            logins: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap()
    .0;
    let mut ordinary = native::bot();
    ordinary.id = uuid::Uuid::new_v4().to_string();
    ordinary.user_id = owner.to_owned();
    ordinary.platform = "discord".into();
    ordinary.credential_source = "user".into();
    ordinary.bot_token_encrypted = state
        .encryption_keys
        .encrypt(b"synthetic-owned-discord-token")
        .await
        .unwrap();
    ordinary.app_id = Some("901".into());
    ordinary.public_key = Some(hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&[38; 32])
            .verifying_key()
            .as_bytes(),
    ));
    ordinary.platform_bot_id = "901".into();
    ordinary.label = "My ordinary Discord bot".into();
    let mut foreign = ordinary.clone();
    foreign.id = uuid::Uuid::new_v4().to_string();
    foreign.user_id = base_bot.user_id.clone();
    foreign.platform_bot_id = "902".into();
    foreign.app_id = Some("902".into());
    foreign.bot_token_encrypted = state
        .encryption_keys
        .encrypt(b"synthetic-foreign-discord-token")
        .await
        .unwrap();
    foreign.label = "Foreign ordinary Discord bot".into();
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_many([&ordinary, &foreign])
        .await
        .unwrap();
    Arc::new(ManagementEgress {
        state: state.clone(),
        owner: owner.to_owned(),
        service: service.clone(),
        foreign_service,
        owned_bot: ordinary.id.clone(),
        foreign_bot: foreign.id.clone(),
        foreign_agent: foreign_agent.id.clone(),
        foreign_owner: base_bot.user_id.clone(),
        inner,
        retained_key: Mutex::new(None),
        created: Mutex::new(None),
        assignment: Mutex::new(None),
        failure: Mutex::new(None),
    })
}

async fn assert_managed(inspected: &ManagementEgress, provider: &MockServer) {
    let state = &inspected.state;
    let (agent_id, channel_id) = inspected.created.lock().await.clone().unwrap();
    let agent = team::live_specialist(&state.db, &inspected.owner, &agent_id)
        .await
        .unwrap();
    assert_eq!(agent.user_id, inspected.owner);
    assert_eq!(agent.display_name.as_deref(), Some("Channel helper"));
    assert_eq!(agent.description, "Summarize the owner's notes");
    assert_eq!(agent.persona.as_deref(), Some("Answer clearly and briefly"));
    assert_eq!(agent.grants.service_ids, vec![inspected.service.clone()]);
    assert!(agent.grants.account_read);
    let home = engine::get(
        &state.db,
        &inspected.owner,
        agent.home_conversation_id.as_ref().unwrap(),
    )
    .await
    .unwrap();
    let specialist_key =
        key_service::get_api_key(&state.db, &inspected.owner, &home.credential_api_key_id)
            .await
            .unwrap();
    let (source, assigned_turn) = inspected.assignment.lock().await.clone().unwrap();
    assert_eq!(home.report_to.as_deref(), Some(source.as_str()));
    let messages = engine::messages(&state.db, &inspected.owner, &home.id, 100, None)
        .await
        .unwrap();
    assert!(messages.iter().any(|m| m.role == "orchestrator"
        && m.origin == Some(TurnOrigin::Orchestrator)
        && m.turn_id == assigned_turn
        && m.text == "Summarize the notes for this channel owner"));
    let reply = messages
        .iter()
        .find(|m| m.role == "assistant" && m.turn_id == assigned_turn)
        .expect("assigned specialist reply persisted");
    assert_eq!(
        reply.status,
        "completed",
        "specialist error_code={:?}, stage={:?}",
        reply.error_code,
        reply
            .error_detail
            .as_ref()
            .map(|detail| detail.stage.as_str())
    );
    assert!(reply.error_code.is_none());
    assert_eq!(reply.text, "Specialist completed the assigned notes task");
    let calls: Vec<_> = provider
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.url.path() == "/v1/responses")
        .collect();
    assert_eq!(calls.len(), 1, "one actual local specialist request");
    let authorization = calls[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    let raw = Zeroizing::new(authorization.strip_prefix("Bearer ").unwrap().to_owned());
    let (upstream_owner, upstream_key, _) = key_service::validate_api_key(&state.db, &raw)
        .await
        .unwrap();
    assert_eq!(upstream_owner, inspected.owner);
    assert_eq!(upstream_key.id, specialist_key.id);
    let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
    assert!(
        body.to_string()
            .contains("Summarize the notes for this channel owner")
    );
    assert!(!specialist_key.allow_all_services);
    assert_eq!(
        specialist_key.allowed_service_ids,
        vec![inspected.service.clone()]
    );
    let channel = state
        .db
        .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find_one(doc! {"_id":&channel_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(channel.user_id, inspected.owner);
    assert_eq!(channel.channel_bot_id, inspected.owned_bot);
    assert_eq!(channel.agent_id.as_deref(), Some(agent_id.as_str()));
    assert_eq!(channel.status, "active");
    assert_eq!(channel.transport, "direct");
    assert!(
        channel.owner_sender_ids.is_empty(),
        "a separate owned bot still requires its own owner verification"
    );
    let route = state
        .db
        .collection::<ChannelConversation>(crate::models::channel_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id":channel.route_id.as_ref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(route.user_id, inspected.owner);
    assert_eq!(
        route.channel_bot_id.as_deref(),
        Some(inspected.owned_bot.as_str())
    );
    let bots = state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME);
    assert_eq!(
        bots.find_one(doc! {"_id":&inspected.owned_bot})
            .await
            .unwrap()
            .unwrap()
            .label,
        "Managed from my private NyxBot"
    );
    assert_eq!(
        bots.find_one(doc! {"_id":&inspected.foreign_bot})
            .await
            .unwrap()
            .unwrap()
            .label,
        "Foreign ordinary Discord bot"
    );
    assert_eq!(
        team::live_specialist(
            &state.db,
            &inspected.foreign_owner,
            &inspected.foreign_agent
        )
        .await
        .unwrap()
        .description,
        "Foreign private specialist"
    );
}

#[test]
fn private_channel_owner_manages_specialist_grants_and_owned_bot_then_logout_restores_faq() {
    crate::test_utils::run_native_journey_test("native_owner_management", || async {
        let mut j = NativeJourney::new().await;
        j.state
            .db
            .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id":&j.profile.id},
                doc! {"$set":{"public_enabled":true}},
            )
            .await
            .unwrap();
        let inspected =
            management_fixture(&j.state, &j.personal, &j.bot, &j.server, j.egress.clone()).await;
        let execution = crate::services::assistant_nyxagent_runtime::producer(
            j.state.clone(),
            inspected.clone(),
            crate::services::platform_public_credential_service::producer(j.state.clone()),
        );
        let mut producers = assemble(
            j.state.clone(),
            execution,
            inspected.clone(),
            Arc::new(TestSettledHook),
        );
        let (sender, finished) = tokio::sync::mpsc::channel(4);
        let inner = producers.transport.clone();
        Arc::get_mut(&mut producers).unwrap().transport = Arc::new(ObservedTransport {
            inner,
            native: Some(j.transport.clone()),
            finished: sender,
        });
        j.producers = producers;
        j.finished = finished;

        j.dispatch(
            j.admission(
                501,
                "ask",
                Some(("question", "Create my specialist and attach my bot")),
            )
            .await,
        )
        .await;
        assert_eq!(
            j.egress.0.load(Ordering::SeqCst),
            0,
            "unlinked text remains public FAQ"
        );
        assert_eq!(
            j.state
                .db
                .collection::<Document>(AGENTS)
                .count_documents(doc! {"user_id":&j.personal})
                .await
                .unwrap(),
            0
        );
        assert!(inspected.retained_key.lock().await.is_none());
        let public: Value = serde_json::from_slice(
            &j.server
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body,
        )
        .unwrap();
        assert!(
            public["content"]
                .as_str()
                .unwrap()
                .contains("FAQ never uses personal data")
        );
        assert_public_faq(&j.state, &j.profile.id).await;

        j.dispatch(j.admission(502, "login", None).await).await;
        let login: Value = serde_json::from_slice(
            &j.server
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body,
        )
        .unwrap();
        let url = reqwest::Url::parse(
            login["content"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .find(|word| word.starts_with("https://") || word.starts_with("http://"))
                .unwrap(),
        )
        .unwrap();
        let token = Zeroizing::new(
            url.fragment()
                .unwrap()
                .strip_prefix("intent_token=")
                .unwrap()
                .to_owned(),
        );
        let review = j
            .browser(
                "POST",
                "/api/v1/platform-personal/review",
                Some(json!({"intent_token":token.as_str()})),
            )
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let approved = j.browser("POST","/api/v1/platform-personal/approve",Some(json!({
            "challenge_id":review["challenge_id"],"review_digest":review["review_digest"],"replace_existing":false}))).await;
        assert_eq!(
            j.state
                .db
                .collection::<Document>(crate::models::platform_personal_binding::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&j.personal,"state":"active"})
                .await
                .unwrap(),
            0,
            "browser consent alone grants no private-channel authority"
        );
        let command = Zeroizing::new(
            approved["confirmation_command"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        j.dispatch(
            j.admission(
                503,
                "confirm",
                Some(("code", command.strip_prefix("/nyx confirm code:").unwrap())),
            )
            .await,
        )
        .await;
        j.dispatch(
            j.admission(510, "ask", Some(("question", "FAQ connect-services")))
                .await,
        )
        .await;
        assert_public_faq(&j.state, &j.profile.id).await;
        assert_eq!(
            j.egress.0.load(Ordering::SeqCst),
            0,
            "explicit linked Discord FAQ never enters personal execution"
        );
        j.dispatch(
            j.admission(
                504,
                "ask",
                Some(("question", "Create my specialist and attach my bot")),
            )
            .await,
        )
        .await;
        let delivered =
            tokio::time::timeout(std::time::Duration::from_secs(30), j.finished.recv()).await;
        let failure = inspected.failure.lock().await.clone();
        assert!(failure.is_none(), "owner tool failure: {failure:?}");
        let (reply, outcome) = delivered.unwrap().unwrap();
        assert!(matches!(outcome, DeliveryOutcome::NativeSent { .. }));
        assert_managed(&inspected, &j.server).await;
        let bots = j
            .state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME);
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            engine::get(&j.state.db, &j.personal, &reply.turn.conversation_id)
                .await
                .unwrap()
                .agent_id,
            Some(
                team::ensure_nyxbot(&j.state.db, &j.personal)
                    .await
                    .unwrap()
                    .id
            )
        );

        j.dispatch(j.admission(505, "logout", None).await).await;
        assert_eq!(
            j.state
                .db
                .collection::<Document>(crate::models::platform_personal_binding::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&j.personal,"state":"active"})
                .await
                .unwrap(),
            0
        );
        let key = inspected.retained_key.lock().await.take().unwrap();
        refused(
            &j.state,
            &key,
            "nyxid__spawn_subagent",
            json!({"name":"after-logout","description":"Must not be created"}),
            ExpectedRefusal::NoLiveTurn,
        )
        .await;
        refused(
            &j.state,
            &key,
            "nyxid__update_channel_bot",
            json!({"bot_id":inspected.owned_bot,"label":"After logout forbidden"}),
            ExpectedRefusal::NoLiveTurn,
        )
        .await;
        j.dispatch(
            j.admission(506, "ask", Some(("question", "Create another specialist")))
                .await,
        )
        .await;
        let public: Value = serde_json::from_slice(
            &j.server
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body,
        )
        .unwrap();
        assert!(
            public["content"]
                .as_str()
                .unwrap()
                .contains("FAQ never uses personal data")
        );
        assert_public_faq(&j.state, &j.profile.id).await;
        assert_eq!(
            j.egress.0.load(Ordering::SeqCst),
            1,
            "logout cannot start another personal turn"
        );
        assert_eq!(
            j.state
                .db
                .collection::<Document>(AGENTS)
                .count_documents(doc! {"user_id":&j.personal,"kind":"specialist"})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            bots.find_one(doc! {"_id":&inspected.owned_bot})
                .await
                .unwrap()
                .unwrap()
                .label,
            "Managed from my private NyxBot"
        );
        j.state.db.drop().await.unwrap();
    });
}

#[test]
fn telegram_private_owner_uses_the_same_specialist_and_channel_tools_then_logout_refuses_them() {
    crate::test_utils::run_native_journey_test("telegram_owner_management", || async {
        let mut j = Journey::new().await;
        let owner = uuid::Uuid::new_v4().to_string();
        j.state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let bot = j
            .state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":&j.profile.channel_bot_id})
            .await
            .unwrap()
            .unwrap();
        j.state
            .db
            .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id":&j.profile.id},
                doc! {"$set":{"public_enabled":true}},
            )
            .await
            .unwrap();
        let inspected =
            management_fixture(&j.state, &owner, &bot, &j.server, j.egress.clone()).await;
        let execution = crate::services::assistant_nyxagent_runtime::producer(
            j.state.clone(),
            inspected.clone(),
            crate::services::platform_public_credential_service::producer(j.state.clone()),
        );
        let mut producers = assemble(
            j.state.clone(),
            execution,
            inspected.clone(),
            Arc::new(TestSettledHook),
        );
        let identity = producers.identity.clone();
        let (sender, mut finished) = tokio::sync::mpsc::channel(4);
        // Observe the same production consumer as Discord; the underlying Telegram
        // transport still uses the existing loopback Telegram API fixture.
        Arc::get_mut(&mut producers).unwrap().transport = Arc::new(ObservedTransport {
            inner: Arc::new(LocalTransport {
                state: j.state.clone(),
                identity,
                base: j.server.uri(),
                personal_gate: None,
            }),
            native: None,
            finished: sender,
        });
        j.producers = producers;
        j.webhook(&j.update(601, "Create my specialist and attach my bot", false))
            .await;
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
        assert!(inspected.retained_key.lock().await.is_none());
        assert_eq!(
            j.state
                .db
                .collection::<Document>(AGENTS)
                .count_documents(doc! {"user_id":&owner})
                .await
                .unwrap(),
            0
        );
        assert!(
            j.sends().await.last().unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("FAQ never uses personal data")
        );
        assert_public_faq(&j.state, &j.profile.id).await;
        assert_eq!(j.update(602, "/login", false)["message"]["from"]["id"], 700);
        super::super::account_parity::confirm_account(&j, 700, &owner, 602).await;
        j.webhook(&j.update(604, "Create my specialist and attach my bot", false))
            .await;
        let delivered =
            tokio::time::timeout(std::time::Duration::from_secs(30), finished.recv()).await;
        let failure = inspected.failure.lock().await.clone();
        assert!(failure.is_none(), "owner tool failure: {failure:?}");
        let (_, outcome) = delivered.unwrap().unwrap();
        assert!(matches!(outcome, DeliveryOutcome::Sent { .. }));
        assert_managed(&inspected, &j.server).await;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        j.webhook(&j.update(605, "/logout", false)).await;
        assert_eq!(
            j.state
                .db
                .collection::<Document>(crate::models::platform_personal_binding::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&owner,"state":"active"})
                .await
                .unwrap(),
            0
        );
        let key = inspected.retained_key.lock().await.take().unwrap();
        refused(
            &j.state,
            &key,
            "nyxid__spawn_subagent",
            json!({"name":"after-logout","description":"Must not be created"}),
            ExpectedRefusal::NoLiveTurn,
        )
        .await;
        refused(
            &j.state,
            &key,
            "nyxid__update_channel_bot",
            json!({"bot_id":inspected.owned_bot,"label":"After logout forbidden"}),
            ExpectedRefusal::NoLiveTurn,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        j.webhook(&j.update(606, "Create another specialist", false))
            .await;
        assert!(
            j.sends().await.last().unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("FAQ never uses personal data")
        );
        assert_public_faq(&j.state, &j.profile.id).await;
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 1);
        assert_managed(&inspected, &j.server).await;
        assert_eq!(
            j.state
                .db
                .collection::<Document>(AGENTS)
                .count_documents(doc! {"user_id":&owner,"kind":"specialist"})
                .await
                .unwrap(),
            1
        );
        j.state.db.drop().await.unwrap();
    });
}
