//! Transport regressions use real thread keys, live Mongo authority and provider stubs.
use super::*;
use crate::models::{
    assistant_conversation::AssistantConversation,
    assistant_sharing::{AgentConsent, ConsentDecision, SharedExecution},
};
use crate::services::{
    assistant_service_execution as execution, proxy_service, shared_agent_consent as consent,
    shared_agent_transport as transport,
};
use mongodb::bson;

struct TransportFixture {
    f: Fixture,
    agent: crate::models::assistant_agent::AssistantAgent,
    row: AssistantConversation,
    owner_connection: String,
    actor_connection: String,
    catalog: String,
}
impl TransportFixture {
    async fn new(name: &str, url: &str) -> Self {
        let f = Fixture::new(name).await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        for actor in [&f.admin, &f.outsider] {
            f.state
                .db
                .collection::<Document>("notification_channels")
                .insert_one(doc! {
                    "_id":Uuid::new_v4().to_string(), "user_id":actor, "approval_required":false,
                    "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now()
                })
                .await
                .unwrap();
        }
        let agent = publish(&f, false).await;
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.slug = "chrono-sandbox".into();
        catalog.base_url = url.into();
        catalog.auth_method = "none".into();
        catalog.requires_user_credential = false;
        catalog.created_by = "admin".into();
        catalog.service_category = "connection".into();
        catalog.identity_propagation_mode = "none".into();
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .insert_one(&catalog)
            .await
            .unwrap();
        let owner_connection = f.service(&f.admin, "shared-transport-owner").await;
        let actor_connection = f.service(&f.outsider, "shared-transport-actor").await;
        f.state
            .db
            .collection::<Document>("user_services")
            .update_many(
                doc! {"_id":{"$in":[&owner_connection,&actor_connection]}},
                doc! {"$set":{"catalog_service_id":&catalog.id,"identity_propagation_mode":"none"}},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("user_endpoints")
            .update_many(
                doc! {"user_id":{"$in":[&f.admin,&f.outsider]}},
                doc! {"$set":{"url":url}},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("assistant_agents")
            .update_one(
                doc! {"_id":&agent.id},
                doc! {"$set":{"grants.service_ids":[&owner_connection]}},
            )
            .await
            .unwrap();
        sharing::update(
            &f.state.db,
            &f.admin,
            &agent.id,
            sharing::SharingUpdate {
                expected_version: 1,
                enabled: true,
                audience: vec![sharing::Audience {
                    subject_kind: ShareSubject::Person,
                    subject_id: f.outsider.clone(),
                    role: ShareRole::User,
                }],
                shared_access: BTreeMap::from([(owner_connection.clone(), GuestAccess::All)]),
            },
            true,
        )
        .await
        .unwrap();
        let row = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.outsider, &agent)
            .await
            .unwrap();
        let row = crate::services::assistant_authority_tests::pin_assignment(
            &f.state.db,
            &f.outsider,
            &row.id,
            None,
        )
        .await;
        Self {
            f,
            agent,
            row,
            owner_connection,
            actor_connection,
            catalog: catalog.id,
        }
    }
    async fn auth(&self) -> crate::mw::auth::AuthUser {
        crate::mw::auth::api_key_auth_user(
            &self.f.state.db,
            &self.f.key(&self.row).await,
            None,
            None,
            None,
        )
        .await
        .unwrap()
    }
    async fn assert_audit(&self, event: &str, actor_connection: bool) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(row) = self
                    .f
                    .state
                    .db
                    .collection::<Document>("audit_log")
                    .find_one(doc! {"event_type": event})
                    .await
                    .unwrap()
                {
                    let data = row.get_document("event_data").unwrap();
                    assert_eq!(data.get_str("execution_mode").unwrap(), "shared");
                    assert_eq!(data.get_str("billing_owner_id").unwrap(), self.f.outsider);
                    assert_eq!(data.get_str("actor_user_id").unwrap(), self.f.outsider);
                    assert_eq!(data.get_str("agent_owner_id").unwrap(), self.f.admin);
                    assert_eq!(
                        data.get_str("credential_source").unwrap(),
                        if actor_connection {
                            "actor_personal"
                        } else {
                            "owner_shared"
                        }
                    );
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn decision(
        &self,
        actor: bool,
        method: &str,
        path: &str,
        websocket: bool,
    ) -> execution::Decision {
        let auth = self.auth().await;
        let id = if actor {
            &self.actor_connection
        } else {
            &self.owner_connection
        };
        let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
            &self.f.state.db,
            &self.f.state.encryption_keys,
            &self.f.outsider,
            id,
            None,
            None,
            proxy_service::ProxyExecutionContext::new(None, self.f.state.platform_user_rate_limit)
                .with_assistant_access(auth.org_agent_access.as_deref()),
        )
        .await
        .unwrap()
        .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        if websocket {
            headers.insert("upgrade", "websocket".parse().unwrap());
        }
        execution::check_http(
            &self.f.state,
            &auth,
            Some(id),
            Some(if actor {
                &self.f.outsider
            } else {
                &self.f.admin
            }),
            &resolved.target,
            method,
            path,
            None,
            b"",
            None,
            None,
            &headers,
        )
        .await
        .unwrap()
    }
    async fn decide(&self, choice: ConsentDecision) {
        let row = self
            .f
            .state
            .db
            .collection::<AgentConsent>("assistant_agent_consents")
            .find_one(doc! {"decision":"pending"})
            .await
            .unwrap()
            .unwrap();
        consent::decide(&self.f.state.db, &self.f.outsider, &row.id, choice)
            .await
            .unwrap();
    }
    async fn revoke(&self) {
        sharing::update(
            &self.f.state.db,
            &self.f.admin,
            &self.agent.id,
            sharing::SharingUpdate {
                expected_version: 2,
                enabled: false,
                audience: vec![],
                shared_access: BTreeMap::new(),
            },
            true,
        )
        .await
        .unwrap();
    }
    async fn binding(&self, actor: bool, method: &str, websocket: bool) -> SharedExecution {
        if actor {
            assert!(
                self.decision(true, method, "/work", websocket)
                    .await
                    .refusal
                    .is_some()
            );
            self.decide(ConsentDecision::Once).await;
        }
        self.decision(actor, method, "/work", websocket)
            .await
            .admission
            .expect("admitted")
            .shared_execution()
            .unwrap()
            .unwrap()
    }
}

#[tokio::test]
async fn shared_transports_durable_bindings_preserve_payer_consent_and_live_revocation() {
    for (name, method, websocket) in [
        ("managed", "POST", false),
        ("watch", "POST", false),
        ("websocket", "GET", true),
        ("connect", "CONNECT", false),
    ] {
        for actor_connection in [false, true] {
            let f = TransportFixture::new(
                &format!("shared_transport_{name}"),
                "https://example.invalid",
            )
            .await;
            let binding = f.binding(actor_connection, method, websocket).await;
            assert_eq!(binding.actor, f.f.outsider);
            assert_eq!(binding.owner, f.f.admin);
            assert_eq!(binding.consent_id.is_some(), actor_connection);
            transport::revalidate(&f.f.state.db, &binding)
                .await
                .unwrap();
            let auth = f.auth().await;
            for class in [
                CredentialClass::UserOwned,
                CredentialClass::NyxidManagedMaster,
            ] {
                let payer =
                    f.f.state
                        .billing
                        .owner_resolver()
                        .resolve_for_assistant_execution(
                            &f.f.outsider,
                            if actor_connection {
                                &f.f.outsider
                            } else {
                                &f.f.admin
                            },
                            class,
                            auth.org_agent_access.as_deref(),
                        )
                        .await
                        .unwrap();
                assert_eq!(payer.owner_id, f.f.outsider);
            }
            let mut event = json!({});
            transport::annotate(&mut event, Some(&binding));
            assert_eq!(event["billing_owner_id"], f.f.outsider);
            assert_eq!(event["execution_mode"], "shared");
            assert_eq!(event["agent_owner_id"], f.f.admin);
            assert!(!event.to_string().contains(&binding.key));
            if actor_connection {
                assert!(
                    f.decision(true, method, "/work", websocket)
                        .await
                        .refusal
                        .is_some(),
                    "Once cannot authorize a second {name}"
                );
            }
            // A detached continuation can outlive the initiating turn, but ordinary key auth cannot.
            f.f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"active_turn":null}})
                .await
                .unwrap();
            transport::revalidate(&f.f.state.db, &binding)
                .await
                .unwrap();
            assert!(
                crate::mw::auth::api_key_auth_user(
                    &f.f.state.db,
                    &f.f.key(&f.row).await,
                    None,
                    None,
                    None
                )
                .await
                .is_err()
            );
            f.revoke().await;
            assert!(
                transport::revalidate(&f.f.state.db, &binding)
                    .await
                    .is_err(),
                "revoked {name}"
            );
            f.f.state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn shared_transports_stop_policy_behavior_and_readiness_fence_every_effect() {
    let f = TransportFixture::new("shared_transport_fences", "https://example.invalid").await;
    let binding = f.binding(false, "POST", false).await;
    f.f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        transport::revalidate(&f.f.state.db, &binding)
            .await
            .is_err()
    );
    f.f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.stop_requested":false}},
        )
        .await
        .unwrap();
    transport::revalidate(&f.f.state.db, &binding)
        .await
        .unwrap();
    f.f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":&f.agent.id},
            doc! {"$set":{"persona":"Changed behavior"}},
        )
        .await
        .unwrap();
    assert!(
        transport::revalidate(&f.f.state.db, &binding)
            .await
            .is_err()
    );
    f.f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":&f.agent.id},
            doc! {"$set":{"persona":&f.agent.persona}},
        )
        .await
        .unwrap();
    transport::revalidate(&f.f.state.db, &binding)
        .await
        .unwrap();
    f.f.state
        .db
        .collection::<Document>("security_replicas")
        .update_many(
            doc! {},
            doc! {"$pull":{"capabilities":transport::READINESS}},
        )
        .await
        .unwrap();
    assert!(
        transport::revalidate(&f.f.state.db, &binding)
            .await
            .is_err()
    );
    crate::services::security_readiness_service::ready_test_replica(&f.f.state.db).await;
    flags::set_platform_override(
        &f.f.state.db,
        sharing::FLAG,
        &flags::FlagTarget::Global,
        false,
        &f.f.admin,
    )
    .await
    .unwrap();
    assert!(
        transport::revalidate(&f.f.state.db, &binding)
            .await
            .is_err()
    );
    f.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_transports_discovery_keeps_machine_and_account_authority_excluded() {
    let f = TransportFixture::new("shared_transport_tools", "https://example.invalid").await;
    let auth = f.auth().await;
    let chat = auth.assistant_chat.as_deref().unwrap();
    let tools =
        crate::services::assistant_account_tools::virtual_service_for(&f.f.state.db, chat).await;
    assert!(
        tools
            .endpoints
            .iter()
            .any(|e| e.name == "list_channel_chats")
    );
    assert!(tools.endpoints.iter().any(|e| e.name == "post_to_chat"));
    for name in [
        "list_subagents",
        "list_services",
        "set_machine_capabilities",
        "create_automation",
        "link_channel_agent",
    ] {
        assert!(!tools.endpoints.iter().any(|e| e.name == name));
        let (_, refused) = crate::handlers::assistant_team::execute_tool(
            &f.f.state,
            chat,
            &format!("nyxid__{name}"),
            &json!({}),
        )
        .await;
        assert!(refused, "{name}");
    }
    f.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_transports_async_mcp_survives_settlement_and_two_replica_polling() {
    use crate::models::async_service_operation::{
        AsyncServiceOperation, COLLECTION_NAME as WATCHES,
    };
    use crate::services::async_service_operation as watches;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    for (outcome, actor_connection) in [
        ("completed", false),
        ("revoked", false),
        ("stopped", false),
        ("completed", true),
        ("revoked", true),
        ("stopped", true),
    ] {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/executions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"operation_id":"op-transport","status":"queued"})),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        Mock::given(method("GET"))
            .and(path("/executions/op-transport"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"succeeded"})))
            .expect(if outcome == "completed" { 1 } else { 0 })
            .mount(&upstream)
            .await;
        Mock::given(method("GET"))
            .and(path("/executions/op-transport/result"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"output":"quoted provider result"})),
            )
            .expect(if outcome == "completed" { 1 } else { 0 })
            .mount(&upstream)
            .await;
        Mock::given(method("POST"))
            .and(path("/executions/op-transport/cancel"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"cancelled"})))
            .expect(if outcome == "stopped" { 1 } else { 0 })
            .mount(&upstream)
            .await;
        let f = TransportFixture::new("shared_transport_async", &upstream.uri()).await;
        watches::ensure_indexes(&f.f.state.db).await.unwrap();
        // Hold the normal owner pool so the sweep queues the event without a
        // detached worker racing these delivery assertions. Admission below
        // consumes it through the real event-turn pipeline.
        let limit =
            crate::handlers::assistant_team::team_pool_limit(&f.f.state, &f.f.outsider).await + 1;
        let mut permits = Vec::new();
        for _ in 0..limit {
            permits.push(
                f.f.state
                    .direct_chat_limiter
                    .try_acquire_pool("assistant_team", &f.f.outsider, limit)
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        crate::services::catalog_spec_sync::sync_seeded_service_endpoints(&f.f.state.db)
            .await
            .unwrap();
        let credential =
            crate::services::assistant_agent_credential_service::load_for_conversation(
                &f.f.state.db,
                &f.f.state.encryption_keys,
                &f.f.outsider,
                &f.row.id,
            )
            .await
            .unwrap()
            .unwrap();
        let listed = mcp(&f.f.state, &credential.raw_key, "tools/list", json!({})).await;
        let name = listed["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .find(|t| {
                t["shared_connection"]["connection_id"]
                    == if actor_connection {
                        f.actor_connection.as_str()
                    } else {
                        f.owner_connection.as_str()
                    }
                    && t["name"]
                        .as_str()
                        .is_some_and(|n| n.ends_with("__submit_execution_handler"))
            })
            .unwrap_or_else(|| panic!("submit missing: {listed}"))["name"]
            .clone();
        if actor_connection {
            let prompt=mcp(&f.f.state,&credential.raw_key,"tools/call",json!({"name":name,
                "arguments":{"Idempotency-Key":"submit-shared","script":"print(1)","language":"python"}})).await;
            assert_eq!(prompt["result"]["isError"], true, "{prompt}");
            assert_eq!(
                f.f.state
                    .db
                    .collection::<Document>(WATCHES)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "a consent prompt must not leave an unsubmitted watch"
            );
            f.decide(ConsentDecision::Once).await;
        }
        let submitted=mcp(&f.f.state,&credential.raw_key,"tools/call",json!({"name":name,
            "arguments":{"Idempotency-Key":"submit-shared","script":"print(1)","language":"python"}})).await;
        assert_eq!(submitted["result"]["isError"], false, "{submitted}");
        let watch =
            f.f.state
                .db
                .collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        let binding = watch
            .shared_execution
            .as_ref()
            .expect("durable shared binding");
        assert_eq!(binding.actor, f.f.outsider);
        assert_eq!(binding.owner, f.f.admin);
        assert_eq!(binding.consent_id.is_some(), actor_connection);
        if actor_connection {
            let consumed =
                f.f.state
                    .db
                    .collection::<AgentConsent>("assistant_agent_consents")
                    .find_one(doc! {"decision":"consumed"})
                    .await
                    .unwrap()
                    .unwrap();
            assert!(
                consumed.expires_at.unwrap() > chrono::Utc::now() + chrono::Duration::minutes(90)
            );
            let second=mcp(&f.f.state,&credential.raw_key,"tools/call",json!({"name":name,
                "arguments":{"Idempotency-Key":"different-job","script":"print(2)","language":"python"}})).await;
            assert_eq!(
                second["result"]["isError"], true,
                "Once cannot authorize a second job: {second}"
            );
            assert_eq!(
                f.f.state
                    .db
                    .collection::<Document>(WATCHES)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1,
                "only the submitted job retains a watch"
            );
        }
        if outcome == "stopped" {
            f.f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await
                .unwrap();
            crate::handlers::mcp_transport::async_operations::cancel_conversation(
                &f.f.state,
                &f.f.outsider,
                &f.row.id,
            )
            .await
            .unwrap();
        } else {
            f.f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"active_turn":null}})
                .await
                .unwrap();
            if outcome == "revoked" {
                f.revoke().await;
            }
            f.f.state
                .db
                .collection::<Document>(WATCHES)
                .update_many(
                    doc! {},
                    doc! {"$set":{"poll_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            let (a, b) = tokio::join!(
                crate::handlers::mcp_transport::async_operations::sweep(&f.f.state),
                crate::handlers::mcp_transport::async_operations::sweep(&f.f.state)
            );
            a.unwrap();
            b.unwrap();
            crate::handlers::mcp_transport::async_operations::sweep(&f.f.state)
                .await
                .unwrap();
        }
        let final_watch =
            f.f.state
                .db
                .collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&watch.id})
                .await
                .unwrap()
                .unwrap();
        if outcome == "completed" {
            assert_eq!(final_watch.reason.as_deref(), Some("completed"));
            assert!(final_watch.result_encrypted.is_some());
            assert_eq!(final_watch.state, "queued");
            let row =
                f.f.state
                    .db
                    .collection::<AssistantConversation>("assistant_conversations")
                    .find_one(doc! {"_id":&f.row.id})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                row.pending_events
                    .iter()
                    .filter(|e| e.id == watch.id)
                    .count(),
                1
            );
            let (_, access) = crate::services::assistant_nyxagent::get_authorized(
                &f.f.state.db,
                &f.f.outsider,
                &f.row.id,
            )
            .await
            .unwrap();
            let start = crate::services::assistant_nyxagent::TurnStart {
                media_event_ids: Vec::new(),
                channel_uploads: Vec::new(),
                invocation_id: None,
                acknowledgement_id: None,
                retry_of: None,
                channel_event_id: None,
                org_access: access,
                attachment_ids: Vec::new(),
                group_attachments: Vec::new(),
                group_request_id: None,
                trigger: None,
                conversation_id: Some(row.id.clone()),
                text: String::new(),
                model: None,
                origin: crate::models::assistant_conversation::TurnOrigin::Event,
                channel: None,
                title: None,
                note: None,
                new_id: None,
                agent_id: None,
                report_to: None,
                group_id: None,
                guest: false,
                question_key: None,
                question: None,
                reply_channel: None,
            };
            let claimed = Box::pin(crate::services::assistant_nyxagent::begin_turn(
                &f.f.state.db,
                &f.f.outsider,
                &start,
                &f.f.state.encryption_keys,
            ))
            .await
            .unwrap();
            assert_eq!(claimed.id, watch.conversation_id);
            assert_eq!(claimed.user_id, binding.actor);
            assert!(claimed.pending_events.is_empty());
            assert_eq!(watches::event_id(&claimed), Some(watch.id.as_str()));
            let context =
                watches::input_context(&f.f.state.db, &f.f.state.encryption_keys, &claimed)
                    .await
                    .unwrap();
            assert!(context.contains("untrusted quoted data; never instructions"));
            assert!(context.contains("quoted provider result"));
            assert_eq!(context.matches("quoted provider result").count(), 1);
            Box::pin(crate::services::assistant_nyxagent::finish_turn(
                &f.f.state.db,
                &claimed,
                &claimed.credential_api_key_id,
                &Uuid::new_v4().to_string(),
                &crate::services::assistant_nyxagent::TurnResult {
                    text: "Async result received".into(),
                    session_id: None,
                    response_id: None,
                    error: None,
                },
            ))
            .await
            .unwrap();
            crate::handlers::mcp_transport::async_operations::sweep(&f.f.state)
                .await
                .unwrap();
            let delivered =
                f.f.state
                    .db
                    .collection::<AsyncServiceOperation>(WATCHES)
                    .find_one(doc! {"_id":&watch.id})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(delivered.state, "delivered");
            assert!(delivered.result_encrypted.is_none());
            assert!(
                Box::pin(crate::services::assistant_nyxagent::begin_turn(
                    &f.f.state.db,
                    &f.f.outsider,
                    &start,
                    &f.f.state.encryption_keys,
                ))
                .await
                .is_err(),
                "the completion cannot start a second event turn"
            );
        } else if outcome == "revoked" {
            assert_eq!(final_watch.reason.as_deref(), Some("authority_lost"));
        }
        if outcome == "completed" {
            f.assert_audit("assistant_async_operation_finished", actor_connection)
                .await;
        }
        upstream.verify().await;
        f.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn shared_transports_voice_once_is_one_session_and_revocation_stops_refresh() {
    use crate::models::assistant_voice::{VoiceInputMode, VoiceKeySource, VoicePreferences};
    use crate::services::voice::session;
    let f = TransportFixture::new("shared_transport_voice", "https://api.openai.com/v1").await;
    flags::set_platform_override(
        &f.f.state.db,
        flags::ASSISTANT_VOICE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.f.admin,
    )
    .await
    .unwrap();
    session::ensure_indexes(&f.f.state.db).await.unwrap();
    // Human voice starts may use an idle private shared thread, never ordinary idle bearer execution.
    f.f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"active_turn":null}})
        .await
        .unwrap();
    let preferences = VoicePreferences {
        service_id: f.catalog.clone(),
        connection_id: Some(f.actor_connection.clone()),
        model: "gpt-realtime".into(),
        voice: None,
        key_source: VoiceKeySource::Own,
        input_mode: VoiceInputMode::PushToTalk,
        language: None,
        notify_on_completion: false,
    };
    let request = Uuid::new_v4().to_string();
    assert!(
        session::admit(
            &f.f.state.db,
            &f.f.outsider,
            &f.row.id,
            &request,
            preferences.clone(),
            "identity".into(),
            "worker"
        )
        .await
        .is_err()
    );
    f.decide(ConsentDecision::Once).await;
    let call = session::admit(
        &f.f.state.db,
        &f.f.outsider,
        &f.row.id,
        &request,
        preferences.clone(),
        "identity".into(),
        "worker",
    )
    .await
    .unwrap();
    assert_eq!(call.shared_execution.as_ref().unwrap().actor, f.f.outsider);
    session::refresh(&f.f.state.db, &call).await.unwrap();
    session::close(&f.f.state.db, &call, "test_closed", true)
        .await
        .unwrap();
    assert!(
        session::admit(
            &f.f.state.db,
            &f.f.outsider,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            preferences,
            "identity".into(),
            "worker"
        )
        .await
        .is_err()
    );
    f.decide(ConsentDecision::Always).await;
    f.revoke().await;
    assert!(session::refresh(&f.f.state.db, &call).await.is_err());
    f.f.state.db.drop().await.unwrap();
}

async fn proxy_server(f: &TransportFixture) -> (String, tokio::task::JoinHandle<()>) {
    use crate::services::billing::route_inventory::{BillingIngress, BillingRoutePolicy};
    use axum::{
        Router,
        body::Body,
        extract::{Path, State},
        http::Request,
        routing::any,
    };
    let state = f.f.state.clone();
    let key = f.f.key(&f.row).await;
    let router = Router::new().fallback(any(move |mut request: Request<Body>| {
        let state = state.clone();
        let key = key.clone();
        async move {
            let auth =
                crate::mw::auth::api_key_auth_user(&state.db, &key, None, None, None).await?;
            request
                .extensions_mut()
                .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
            Box::pin(crate::handlers::proxy::proxy_request_by_slug(
                State(state),
                auth,
                Default::default(),
                Path(("shared-transport-owner".into(), "socket".into())),
                request,
            ))
            .await
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, task)
}

#[tokio::test]
async fn shared_transports_websocket_rechecks_before_frames_and_settles_actor() {
    use axum::{
        Router,
        extract::ws::{Message, WebSocketUpgrade},
        routing::get,
    };
    use futures::{SinkExt, StreamExt};
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let upstream = Router::new().route(
        "/socket",
        get(move |ws: WebSocketUpgrade| {
            let seen = seen.clone();
            async move {
                ws.on_upgrade(move |mut socket| async move {
                    while let Some(Ok(message)) = socket.recv().await {
                        if let Message::Text(text) = message {
                            seen.fetch_add(1, Ordering::SeqCst);
                            if socket.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                    }
                })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream_task = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let f = TransportFixture::new("shared_transport_ws", &upstream_url).await;
    let (url, server) = proxy_server(&f).await;
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("{}/socket", url.replace("http:", "ws:")))
            .await
            .unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "first".into(),
        ))
        .await
        .unwrap();
    let echo = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(echo.to_text().unwrap(), "first");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    f.revoke().await;
    let _ = socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "after revocation".into(),
        ))
        .await;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    assert!(
        closed.is_none()
            || closed.is_some_and(|v| v.is_err()
                || matches!(v, Ok(tokio_tungstenite::tungstenite::Message::Close(_))))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        tokio_tungstenite::connect_async(format!("{}/socket", url.replace("http:", "ws:")))
            .await
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(event) =
                f.f.state
                    .db
                    .collection::<Document>("audit_log")
                    .find_one(doc! {"event_type":"proxy_ws_disconnect"})
                    .await
                    .unwrap()
            {
                let metadata = event.get_document("event_data").unwrap();
                assert_eq!(metadata.get_str("billing_owner_id").unwrap(), f.f.outsider);
                assert_eq!(metadata.get_str("agent_owner_id").unwrap(), f.f.admin);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    server.abort();
    upstream_task.abort();
    f.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_transports_pool_exact_member_no_silent_failover() {
    use crate::models::service_pool::{PoolMemberContract, PoolStrategy, TierBalance};
    use crate::services::billing::route_inventory::{BillingIngress, BillingRoutePolicy};
    use crate::services::service_pool_service::{self as pools, CreatePoolInput};
    use axum::{
        body::Body,
        extract::{Path, State},
        http::Request,
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    for strategy in [PoolStrategy::Priority, PoolStrategy::RoundRobin] {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/records"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .expect(1)
            .mount(&upstream)
            .await;
        let f = TransportFixture::new("shared_transport_pool", &upstream.uri()).await;
        let member = bson::from_document(doc! {"user_service_id":&f.owner_connection}).unwrap();
        pools::create_pool(
            &f.f.state.db,
            &f.f.admin,
            CreatePoolInput {
                slug: "shared-pool".into(),
                name: "Shared pool".into(),
                description: None,
                strategy,
                tier_balance: TierBalance::RoundRobin,
                member_contract: PoolMemberContract::SameApi,
                failover: None,
                members: vec![member],
                is_active: Some(true),
            },
        )
        .await
        .unwrap();
        let auth = f.auth().await;
        let mut request = Request::builder()
            .uri("/api/v1/proxy/s/shared-pool/records")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
        let response = Box::pin(crate::handlers::proxy::proxy_request_by_slug(
            State(f.f.state.clone()),
            auth.clone(),
            Default::default(),
            Path(("shared-pool".into(), "records".into())),
            request,
        ))
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let pin =
            f.f.state
                .db
                .collection::<Document>(execution::COLLECTION)
                .find_one(doc! {"kind":"shared_pool_selection"})
                .await
                .unwrap();
        assert!(pin.is_some(), "pool selection pinned before dispatch");
        f.revoke().await;
        let stale = crate::services::shared_agent_service::load(
            &f.f.state.db,
            &f.f.outsider,
            Some(std::slice::from_ref(&f.agent.id)),
        )
        .await
        .unwrap();
        assert!(stale.agents.is_empty());
        // A request authenticated before revocation must also fail at fresh
        // pool admission/egress, without reaching either provider attempt.
        let mut request = Request::builder()
            .uri("/api/v1/proxy/s/shared-pool/records")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
        assert!(
            Box::pin(crate::handlers::proxy::proxy_request_by_slug(
                State(f.f.state.clone()),
                auth,
                Default::default(),
                Path(("shared-pool".into(), "records".into())),
                request,
            ))
            .await
            .is_err()
        );
        upstream.verify().await;
        f.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn shared_transports_connect_fixed_origin_stops_before_next_buffer() {
    use std::sync::{Arc, atomic::AtomicUsize};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let effects = Arc::new(AtomicUsize::new(0));
    let count = effects.clone();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(socket.read_u8().await.unwrap());
        }
        assert!(headers.starts_with(b"CONNECT "));
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let mut data = [0u8; 64];
        while let Ok(n) = socket.read(&mut data).await {
            if n == 0 {
                break;
            }
            count.fetch_add(n, Ordering::SeqCst);
            if socket.write_all(&data[..n]).await.is_err() {
                break;
            }
        }
    });
    let f = TransportFixture::new("shared_transport_connect", &upstream_url).await;
    let (url, server) = proxy_server(&f).await;
    let mut client = tokio::net::TcpStream::connect(url.trim_start_matches("http://"))
        .await
        .unwrap();
    client
        .write_all(b"CONNECT /socket HTTP/1.1\r\nHost: ignored.invalid\r\n\r\n")
        .await
        .unwrap();
    let mut headers = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(client.read_u8().await.unwrap());
        }
    })
    .await
    .unwrap();
    assert!(
        headers.starts_with(b"HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&headers)
    );
    client.write_all(b"first").await.unwrap();
    let mut echo = [0u8; 5];
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.read_exact(&mut echo),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&echo, b"first");
    f.revoke().await;
    let _ = client.write_all(b"after").await;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(10), client.read(&mut echo))
        .await
        .unwrap();
    assert!(closed.is_err() || closed.unwrap() == 0);
    assert_eq!(effects.load(Ordering::SeqCst), 5);
    f.assert_audit("proxy_connect_closed", false).await;
    server.abort();
    upstream.abort();
    f.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_transports_platform_grants_are_recipient_scoped_and_live() {
    let f = TransportFixture::new("shared_transport_platform_acl", "https://example.invalid").await;
    let catalogs = f.f.state.db.collection::<Document>("downstream_services");
    catalogs.update_one(doc! {"_id":&f.catalog}, doc! {"$set":{
        "auth_method":"bearer", "credential_encrypted": bson::Binary {subtype:bson::spec::BinarySubtype::Generic,
            bytes:f.f.state.encryption_keys.encrypt(b"test-platform-key").await.unwrap()},
        "platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[&f.f.admin]}
    }}).await.unwrap();
    f.f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&f.owner_connection},
            doc! {"$set":{"credential_binding":"platform"}},
        )
        .await
        .unwrap();
    let auth = f.auth().await;
    let resolve = || {
        proxy_service::resolve_proxy_target_by_user_service_id(
            &f.f.state.db,
            &f.f.state.encryption_keys,
            &f.f.outsider,
            &f.owner_connection,
            None,
            None,
            proxy_service::ProxyExecutionContext::new(None, f.f.state.platform_user_rate_limit)
                .with_assistant_access(auth.org_agent_access.as_deref()),
        )
    };
    assert!(
        resolve().await.is_err(),
        "a share cannot borrow the publisher's platform grant"
    );
    catalogs
        .update_one(
            doc! {"_id":&f.catalog},
            doc! {"$set":{"platform_key.allowed_owner_ids":[&f.f.admin,&f.f.outsider]}},
        )
        .await
        .unwrap();
    assert!(resolve().await.unwrap().is_some());
    let binding = f.binding(false, "GET", true).await;
    transport::revalidate(&f.f.state.db, &binding)
        .await
        .unwrap();
    catalogs
        .update_one(
            doc! {"_id":&f.catalog},
            doc! {"$set":{"platform_key.allowed_owner_ids":[&f.f.admin]}},
        )
        .await
        .unwrap();
    assert!(
        transport::revalidate(&f.f.state.db, &binding)
            .await
            .is_err()
    );
    f.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_transports_readiness_refuses_new_entries_preserves_foreground() {
    use crate::errors::AppError;
    let f = TransportFixture::new("shared_transport_disabled", "https://example.invalid").await;
    let auth = f.auth().await;
    let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
        &f.f.state.db,
        &f.f.state.encryption_keys,
        &f.f.outsider,
        &f.owner_connection,
        None,
        None,
        proxy_service::ProxyExecutionContext::new(None, f.f.state.platform_user_rate_limit)
            .with_assistant_access(auth.org_agent_access.as_deref()),
    )
    .await
    .unwrap()
    .unwrap();
    f.f.state
        .db
        .collection::<Document>("security_replicas")
        .update_many(
            doc! {},
            doc! {"$pull":{"capabilities":transport::READINESS}},
        )
        .await
        .unwrap();
    for (method, websocket) in [("GET", true), ("CONNECT", false), ("GET", false)] {
        let mut headers = axum::http::HeaderMap::new();
        if websocket {
            headers.insert("upgrade", "websocket".parse().unwrap());
        }
        let result = execution::check_http(
            &f.f.state,
            &auth,
            Some(&f.owner_connection),
            Some(&f.f.admin),
            &resolved.target,
            method,
            "/work",
            None,
            b"",
            None,
            None,
            &headers,
        )
        .await;
        if websocket || method == "CONNECT" {
            assert!(matches!(
                result,
                Err(AppError::SecurityReadinessUnavailable)
            ));
        } else {
            assert!(
                result.unwrap().admission.is_some(),
                "Phase 1 foreground remains available"
            );
        }
    }
    assert!(matches!(
        crate::services::assistant_voice::thread(&f.f.state.db, &f.f.outsider, &f.row.id).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(matches!(
        crate::services::service_pool_routing::select_slug_metadata(
            &f.f.state.db,
            &f.f.state.encryption_keys,
            None,
            None,
            &f.f.outsider,
            "pool-route",
            auth.org_agent_access.as_deref()
        )
        .await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    flags::set_platform_override(
        &f.f.state.db,
        sharing::FLAG,
        &flags::FlagTarget::Global,
        false,
        &f.f.admin,
    )
    .await
    .unwrap();
    assert!(
        crate::mw::auth::api_key_auth_user(&f.f.state.db, &f.f.key(&f.row).await, None, None, None)
            .await
            .is_err()
    );
    f.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_transports_opaque_streams_cannot_bypass_operation_policies() {
    use wiremock::MockServer;
    let upstream = MockServer::start().await;
    let f = TransportFixture::new("shared_transport_stream_policy", &upstream.uri()).await;
    // Even a scope that permits the CONNECT handshake cannot authorize the
    // opaque protocol carried inside it.
    let mut scoped = f.auth().await;
    scoped.assistant_operation_scopes.insert(
        f.owner_connection.clone(),
        bson::from_document(doc! {"revision":1_i64,"catalog_service_id":&f.catalog,
        "operations":[{"endpoint_id":null,"rule":{"method":"CONNECT",
            "path_template":"/socket","path_parameter_constraints":{}}}]})
        .unwrap(),
    );
    let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
        &f.f.state.db,
        &f.f.state.encryption_keys,
        &f.f.outsider,
        &f.owner_connection,
        None,
        None,
        proxy_service::ProxyExecutionContext::new(None, f.f.state.platform_user_rate_limit)
            .with_assistant_access(scoped.org_agent_access.as_deref()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        execution::check_http(
            &f.f.state,
            &scoped,
            Some(&f.owner_connection),
            Some(&f.f.admin),
            &resolved.target,
            "CONNECT",
            "/socket",
            None,
            b"",
            None,
            None,
            &axum::http::HeaderMap::new(),
        )
        .await
        .is_err()
    );
    let (url, server) = proxy_server(&f).await;
    for principal in [&f.f.admin, &f.f.outsider] {
        for (effect, service) in [
            ("deny", &f.owner_connection),
            ("require_approval", &f.owner_connection),
            ("deny", &f.catalog),
            ("require_approval", &f.catalog),
        ] {
            f.f.state
                .db
                .collection::<Document>("service_approval_configs")
                .insert_one(doc! {
                    "_id":Uuid::new_v4().to_string(), "user_id":principal,
                    "service_id":service, "service_name":"stream",
                    "approval_required":false, "approval_mode":"per_request",
                    "rules":[{"methods":["POST"],"resource_pattern":"/writes/*",
                        "verbs":[], "effect":effect,"mode":"per_request"}],
                    "default_effect":"auto_allow",
                    "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now()
                })
                .await
                .unwrap();
            // GET /socket itself is allowed, but subsequent frames are opaque.
            assert!(
                tokio_tungstenite::connect_async(format!("{}/socket", url.replace("http:", "ws:")))
                    .await
                    .is_err()
            );
            assert!(upstream.received_requests().await.unwrap().is_empty());
            f.f.state
                .db
                .collection::<Document>("service_approval_configs")
                .delete_many(doc! {})
                .await
                .unwrap();
        }
    }
    f.f.state.db.collection::<Document>("downstream_services").update_one(
        doc! {"_id":&f.catalog},
        doc! {"$set":{"proxy_operation_policy":{"rules":[{"method":"GET","path_template":"/socket","path_parameter_constraints":{}}]}}},
    ).await.unwrap();
    assert!(
        tokio_tungstenite::connect_async(format!("{}/socket", url.replace("http:", "ws:")))
            .await
            .is_err()
    );
    assert!(upstream.received_requests().await.unwrap().is_empty());
    server.abort();
    f.f.state.db.drop().await.unwrap();
}
