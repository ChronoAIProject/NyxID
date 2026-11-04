//! HTTP regressions for the dedicated Voice service boundary.
use super::super::{binding, execution};
use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

async fn proxy_child(
    name: &str,
) -> (
    Fixture,
    crate::models::downstream_service::DownstreamService,
    String,
    String,
    String,
) {
    let (f, voice, service, raw) = setup(name).await;
    let parent = auth(&f, &raw).await.api_key_id.unwrap();
    f.state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(doc! {"_id":&parent}, doc! {"$set":{"scopes":"proxy"}})
        .await
        .unwrap();
    let (_, secret) = child(&f, &parent).await;
    (f, voice, service, raw, secret)
}

async fn request(
    app: &Router,
    key: &str,
    method: &str,
    uri: &str,
    body: Body,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"body":String::from_utf8_lossy(&bytes)}));
    (status, value)
}

async fn set_operation_inputs(
    f: &Fixture,
    raw: &str,
    service: &str,
    method: &str,
    path: &str,
    inputs: Value,
) {
    let key = auth(f, raw).await.api_key_id.unwrap();
    let scope: crate::models::agent_operation_scope::AgentOperationScope =
        serde_json::from_value(json!({
            "revision":1,
            "operations":[{"rule":{"method":method,"path_template":path},"inputs":inputs}],
        }))
        .unwrap();
    let scopes = std::collections::BTreeMap::from([(service.to_owned(), scope)]);
    f.state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(
            doc! {"_id":key},
            doc! {"$set":{"assistant_operation_scopes":bson::to_bson(&scopes).unwrap()}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn voice_service_ingress_child_discovers_only_its_connection_without_mcp_operations() {
    Box::pin(async {
        let (f, _, service, _, secret) = proxy_child("voice_service_inventory").await;
        let a = auth(&f, &secret).await;
        let identity = execution::resolve(&f.state.db, &a, &service).await.unwrap();
        let other_thread =
            crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.nyxbot_thread)
                .await
                .unwrap();
        let other_agent = crate::services::assistant_team_service::agent_for_conversation(
            &f.state.db,
            &other_thread,
        )
        .await
        .unwrap();
        let other = binding::publish(
            &f.state.db,
            &f.owner,
            &other_agent.id,
            &f.state.config.base_url,
        )
        .await
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let (status, body) = request(&app, &secret, "GET", "/api/v1/keys", Body::empty()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let keys = body["keys"].as_array().unwrap();
        assert_eq!(
            keys.len(),
            1,
            "Inventory must apply the exact service grant: {body}"
        );
        assert_eq!(keys[0]["id"], service);
        assert_ne!(keys[0]["id"], other.id);
        assert_eq!(
            keys[0]["endpoint_url"],
            execution::service_url(&f.state.config.base_url, &service).unwrap()
        );
        let endpoint = f
            .state
            .db
            .collection::<crate::models::user_endpoint::UserEndpoint>(
                crate::models::user_endpoint::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&identity.service.endpoint_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(endpoint.url, keys[0]["endpoint_url"]);
        assert!(endpoint.openapi_spec_url.is_none());
        let catalog = crate::services::mcp_service::load_operation_catalog(
            &f.state.db,
            &f.state.node_ws_manager,
            &f.owner,
            crate::services::mcp_service::NodeScope::Unrestricted,
            crate::services::mcp_service::ServiceScope::Allowed(std::slice::from_ref(&service)),
        )
        .await
        .unwrap();
        assert!(
            catalog
                .services
                .iter()
                .chain(&catalog.unpublished_services)
                .all(|entry| entry.endpoints.is_empty()),
            "Voice must not publish ordinary MCP operations"
        );

        let prefix = format!("/api/v1/proxy/nyxbot-voice/{service}");
        let (status, body) = request(
            &app,
            &secret,
            "GET",
            &format!("{prefix}/voice/options"),
            Body::empty(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "The connection supplies its own Bot: {body}"
        );
        let (status, _) = request(
            &app,
            &secret,
            "GET",
            &format!("/api/v1/proxy/nyxbot-voice/{}/voice/options", other.id,),
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_service_ingress_session_cannot_be_read_or_controlled_by_another_child() {
    Box::pin(async {
        let (f, voice, service, raw, secret) = proxy_child("voice_service_child_session").await;
        let parent = auth(&f, &raw).await.api_key_id.unwrap();
        let (_, sibling) = child(&f, &parent).await;
        let a = auth(&f, &secret).await;
        let proof = keys::authorize(&f.state.db, &a, &f.row.id).await.unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let session_uri = format!(
            "/api/v1/proxy/nyxbot-voice/{service}/voice/sessions/{}",
            call.id
        );
        let (status, body) = request(&app, &secret, "GET", &session_uri, Body::empty()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], call.id);

        for key in [&sibling, &raw] {
            let (status, body) = request(&app, key, "GET", &session_uri, Body::empty()).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "A shared parent grant cannot adopt another credential's call: {body}"
            );
            let (status, body) = request(
                &app,
                key,
                "POST",
                &format!("{session_uri}/control"),
                Body::from(
                    json!({"command_id":Uuid::new_v4(),"expected_revision":0,"action":"end"})
                        .to_string(),
                ),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "Another child cannot stop this call: {body}"
            );
        }
        let stored = session::get(&f.state.db, &f.owner, &f.row.id, &call.id)
            .await
            .unwrap();
        assert_eq!(stored.control_revision, 0);
        assert!(!stored.end_requested);
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_service_ingress_rejects_unknown_paths_and_large_bodies_without_endpoint_egress() {
    Box::pin(async {
        let (f, _, service, _, secret) = proxy_child("voice_service_no_egress").await;
        let identity = execution::resolve(&f.state.db, &auth(&f, &secret).await, &service)
            .await
            .unwrap();
        let received = Arc::new(AtomicUsize::new(0));
        let requests = received.clone();
        let trap = Router::new().fallback(move || {
            let requests = requests.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                StatusCode::BAD_GATEWAY
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, trap).await.unwrap() });
        // A legacy/imported endpoint URL and forwarding flag cannot turn the
        // dedicated ingress into a generic proxy or send it the device Key.
        f.state
            .db
            .collection::<bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&identity.service.endpoint_id},
                doc! {"$set":{"url":format!("http://{address}")}},
            )
            .await
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&service},
                doc! {"$set":{"forward_access_token":true}},
            )
            .await
            .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let prefix = format!("/api/v1/proxy/nyxbot-voice/{service}");
        for uri in [
            &prefix,
            &format!("{prefix}/"),
            &format!("{prefix}/unrecognized"),
            &format!("{prefix}/voice/sessions/unknown/unsupported"),
        ] {
            let (status, body) = request(&app, &secret, "GET", uri, Body::empty()).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
        }
        let (status, body) = request(
            &app,
            &secret,
            "GET",
            &format!("/api/v1/proxy/nyxbot-voice/{}", Uuid::new_v4(),),
            Body::empty(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Unknown connections must still check scope: {body}"
        );
        let (status, body) = request(
            &app,
            &secret,
            "POST",
            &format!("{prefix}/voice/sessions"),
            Body::from(vec![b'x'; 128 * 1024 + 1]),
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        let (status, body) = request(
            &app,
            &secret,
            "GET",
            &format!("{prefix}/voice/options"),
            Body::empty(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "Known operations must stay local: {body}"
        );
        assert_eq!(
            received.load(Ordering::SeqCst),
            0,
            "No caller credential may reach a stored endpoint host"
        );
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("approval_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        server.abort();
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_service_ingress_checks_actual_optional_and_required_query_limits() {
    Box::pin(async {
        let (f, _, service, raw, secret) = proxy_child("voice_service_query_inputs").await;
        let before_scope = auth(&f, &secret).await;
        // Both conversations belong to this Bot. The input limit, rather than
        // the independent Bot boundary, must reject the second conversation.
        let mut other = f.row.clone();
        other.id = format!("nyxa-{}", Uuid::new_v4().simple());
        f.state
            .db
            .collection::<crate::models::assistant_conversation::AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .insert_one(&other)
            .await
            .unwrap();
        let identity = execution::resolve(&f.state.db, &auth(&f, &secret).await, &service)
            .await
            .unwrap();
        execution::conversation(&f.state.db, &identity, &other.id)
            .await
            .unwrap();
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let uri = format!("/api/v1/proxy/nyxbot-voice/{service}/voice/options");
        for required in [false, true] {
            set_operation_inputs(
                &f,
                &raw,
                &service,
                "GET",
                "/voice/options",
                json!({
                    "query":{"conversation_id":{"required":required,
                        "rule":{"type":"exact","value":f.row.id}}},
                }),
            )
            .await;
            if !required {
                let query = format!("conversation_id={}", other.id);
                let result = crate::handlers::assistant_voice::internal::dispatch(
                    &f.state,
                    &before_scope,
                    &service,
                    "GET",
                    "voice/options",
                    Some(&query),
                    &[],
                    axum::http::HeaderMap::new(),
                    None,
                )
                .await;
                assert!(
                    matches!(
                        result,
                        Err(crate::errors::AppError::ApiKeyScopeForbidden(_))
                    ),
                    "Ingress must check current input limits even with an older extracted identity"
                );
            }
            let (status, body) = request(
                &app,
                &secret,
                "GET",
                &format!("{uri}?conversation_id={}", f.row.id),
                Body::empty(),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "Compliant query must survive application rechecks: {body}"
            );
            let (status, body) = request(&app, &secret, "GET", &uri, Body::empty()).await;
            assert_eq!(
                status,
                if required {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::OK
                },
                "{body}"
            );
            for query in [
                format!("conversation_id={}", other.id),
                format!("conversation_id={}&undeclared=1", f.row.id),
            ] {
                let (status, body) = request(
                    &app,
                    &secret,
                    "GET",
                    &format!("{uri}?{query}"),
                    Body::empty(),
                )
                .await;
                assert_eq!(
                    status,
                    StatusCode::FORBIDDEN,
                    "Actual query values and undeclared parameters must be checked: {body}"
                );
            }
        }
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn voice_service_ingress_checks_actual_json_body_and_content_type_limits() {
    Box::pin(async {
        let (f, voice, service, raw, secret) = proxy_child("voice_service_body_inputs").await;
        let proof = keys::authorize(&f.state.db, &auth(&f, &secret).await, &f.row.id)
            .await
            .unwrap();
        let call = session::admit_for_client_key(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &Uuid::new_v4().to_string(),
            platform(&voice),
            "fixture".into(),
            "worker",
            proof,
        )
        .await
        .unwrap();
        // Closed-call control is an idempotent operation. It exercises the
        // real application recheck without creating a provider session.
        session::write(
            &f.state.db,
            &call,
            doc! {"state":"closed","live_slot":false},
        )
        .await
        .unwrap();
        set_operation_inputs(
            &f,
            &raw,
            &service,
            "POST",
            "/voice/sessions/{sid}/control",
            json!({
                "path":{"sid":{"type":"exact","value":call.id}},
                "body":{"type":"object","properties":{
                    "command_id":{"type":"string","max_length":36},
                    "expected_revision":{"type":"integer","minimum":0,"maximum":0},
                    "action":{"type":"exact","value":"mute"}},
                    "required":["command_id","expected_revision","action"]},
            }),
        )
        .await;
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let app = private.with_state(f.state.clone());
        let uri = format!(
            "/api/v1/proxy/nyxbot-voice/{service}/voice/sessions/{}/control",
            call.id
        );
        let compliant = json!({"command_id":Uuid::new_v4(),"expected_revision":0,"action":"mute"});
        let (status, body) = request(
            &app,
            &secret,
            "POST",
            &uri,
            Body::from(compliant.to_string()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "Compliant body must survive application rechecks: {body}"
        );
        assert_eq!(body["id"], call.id);
        let mut forbidden = compliant.clone();
        forbidden["action"] = json!("end");
        let (status, body) = request(
            &app,
            &secret,
            "POST",
            &uri,
            Body::from(forbidden.to_string()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Actual action must satisfy its saved limit: {body}"
        );
        let (status, body) = request(&app, &secret, "POST", &uri, Body::empty()).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "A required JSON body cannot be omitted: {body}"
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&uri)
                    .header("authorization", format!("Bearer {secret}"))
                    .header("content-type", "text/plain")
                    .body(Body::from(compliant.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "JSON value limits must enforce the actual Content-Type"
        );
        let stored = session::get(&f.state.db, &f.owner, &f.row.id, &call.id)
            .await
            .unwrap();
        assert_eq!(stored.control_revision, 0);
        assert!(!stored.end_requested);
        f.state.db.drop().await.unwrap();
    })
    .await;
}
