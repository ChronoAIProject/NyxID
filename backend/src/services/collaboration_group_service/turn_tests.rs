//! Run the detached worker, then authenticate NyxAgent's actual HTTP callbacks.
use super::*;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

async fn connection(
    f: &Fixture,
    owner: &str,
    slug: &str,
    url: &str,
    catalog: &str,
    platform: bool,
) -> String {
    let id = f.service(owner, slug).await;
    f.state.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":&id},doc! {"$set":{"catalog_service_id":catalog,"identity_propagation_mode":"none","credential_binding":if platform {Some("platform")}else{None}}}).await.unwrap();
    let service = f
        .state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_endpoints")
        .update_one(doc! {"_id":service.endpoint_id}, doc! {"$set":{"url":url}})
        .await
        .unwrap();
    id
}

async fn shared_group_turn_e2e(collaboration: bool, upstream_failure: bool) {
    let f = Fixture::new("shared_group_turn_e2e").await;
    enable(&f).await;
    crate::test_utils::init_test_audit_chain_hmac_key();
    let agent = specialist(&f, &f.admin, "shared-callback-agent").await;
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"id":"model-result","choices":[],"ok":true})),
        )
        .mount(&provider)
        .await;
    let mut model = crate::models::downstream_service::test_helpers::dummy_service();
    model.id = Uuid::new_v4().to_string();
    model.slug = "recipient-model".into();
    model.base_url = provider.uri();
    model.service_category = "connection".into();
    model.auth_method = "bearer".into();
    model.requires_user_credential = false;
    model.identity_propagation_mode = "none".into();
    model.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(b"test-master")
        .await
        .unwrap();
    model.platform_key =
        Some(serde_json::from_value(json!({"enabled":true,"audience":"public"})).unwrap());
    model.inference = Some(
        serde_json::from_value(json!({"wire_protocol":"openai_completions","model_list":true}))
            .unwrap(),
    );
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&model)
        .await
        .unwrap();
    connection(&f, &f.member, &model.slug, &provider.uri(), &model.id, true).await;
    let byok = connection(
        &f,
        &f.member,
        "recipient-byok",
        &provider.uri(),
        &model.id,
        false,
    )
    .await;
    let saved_key = svc::user_api_key_service::create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        svc::user_api_key_service::CreateApiKeyParams {
            label: "recipient model",
            credential_type: "bearer",
            credential: "test-recipient-key",
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
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":byok},
            doc! {"$set":{"api_key_id":saved_key.id,"auth_method":"bearer"}},
        )
        .await
        .unwrap();
    let mut service = model.clone();
    service.id = Uuid::new_v4().to_string();
    service.slug = "recipient-noauth".into();
    service.auth_method = "none".into();
    service.credential_encrypted.clear();
    service.platform_key = None;
    service.inference = None;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&service)
        .await
        .unwrap();
    let owner_connection = connection(
        &f,
        &f.admin,
        "owner-noauth",
        &provider.uri(),
        &service.id,
        false,
    )
    .await;
    connection(
        &f,
        &f.member,
        &service.slug,
        &provider.uri(),
        &service.id,
        false,
    )
    .await;
    f.state
        .db
        .collection::<bson::Document>("assistant_agents")
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"grants.service_ids":[&owner_connection]}},
        )
        .await
        .unwrap();
    svc::shared_agent_service::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        svc::shared_agent_service::SharingUpdate {
            expected_version: 0,
            enabled: true,
            audience: vec![svc::shared_agent_service::Audience {
                subject_kind: ShareSubject::Person,
                subject_id: format!("email:{}@example.com", f.member),
                role: ShareRole::User,
            }],
            shared_access: BTreeMap::from([(
                owner_connection,
                crate::models::assistant_agent::GuestAccess::All,
            )]),
        },
        true,
    )
    .await
    .unwrap();
    // Test human creation and message handlers; callback requests use the full
    // production router and authenticate with the key sent to mock NyxAgent.
    let (_, Json(group)) = crate::handlers::assistant_group::create_group(
        State(f.state.clone()),crate::test_utils::test_auth_user(&f.member),
        Json(serde_json::from_value(json!({"name":"Production crew","collaboration":collaboration,"member_agent_ids":[agent.id]})).unwrap()),
    ).await.unwrap();
    if collaboration {
        let stored = svc::collaboration_group_service::get(&f.state.db, &f.member, &group.id)
            .await
            .unwrap();
        join(&f, &stored, &f.viewer).await;
    }
    let captures = Arc::new(Mutex::new(Vec::<(String, u16, String)>::new()));
    let (callback_state, callback_captures) = (f.state.clone(), captures.clone());
    let upstream = Router::new()
        .route("/v1/models",get(|| async {Json(json!({"data":[]}))}))
        .route("/v1/responses",post(move |headers:HeaderMap| {
            let (state,captures)=(callback_state.clone(),callback_captures.clone());
            async move {
                tokio::time::sleep(Duration::from_millis(350)).await;
                let bearer=headers.get("authorization").unwrap().to_str().unwrap();
                let (_,router)=crate::routes::build_router_with_state(state.clone());
                let router=router.with_state(state);
                let mut failed=upstream_failure;
                for (slug,method,path,body,must_succeed) in [
                    ("recipient-model","POST","chat/completions",json!({"model":"test-model","messages":[{"role":"user","content":"hi"}],"stream":false,"store":false}),true),
                    ("recipient-noauth","GET","status",Value::Null,false),
                    ("recipient-byok","POST","chat/completions",json!({"model":"test-model","messages":[{"role":"user","content":"hi"}],"stream":false,"store":false}),false),
                    ("recipient-model","POST","chat/completions",json!({"model":"test-model","messages":[{"role":"user","content":"hi"}],"stream":false,"store":true}),false),
                ] {
                    let response=router.clone().oneshot(Request::builder().uri(format!("/api/v1/proxy/s/{slug}/{path}")).method(method).header("authorization",bearer).header("content-type","application/json").body(if body.is_null(){Body::empty()}else{Body::from(body.to_string())}).unwrap()).await.unwrap();
                    let status=response.status();
                    let bytes=to_bytes(response.into_body(),1024*1024).await.unwrap();
                    let text=String::from_utf8_lossy(&bytes).to_string();
                    let consent=text.contains("shared_connection_consent_required");
                    failed |= !status.is_success() && (must_succeed || !consent);
                    captures.lock().await.push((slug.into(),status.as_u16(),text));
                }
                let frame=json!({"type":if failed{"response.failed"}else{"response.completed"},"sequence_number":0,"response":{"id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef","conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":if failed{"failed"}else{"completed"},"error":if failed{json!({"code":"upstream_error","message":"private callback detail"})}else{Value::Null},"output":if failed{json!([])}else{json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Shared reply"}]}])}}});
                ([("content-type","text/event-stream")],format!("data: {frame}\n\n")).into_response()
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut nyx = service.clone();
    nyx.id = Uuid::new_v4().to_string();
    nyx.slug = svc::assistant_nyxagent::SERVICE_SLUG.into();
    nyx.base_url = format!("http://{address}");
    nyx.service_category = "internal".into();
    nyx.forward_access_token = true;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(nyx)
        .await
        .unwrap();
    let posted = crate::handlers::assistant_group::post_message(
        State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.member),
        Path(group.id.clone()),
        Json(
            serde_json::from_value(json!({"text":"hi","recipient_agent_ids":[agent.id]})).unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(posted.0, StatusCode::ACCEPTED);
    let reply = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(message) = f
                .state
                .db
                .collection::<crate::models::assistant_message::AssistantMessage>(
                    "assistant_messages",
                )
                .find_one(doc! {"user_id":&f.member,"role":"assistant"})
                .sort(doc! {"created_at":-1})
                .await
                .unwrap()
            {
                break message;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("group worker must settle");
    server.abort();
    let captured = captures.lock().await.clone();
    assert_eq!(
        reply.status,
        if upstream_failure {
            "failed"
        } else {
            "completed"
        },
        "turn failure: {:?}; callbacks: {captured:?}",
        serde_json::to_value(&reply.error_detail).unwrap()
    );
    assert_eq!(
        captured.len(),
        4,
        "mock NyxAgent must exercise model, service and consent boundaries"
    );
    assert_eq!(captured[0].1, 200, "model callback: {captured:?}");
    assert_eq!(
        captured[1].1, 409,
        "a service operation still needs explicit consent"
    );
    for captured in &captured[2..] {
        assert_eq!(
            captured.1, 409,
            "BYOK and stateful calls still require consent: {captured:?}"
        );
        let card: Value = serde_json::from_str(&captured.2).unwrap();
        assert_eq!(card["error"], "shared_connection_consent_required");
    }
    let card: Value = serde_json::from_str(&captured[1].2).unwrap();
    assert_eq!(card["error"], "shared_connection_consent_required");
    assert_eq!(
        provider.received_requests().await.unwrap().len(),
        1,
        "only the model has consent-free platform admission"
    );
    let public=tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            if let Some(message)=f.state.db.collection::<bson::Document>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME).find_one(doc! {"group_id":&group.id,"role":if upstream_failure {"notice"}else{"agent"},"text":{"$regex":if upstream_failure {"could not reply"}else{"Shared reply"}}}).await.unwrap(){break message;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("every settlement must reach the public transcript");
    if upstream_failure {
        assert!(
            public
                .get_str("text")
                .unwrap()
                .contains("assistant_unavailable")
        );
    }
    assert!(!public.to_string().contains("private callback detail"));
    assert!(!public.to_string().contains("consent_id"));
    assert!(
        svc::assistant_nyxagent::get(&f.state.db, &f.admin, &reply.conversation_id)
            .await
            .is_err(),
        "agent owner never gets the recipient thread"
    );
    if collaboration {
        assert!(
            svc::assistant_nyxagent::get(&f.state.db, &f.member, &reply.conversation_id)
                .await
                .is_err()
        );
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let events: Vec<bson::Document> = f
                .state
                .db
                .collection(crate::models::audit_log::COLLECTION_NAME)
                .find(doc! {"event_type":"service_request","event_data.execution_mode":"shared"})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            if events.len() >= 4 {
                for event in &events {
                    let data = event.get_document("event_data").unwrap();
                    assert_eq!(data.get_str("actor_user_id").unwrap(), f.member);
                    assert_eq!(data.get_str("owner_user_id").unwrap(), f.member);
                    assert_eq!(data.get_str("agent_owner_id").unwrap(), f.admin);
                }
                assert!(
                    events
                        .iter()
                        .any(|e| e.get_document("event_data").unwrap().get_str("outcome")
                            == Ok("response_received"))
                );
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| e.get_document("event_data").unwrap().get_str("outcome")
                            == Ok("denied"))
                        .count(),
                    3
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("callback audits must retain recipient attribution and explicit consent denials");
    let saved = f
        .state
        .db
        .collection::<bson::Document>("assistant_agents")
        .find_one(doc! {"_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        saved
            .get("owner_model_route")
            .is_none_or(|value| *value == bson::Bson::Null)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_group_turn_e2e_collaboration_callbacks() {
    shared_group_turn_e2e(true, false).await;
}

#[tokio::test]
async fn shared_group_turn_e2e_personal_crew_callbacks() {
    shared_group_turn_e2e(false, false).await;
}

#[tokio::test]
async fn shared_group_turn_e2e_collaboration_failure_notice() {
    shared_group_turn_e2e(true, true).await;
}

#[tokio::test]
async fn shared_group_turn_e2e_crew_failure_notice() {
    shared_group_turn_e2e(false, true).await;
}

#[tokio::test]
async fn collaboration_stop_poll_checks_exact_turn_and_live_authority() {
    let (f, agent, group) = setup("collaboration_stop_poll").await;
    share(&f, &agent, &[&f.member]).await;
    let group = join(&f, &group, &f.member).await;
    let claim = runtime_start(&f, &f.member, &group, &agent).await;
    let row = &claim.conversation;
    let turn = &row.active_turn.as_ref().unwrap().turn_id;
    assert!(
        !runtime::stop_requested(&f.state.db, row, turn)
            .await
            .unwrap()
    );
    assert!(
        runtime::stop_requested(&f.state.db, row, "different-turn")
            .await
            .unwrap()
    );
    // The cheap poll used between full authority checks agrees.
    assert!(
        !runtime::stop_flag_set(&f.state.db, row, turn)
            .await
            .unwrap()
    );
    assert!(
        runtime::stop_flag_set(&f.state.db, row, "different-turn")
            .await
            .unwrap()
    );
    let key = runtime_key(&f, row).await;
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    let chat = auth.assistant_chat.unwrap();
    svc::assistant_nyxagent::conversation_history(&f.state.db, &chat, None, 10)
        .await
        .unwrap();
    assert!(
        svc::assistant_nyxagent::get(&f.state.db, &f.member, &row.id)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        runtime::stop_flag_set(&f.state.db, row, turn)
            .await
            .unwrap()
    );
    assert!(
        runtime::stop_requested(&f.state.db, row, turn)
            .await
            .unwrap()
    );
    assert!(
        svc::assistant_nyxagent::conversation_history(&f.state.db, &chat, None, 10)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<bson::Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn.stop_requested":false}},
        )
        .await
        .unwrap();
    let policy = svc::shared_agent_service::policy(&f.state.db, &agent)
        .await
        .unwrap();
    svc::shared_agent_service::update(
        &f.state.db,
        &f.admin,
        &agent.id,
        svc::shared_agent_service::SharingUpdate {
            expected_version: policy.version,
            enabled: false,
            audience: vec![],
            shared_access: BTreeMap::new(),
        },
        true,
    )
    .await
    .unwrap();
    assert!(
        runtime::stop_requested(&f.state.db, row, turn)
            .await
            .unwrap()
    );
    assert!(
        svc::assistant_nyxagent::conversation_history(&f.state.db, &chat, None, 10)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_failed_settlement_publishes_one_safe_notice() {
    let (f, agent, group) = setup("collaboration_failure_notice").await;
    let claim = runtime_start(&f, &f.admin, &group, &agent).await;
    let row = &claim.conversation;
    let message = Uuid::new_v4().to_string();
    let result = svc::assistant_nyxagent::TurnResult {
        text: "PRIVATE unfinished draft and account details".into(),
        session_id: None,
        response_id: None,
        error: Some(svc::assistant_nyxagent::TurnError::internal(
            "assistant_unavailable",
            "stream",
        )),
    };
    for _ in 0..2 {
        svc::assistant_nyxagent::finish_turn(
            &f.state.db,
            row,
            &row.credential_api_key_id,
            &message,
            &result,
        )
        .await
        .unwrap();
    }
    let history = messages(&f.state.db, &f.admin, &group.id, None)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].role, "notice");
    assert_eq!(
        history[1].text,
        format!("{} could not reply (assistant_unavailable)", agent.name)
    );
    assert!(history[1].activities.is_empty());
    assert!(history[1].attachments.is_empty());
    f.state.db.drop().await.unwrap();
}
