use super::*;
use crate::{
    models::{
        assistant_conversation::{ChannelOrigin, ChannelTurnContext, TurnOrigin},
        catalog_skill_revision::{SkillPin, SkillReference},
        channel_thread::ThreadSenderKind,
    },
    services::{
        assistant_authority_tests::orchestrator_fixture,
        assistant_nyxagent as engine,
        channel_guest_access::{self as guest, AnswerAudience, GuestAdmission, VerifiedGuestInput},
        feature_flag_admin_service as flag_admin,
    },
    test_utils::test_auto_connected_catalog_service,
};
use mongodb::bson::{self, Document, doc};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn archive(main: &str, supporting: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, text) in [("SKILL.md", main), ("references/help.md", supporting)] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn id() -> String {
    Uuid::new_v4().to_string()
}

async fn activate_guests(db: &mongodb::Database, owner: &str) {
    use crate::{
        models::feature_flag_override::FlagTargetKind, services::audit_service::AuditActor,
    };
    crate::services::security_readiness_service::ready_test_replica(db).await;
    let auth = crate::test_utils::test_auth_user(owner);
    guest::set_pilot(
        db,
        guest::PilotSelection {
            bot_ids: None,
            fleet: true,
        },
        &AuditActor::from_auth_user(&auth),
    )
    .await
    .unwrap();
    flag_admin::mutate(
        db,
        guest::FLAG,
        flag_admin::Mutation::Override {
            kind: FlagTargetKind::Global,
            target_id: None,
            enabled: Some(true),
        },
        &AuditActor::from_auth_user(&auth),
    )
    .await
    .unwrap();
}

async fn claim_source_event(
    db: &mongodb::Database,
    id: &str,
    channel: &crate::models::nyxbot_channel::NyxbotChannel,
    origin: &ChannelOrigin,
    sender: &str,
) {
    let now = chrono::Utc::now();
    db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .insert_one(doc! {
            "_id": id, "event_id": id, "user_id": &channel.user_id,
            "channel_id": &channel.id, "partition": &origin.partition, "status": "running",
            "created_at": bson::DateTime::from_chrono(now),
            "expires_at": bson::DateTime::from_chrono(now + chrono::Duration::hours(1)),
            "delivery": {
                "version": 1, "state": "pending", "guest": true, "addressed": true,
                "origin": bson::to_bson(origin).unwrap(), "sender_id": sender,
                "transport": &channel.transport, "source_route_id": &channel.route_id,
                "source_route_api_key_id": &channel.route_api_key_id,
                "stream_deadline": bson::DateTime::from_chrono(now + chrono::Duration::minutes(9)),
                "checked_at": bson::DateTime::from_chrono(now),
            },
        })
        .await
        .unwrap();
}

async fn mcp(router: &axum::Router, key: &str, method: &str, params: Value) -> Value {
    use tower::ServiceExt;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("x-api-key", key)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), http::StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65_536).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_guest_skill_exact_file_preload_and_live_revocation() {
    let f = orchestrator_fixture("channel_guest_skill").await;
    let state = &f.state;
    let db = &state.db;
    let agent_id = &f.chat.agent_id;
    let bot_id = id();
    let binding_id = id();
    let event_id = id();
    activate_guests(db, &f.owner).await;
    db.collection::<Document>("channel_bots").insert_one(doc! {
        "_id": &bot_id, "user_id": &f.owner, "platform":"telegram", "label":"Help bot",
        "platform_bot_id":"123", "platform_bot_username":"helper",
        "guest_access_v1": true, "guest_access_v1_revision": 1_i64,
        "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
        "webhook_secret_hash":"hash", "webhook_registered":true, "status":"active", "is_active":true,
        "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now()
    }).await.unwrap();
    db.collection::<Document>("nyxbot_channels").insert_one(doc! {
        "_id": &binding_id, "user_id": &f.owner, "channel_bot_id": &bot_id, "agent_id": agent_id,
        "platform":"telegram", "bot_label":"Help bot", "transport":"direct", "status":"active",
        "route_api_key_id":id(), "private_chats":"everyone",
        "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now()
    }).await.unwrap();
    let root_contents = archive("public pinned guide", "root supporting file");
    let dependency_contents = archive("dependency instructions", "dependency supporting file");
    let root = SkillReference {
        source: "ornn".into(),
        skill_id: id(),
        name: "root".into(),
        version: "1.0".into(),
        sha256: hex::encode(Sha256::digest(&root_contents)),
        disabled: false,
        dependencies: vec![SkillPin {
            source: "ornn".into(),
            skill_id: id(),
            name: "dependency".into(),
            version: "2.0".into(),
            sha256: hex::encode(Sha256::digest(&dependency_contents)),
        }],
    };
    let agents = db.collection::<Document>("assistant_agents");
    agents
        .update_one(
            doc! {"_id":agent_id},
            doc! {"$set":{
                "skills":bson::to_bson(&vec![&root]).unwrap(), "skills_revision":1_i64,
                "skill_metadata": {&root.skill_id: {"description":"private metadata",
                    "size_bytes":100, "preload_paths":["SKILL.md"], "preload_bytes":100}}
            }},
        )
        .await
        .unwrap();

    let upstream = MockServer::start().await;
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.auth_method = "bearer".into();
    catalog.identity_propagation_mode = "both".into();
    catalog.forward_access_token = true;
    catalog.inject_delegation_token = true;
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&catalog)
    .await
    .unwrap();
    let root_path = format!("/api/v1/skills/{}/versions/1.0/download", root.skill_id);
    let dependency_id = &root.dependencies[0].skill_id;
    let dependency_path = format!("/api/v1/skills/{dependency_id}/versions/2.0/download");
    for (path_name, body) in [
        (root_path, root_contents),
        (dependency_path, dependency_contents),
    ] {
        Mock::given(method("GET"))
            .and(path(path_name))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&upstream)
            .await;
    }

    let mode = guest::select_mode(db, &bot_id).await.unwrap();
    let GuestAdmission::AddOn(guest_turn) = guest::classify(
        db,
        mode,
        VerifiedGuestInput {
            bot_id: &bot_id,
            binding_id: &binding_id,
            platform: "telegram",
            conversation_id: "private-chat",
            source_event_id: &event_id,
            sender_id: Some("verified-sender"),
            audience: AnswerAudience::Private,
            meta_grant_id: None,
        },
    )
    .await
    .unwrap() else {
        panic!("expected enrolled guest")
    };
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        conversation_id: None,
        text: "Read the guide".into(),
        agent_id: None,
        model: None,
        access_mode: None,
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
    });
    start.guest = true;
    start.origin = TurnOrigin::Channel;
    start.channel_event_id = Some(event_id.clone());
    start.new_id = Some(guest_turn.hosted_conversation_id());
    start.agent_id = Some(agent_id.clone());
    let origin = ChannelOrigin {
        org_execution: None,
        native_event_id: None,
        thread: None,
        nyxbot_channel_id: binding_id.clone(),
        partition: "private-chat".into(),
        platform: "telegram".into(),
    };
    let channel = db
        .collection::<crate::models::nyxbot_channel::NyxbotChannel>(
            crate::models::nyxbot_channel::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &binding_id})
        .await
        .unwrap()
        .unwrap();
    claim_source_event(db, &event_id, &channel, &origin, "verified-sender").await;
    start.channel = Some(origin);
    start.channel_context = Some(ChannelTurnContext {
        channel_guest: Some(guest_turn),
        meta_grant: None,
        text_completeness: None,
        org_origin: None,
        display_name: None,
        conversation_label: None,
        followed: false,
        source_event_id: event_id,
        channel_id: binding_id,
        channel_bot_id: bot_id.clone(),
        conversation_kind: "private".into(),
        sender_kind: ThreadSenderKind::Human,
        guest: true,
        original_message: true,
    });
    let row = engine::begin_turn(db, &f.owner, start, &state.encryption_keys)
        .await
        .unwrap_or_else(|error| panic!("Personal guest turn admission failed: {error}"));
    let chat = acks::chat_for_conversation(db, &row)
        .await
        .unwrap()
        .unwrap();
    let agent = team::agent_for_conversation(db, &row).await.unwrap();
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        db,
        &state.encryption_keys,
        &f.owner,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let router = router.with_state(state.clone());
    let tools = mcp(
        &router,
        credential.raw_key.as_str(),
        "tools/list",
        json!({}),
    )
    .await;
    let names = tools["result"]["tools"]
        .as_array()
        .expect("MCP tool listing");
    assert!(names.iter().any(|tool| tool["name"] == GUEST_TOOL_NAME));
    assert!(
        !names
            .iter()
            .any(|tool| tool["name"] == "nyxid__set_agent_skills")
    );
    let package =
        json!({"skill":&root.skill_id,"dependency":dependency_id,"path":"references/help.md"});
    let read = mcp(
        &router,
        credential.raw_key.as_str(),
        "tools/call",
        json!({"name":GUEST_TOOL_NAME,"arguments":&package}),
    )
    .await;
    assert_eq!(read["result"]["isError"], false, "{read}");
    let page: Value = serde_json::from_str(
        read["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP read result"),
    )
    .unwrap();
    assert_eq!(page["content"], "dependency supporting file");
    let preload = preload_for_turn(state, &row, &agent).await.unwrap();
    assert!(preload.contains("public pinned guide"), "{preload}");

    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        for forbidden in [
            "authorization",
            "cookie",
            "x-nyxid-identity-token",
            "x-nyxid-delegation-token",
        ] {
            assert!(!request.headers.contains_key(forbidden), "{forbidden}");
        }
        let token = request.headers[guest_skills::CAPABILITY_HEADER]
            .to_str()
            .unwrap();
        let value = guest_skills::lookup(state, token).await.unwrap();
        let projection = serde_json::to_value(value).unwrap();
        assert_eq!(projection["bot_id"], bot_id);
        assert_eq!(projection["audience"], "private");
        assert!(projection["organization_id"].is_null());
    }
    let original_token = requests[0].headers[guest_skills::CAPABILITY_HEADER]
        .to_str()
        .unwrap();
    db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set":{"updated_at":bson::DateTime::from_millis(catalog.updated_at.timestamp_millis() + 1)}},
        )
        .await
        .unwrap();
    assert!(guest_skills::lookup(state, original_token).await.is_err());

    db.collection::<Document>(crate::models::notification_channel::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&f.owner},
            doc! {"$set":{"approval_required":true}},
        )
        .await
        .unwrap();
    assert!(guest_tool(state, &chat, &package).await.is_err());
    assert!(
        !preload_for_turn(state, &row, &agent)
            .await
            .unwrap()
            .contains("public pinned guide")
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);

    let update: guest_skills::Update = serde_json::from_value(json!({
        "expected_revision":0,"skills_revision":1,
        "attachments":[{"skill_id":&root.skill_id,"audiences":["private"],"all_requests_approved":true}],
    })).unwrap();
    let actor = crate::services::audit_service::AuditActor::from_auth_user(&f.auth);
    guest_skills::set(state, &actor, &bot_id, update)
        .await
        .unwrap();
    assert_eq!(
        guest_tool(state, &chat, &package).await.unwrap()["content"],
        "dependency supporting file"
    );
    assert!(
        preload_for_turn(state, &row, &agent)
            .await
            .unwrap()
            .contains("public pinned guide")
    );
    assert!(
        guest_tool(state, &chat, &json!({"skill":id()}))
            .await
            .is_err()
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), 4);

    let latest = upstream.received_requests().await.unwrap();
    let last = latest.last().unwrap();
    let token = last.headers[guest_skills::CAPABILITY_HEADER]
        .to_str()
        .unwrap();
    crate::services::approval_service::set_service_approval_config(
        db,
        &f.owner,
        &catalog.id,
        &catalog.name,
        Some(true),
        None,
        None,
        Some(&crate::models::service_approval_config::ApprovalEffect::Deny),
    )
    .await
    .unwrap();
    assert!(guest_tool(state, &chat, &package).await.is_err());
    assert!(
        !preload_for_turn(state, &row, &agent)
            .await
            .unwrap()
            .contains("public pinned guide")
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), 4);

    crate::services::approval_service::delete_service_approval_config(
        db,
        &f.owner,
        &catalog.id,
        crate::services::approval_service::ConfigAudit {
            actor,
            user_service_id: None,
        },
        false,
    )
    .await
    .unwrap();
    agents
        .update_one(
            doc! {"_id":agent_id},
            doc! {"$set":{"skills_revision":2_i64}},
        )
        .await
        .unwrap();
    assert!(guest_tool(state, &chat, &package).await.is_err());
    assert!(guest_tool_definition(state, &chat).await.is_some());
    assert!(guest_skills::lookup(state, token).await.is_err());
    assert_eq!(upstream.received_requests().await.unwrap().len(), 4);

    agents
        .update_one(
            doc! {"_id":agent_id},
            doc! {"$set":{"skills":[],"skills_revision":3_i64}},
        )
        .await
        .unwrap();
    assert!(guest_skills::lookup(state, token).await.is_err());
    assert!(guest_tool_definition(state, &chat).await.is_none());
    assert!(guest_tool(state, &chat, &package).await.is_err());
    assert!(
        !preload_for_turn(state, &row, &agent)
            .await
            .unwrap()
            .contains("public pinned guide")
    );
    assert_eq!(upstream.received_requests().await.unwrap().len(), 4);
    db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_guest_skill_org_capability_uses_live_payer_and_maintainer() {
    use crate::billing_integration_tests::{FakeLago, billing_service, create_usage_index};
    use crate::services::org_channel_service as org;
    use std::sync::Arc;
    let mut f = org::tests::Fixture::new("channel_guest_org_skill").await;
    create_usage_index(&f.org.state.db).await;
    f.org.state.billing = Arc::new(billing_service(
        &f.org.state.db,
        Arc::new(FakeLago::default()),
        1000,
    ));
    f.wallet().await;
    f.guests(vec![]).await;
    let state = &f.org.state;
    let db = &state.db;
    activate_guests(db, &f.org.admin).await;
    let origin = f.guest("channel-visitor").await;
    let agent_id = &f.agent.id;
    let binding_id = &origin.nyxbot_channel_id;
    let binding = db
        .collection::<crate::models::nyxbot_channel::NyxbotChannel>(
            crate::models::nyxbot_channel::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":binding_id})
        .await
        .unwrap()
        .unwrap();
    let bot_id = &binding.channel_bot_id;
    db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": bot_id},
            doc! {"$set":{
                "guest_access_v1": true, "guest_access_v1_revision": 1_i64,
            }},
        )
        .await
        .unwrap();
    let event_id = id();
    let root_contents = archive("org bot private guide", "reference");
    let root = SkillReference {
        source: "ornn".into(),
        skill_id: id(),
        name: "org-private-guide".into(),
        version: "1.0".into(),
        sha256: hex::encode(Sha256::digest(&root_contents)),
        disabled: false,
        dependencies: vec![],
    };
    db.collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":agent_id},
            doc! {"$set":{"skills":bson::to_bson(&vec![&root]).unwrap(),"skills_revision":1_i64}},
        )
        .await
        .unwrap();

    let upstream = MockServer::start().await;
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&catalog)
    .await
    .unwrap();
    let root_path = format!("/api/v1/skills/{}/versions/1.0/download", root.skill_id);
    let verified = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let verified_on_server = verified.clone();
    let runtime = tokio::runtime::Handle::current();
    let callback_state = state.clone();
    let expected_org = f.org.org.clone();
    let expected_package = root.skill_id.clone();
    Mock::given(method("GET"))
        .and(path(root_path))
        .respond_with(move |request: &wiremock::Request| {
            let capability = request
                .headers
                .get(guest_skills::CAPABILITY_HEADER)
                .and_then(|token| token.to_str().ok())
                .unwrap()
                .to_owned();
            let state = callback_state.clone();
            let matches = futures::executor::block_on(runtime.spawn(async move {
                let result = guest_skills::lookup(&state, &capability).await.unwrap();
                serde_json::to_value(result).unwrap()
            }))
            .unwrap();
            assert_eq!(matches["organization_id"], expected_org);
            assert_eq!(matches["skill_id"], expected_package);
            verified_on_server.store(true, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_bytes(root_contents.clone())
        })
        .mount(&upstream)
        .await;

    let GuestAdmission::AddOn(turn) = guest::classify(
        db,
        guest::select_mode(db, bot_id).await.unwrap(),
        VerifiedGuestInput {
            bot_id,
            binding_id,
            platform: "lark",
            conversation_id: &origin.partition,
            source_event_id: &event_id,
            sender_id: Some("channel-visitor"),
            audience: AnswerAudience::Private,
            meta_grant_id: None,
        },
    )
    .await
    .unwrap() else {
        panic!("expected enrolled organization guest")
    };
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        conversation_id: None,
        text: "Read the guide".into(),
        agent_id: Some(agent_id.clone()),
        model: None,
        access_mode: None,
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
    });
    start.guest = true;
    start.origin = TurnOrigin::Channel;
    start.channel_event_id = Some(event_id.clone());
    start.new_id = Some(turn.hosted_conversation_id());
    claim_source_event(db, &event_id, &binding, &origin, "channel-visitor").await;
    start.channel = Some(origin.clone());
    start.org_access = Some(
        org::validate(db, &origin, &f.org.org, agent_id)
            .await
            .unwrap(),
    );
    start.channel_context = Some(ChannelTurnContext {
        channel_guest: Some(turn),
        meta_grant: None,
        text_completeness: None,
        org_origin: Some(Box::new(origin.clone())),
        display_name: None,
        conversation_label: None,
        followed: false,
        source_event_id: event_id,
        channel_id: binding_id.clone(),
        channel_bot_id: bot_id.clone(),
        conversation_kind: "private".into(),
        sender_kind: ThreadSenderKind::Human,
        guest: true,
        original_message: true,
    });
    let row = Box::pin(engine::begin_turn(
        db,
        &f.org.org,
        start,
        &state.encryption_keys,
    ))
    .await
    .unwrap_or_else(|error| panic!("Organization guest turn admission failed: {error}"));
    let chat = acks::chat_for_conversation(db, &row)
        .await
        .unwrap()
        .unwrap();
    let payer = chat
        .org_agent_access
        .as_ref()
        .and_then(|access| access.channel())
        .and_then(|channel| channel.guest_payer())
        .unwrap();
    assert_eq!(payer.org_id(), f.org.org);
    let package = json!({"skill":root.skill_id});
    assert_eq!(
        Box::pin(guest_tool(state, &chat, &package))
            .await
            .unwrap_or_else(|error| panic!("Organization guest skill read failed: {error}"))["content"],
        "org bot private guide"
    );
    assert!(verified.load(std::sync::atomic::Ordering::SeqCst));
    let metered = db
        .collection::<crate::models::usage_meter::UsageMeterRow>(
            crate::models::usage_meter::COLLECTION_NAME,
        )
        .find_one(doc! {"service_slug": "ornn-api", "api_key_id": &row.credential_api_key_id})
        .await
        .unwrap()
        .expect("metered skill read");
    assert_eq!(metered.billing_owner_id, f.org.org);
    assert!(metered.org_channel_guest.is_some());

    let requests = upstream.received_requests().await.unwrap();
    let capability = requests[0].headers[guest_skills::CAPABILITY_HEADER]
        .to_str()
        .unwrap();
    f.org.revoke(&f.org.admin).await;
    assert!(guest_skills::lookup(state, capability).await.is_err());
    assert!(Box::pin(guest_tool(state, &chat, &package)).await.is_err());
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    db.drop().await.unwrap();
}
