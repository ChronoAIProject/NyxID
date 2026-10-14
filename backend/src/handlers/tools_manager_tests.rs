//! Local MongoDB regression coverage for limited Tool authority.
use super::{catalog_authority as authority, endpoints, services};
use crate::{
    errors::AppError,
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        role::{COLLECTION_NAME as ROLES, Role},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{role_service, service_endpoint_service as operations},
    test_utils::{connect_test_database, test_app_state, test_auth_user, test_user},
};
use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use uuid::Uuid;

// Local-only E2Es (ignored in CI) are excluded from cargo-llvm-cov builds:
// they never execute there, and instrumenting them pushes the single backend
// test crate past the coverage runner's memory.
#[cfg(not(coverage))]
#[path = "catalog_manager_e2e_tests.rs"]
mod catalog_e2e;

async fn seed(db: &mongodb::Database, platform: &str, manager: bool, catalog: bool) -> String {
    role_service::seed_system_roles(db).await.unwrap();
    let id = Uuid::new_v4().to_string();
    let mut u = test_user(&id, UserType::Person);
    for slug in [
        Some(platform),
        manager.then_some("tools_manager"),
        catalog.then_some("catalog_manager"),
    ]
    .into_iter()
    .flatten()
    {
        let r = db
            .collection::<Role>(ROLES)
            .find_one(doc! {"slug":slug})
            .await
            .unwrap()
            .unwrap();
        u.role_ids.push(r.id);
    }
    db.collection::<User>(USERS).insert_one(u).await.unwrap();
    id
}
async fn create(
    state: &crate::AppState,
    id: &str,
    body: Value,
) -> crate::errors::AppResult<services::ServiceResponse> {
    services::create_service_request(
        State(state.clone()),
        test_auth_user(id),
        crate::telemetry::TelemetryContext::default(),
        Json(body),
    )
    .await
    .map(|Json(v)| v)
}
async fn update(
    state: &crate::AppState,
    id: &str,
    tool: &str,
    body: Value,
) -> crate::errors::AppResult<services::ServiceResponse> {
    services::update_service(
        State(state.clone()),
        test_auth_user(id),
        crate::telemetry::TelemetryContext::default(),
        Path(tool.into()),
        Json(serde_json::from_value(body).unwrap()),
    )
    .await
    .map(|Json(v)| v)
}
async fn change_test_publication(
    db: &mongodb::Database,
    service_id: &str,
    expected_kind: crate::models::downstream_service::OfferingKind,
    ids: &[String],
    publication: crate::models::service_endpoint::PublicationState,
    actor: &crate::services::audit_service::AuditActor,
    audit_key: &[u8],
) -> crate::errors::AppResult<Vec<crate::models::service_endpoint::ServiceEndpoint>> {
    let mut observed = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":service_id})
        .await?
        .unwrap();
    observed.offering_kind = expected_kind;
    crate::services::tool_publication_service::change_publication(
        db,
        &observed,
        &crate::services::catalog_mutation_service::Authority {
            catalog_base_url: "http://localhost:3001".into(),
            actor: actor.clone(),
            first_party_human: true,
            admin_required: false,
        },
        ids,
        publication,
        audit_key,
    )
    .await
}

fn new(slug: &str) -> Value {
    json!({"name":slug,"slug":slug,"base_url":"https://vendor.example/api","auth_method":"bearer","offering_kind":"tool","service_category":"internal"})
}

#[tokio::test]
async fn tools_manager_service_ingress_unknown_fields_name_only_and_type_errors_redact() {
    use super::curation_tests::request;
    let db = connect_test_database("service_request_diagnostics")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let service = create(&state, &admin, json!({"name":"Diagnostics", "slug":"diagnostics", "base_url":"https://api.example.test", "auth_method":"none"})).await.unwrap();
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&admin).unwrap(),
        "openid profile email admin write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    for (method, route, body, field) in [
        (
            "POST",
            "/api/v1/services".to_string(),
            json!({"name":"Bad", "unknown_option":"REQUEST_VALUE_CANARY"}),
            Some("unknown_option"),
        ),
        (
            "PUT",
            format!("/api/v1/services/{}", service.id),
            json!({"unknown_option":"REQUEST_VALUE_CANARY"}),
            Some("unknown_option"),
        ),
        (
            "POST",
            "/api/v1/services".to_string(),
            json!({"name":"Bad", "offering_kind":"REQUEST_VALUE_CANARY"}),
            None,
        ),
        (
            "PUT",
            format!("/api/v1/services/{}", service.id),
            json!({"offering_kind":"REQUEST_VALUE_CANARY"}),
            None,
        ),
        (
            "PUT",
            format!("/api/v1/services/{}", service.id),
            json!({"is_active":"REQUEST_VALUE_CANARY"}),
            None,
        ),
    ] {
        let (status, result) = request(&state, method, &route, &token, Some(body)).await;
        assert_eq!(status, 400);
        let output = result.to_string();
        assert!(!output.contains("REQUEST_VALUE_CANARY"));
        if let Some(field) = field {
            assert!(output.contains(field));
        } else {
            assert!(output.contains("Invalid service request fields"));
        }
    }
    assert_eq!(
        db.collection::<DownstreamService>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_all_tool_creation_fences_live_roles_but_receipt_replay_is_durable() {
    use crate::services::catalog_skill_service::{self, SkillUpdate};
    let db = connect_test_database("tool_create_live_authority")
        .await
        .unwrap();
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", true, false).await;
    for (actor, marked, role_slug) in [(&admin, false, "admin"), (&manager, true, "tools_manager")]
    {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = format!("create-fence-{}", service.id);
        service.created_by = actor.clone();
        service.offering_kind = crate::models::downstream_service::OfferingKind::Tool;
        service.auth_method = "none".into();
        service.service_category = "internal".into();
        service.requires_user_credential = false;
        service.tools_manager_created = marked;
        let request_id = Uuid::new_v4().to_string();
        let input = SkillUpdate::default();
        let committed = catalog_skill_service::create_with_endpoints(
            &db,
            &service,
            actor,
            &input,
            &request_id,
            "stable-local-intent",
            &[],
        )
        .await
        .unwrap();
        let role = db
            .collection::<Role>(ROLES)
            .find_one(doc! {"slug":role_slug})
            .await
            .unwrap()
            .unwrap();
        db.collection::<Document>(USERS)
            .update_one(doc! {"_id":actor}, doc! {"$pull":{"role_ids":role.id}})
            .await
            .unwrap();
        let replay = catalog_skill_service::create_with_endpoints(
            &db,
            &service,
            actor,
            &input,
            &request_id,
            "stable-local-intent",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(
            replay.id, committed.id,
            "receipt replay performs no new write"
        );
        service.id = Uuid::new_v4().to_string();
        service.slug = format!("new-create-fence-{}", service.id);
        assert!(
            matches!(
                catalog_skill_service::create_with_endpoints(
                    &db,
                    &service,
                    actor,
                    &input,
                    &Uuid::new_v4().to_string(),
                    "new-local-intent",
                    &[]
                )
                .await,
                Err(AppError::Forbidden(_))
            ),
            "unmarked Admin and marked Manager creations both fence live authority"
        );
        assert!(
            db.collection::<Document>(SERVICES)
                .find_one(doc! {"_id":&service.id})
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        db.collection::<Document>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_catalog_provider_and_service_writes_require_human_identity_matrix() {
    use crate::mw::auth::AuthMethod;
    use axum::response::IntoResponse;
    let db = connect_test_database("catalog_mutation_identity_matrix")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", false, true).await;
    let service = create(&state, &admin, json!({"name":"Legacy", "slug":"legacy-identity", "base_url":"https://api.example.test", "auth_method":"none"})).await.unwrap();
    let provider = super::providers::create_provider(State(state.clone()), test_auth_user(&admin),
        Json(serde_json::from_value(json!({"name":"Local provider", "slug":"local-identity", "provider_type":"api_key"})).unwrap())).await.unwrap().0;
    let before_service = db
        .collection::<Document>(SERVICES)
        .find_one(doc! {"_id":&service.id})
        .await
        .unwrap()
        .unwrap();
    let before_provider = db
        .collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
        .find_one(doc! {"_id":&provider.id})
        .await
        .unwrap()
        .unwrap();
    for actor in [&admin, &manager] {
        for kind in [
            "api_key",
            "agent_key_login",
            "delegated",
            "service_account",
            "oauth_app",
        ] {
            let mut auth = test_auth_user(actor);
            match kind {
                "api_key" | "agent_key_login" => {
                    auth.auth_method = AuthMethod::ApiKey;
                    auth.api_key_id = Some(Uuid::new_v4().to_string());
                    auth.api_key_credential_id =
                        (kind == "agent_key_login").then(|| Uuid::new_v4().to_string());
                }
                "delegated" => {
                    auth.auth_method = AuthMethod::Delegated;
                    auth.acting_client_id = Some(Uuid::new_v4().to_string());
                }
                "service_account" => {
                    auth.auth_method = AuthMethod::ServiceAccount;
                    auth.user_id = Uuid::new_v4();
                }
                "oauth_app" => {
                    auth.auth_method = AuthMethod::AccessToken;
                    auth.oauth_client_id = Some(Uuid::new_v4().to_string());
                }
                _ => unreachable!(),
            }
            let create_service = services::create_service_request(State(state.clone()), auth.clone(), crate::telemetry::TelemetryContext::default(),
                Json(json!({"name":"Denied", "slug":"denied-identity", "base_url":"https://api.example.test", "auth_method":"none"}))).await.unwrap_err();
            let update_service = services::update_service_request(
                State(state.clone()),
                auth.clone(),
                crate::telemetry::TelemetryContext::default(),
                Path(service.id.clone()),
                Json(json!({"description":"Denied"})),
            )
            .await
            .unwrap_err();
            let create_provider = super::providers::create_provider(State(state.clone()), auth.clone(),
                Json(serde_json::from_value(json!({"name":"Denied", "slug":"denied-provider", "provider_type":"api_key"})).unwrap())).await.unwrap_err();
            let update_provider = super::providers::update_provider(
                State(state.clone()),
                auth,
                Path(provider.id.clone()),
                Json(serde_json::from_value(json!({"description":"Denied"})).unwrap()),
            )
            .await
            .unwrap_err();
            for error in [
                create_service,
                update_service,
                create_provider,
                update_provider,
            ] {
                assert!(
                    matches!(error, AppError::ForbiddenWithGuidance { .. }),
                    "{kind}"
                );
                assert_eq!(error.into_response().status(), 403, "{kind}");
            }
        }
    }
    assert_eq!(
        db.collection::<Document>(SERVICES)
            .find_one(doc! {"_id":&service.id})
            .await
            .unwrap()
            .unwrap(),
        before_service
    );
    assert_eq!(
        db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
            .find_one(doc! {"_id":&provider.id})
            .await
            .unwrap()
            .unwrap(),
        before_provider
    );
    assert_eq!(
        db.collection::<Document>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_manager_no_auth_http_ws_mcp_acl_preserves_provider_own_connection() {
    use axum::{
        extract::ws::{Message, WebSocketUpgrade},
        routing::get,
    };
    use futures::StreamExt;
    use std::{
        future::IntoFuture,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio_tungstenite::tungstenite::{Error as WsError, client::IntoClientRequest};
    let db = connect_test_database("no_auth_transport_matrix")
        .await
        .unwrap();
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", true, false).await;
    let person = seed(&db, "user", false, false).await;
    let hits = Arc::new(AtomicUsize::new(0));
    let http_hits = hits.clone();
    let ws_hits = hits.clone();
    let vendor_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let vendor = format!("http://{}", vendor_listener.local_addr().unwrap());
    let vendor_server = tokio::spawn(
        axum::serve(
            vendor_listener,
            axum::Router::new()
                .route(
                    "/read",
                    get(move || {
                        let hits = http_hits.clone();
                        async move {
                            hits.fetch_add(1, Ordering::SeqCst);
                            Json(json!({"ok":true}))
                        }
                    }),
                )
                .route(
                    "/socket",
                    get(move |ws: WebSocketUpgrade| {
                        ws_hits.fetch_add(1, Ordering::SeqCst);
                        async move {
                            ws.on_upgrade(|mut socket| async move {
                                socket
                                    .send(Message::Text("no-auth-ready".into()))
                                    .await
                                    .unwrap();
                            })
                        }
                    }),
                ),
        )
        .into_future(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut config = crate::test_utils::test_app_config();
    config.base_url = base.clone();
    config.rate_limit_per_second = 1000;
    config.rate_limit_burst = 2000;
    let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
    let mut body = new("no-auth-matrix-tool");
    body["base_url"] = vendor.clone().into();
    body["auth_method"] = "none".into();
    body["capabilities"] = json!({"supports_websocket":true});
    let tool = create(&state, &manager, body).await.unwrap();
    let mut ids = Vec::new();
    for (name, path) in [("read", "/read"), ("socket", "/socket")] {
        ids.push(
            endpoints::create_endpoint(
                State(state.clone()),
                test_auth_user(&manager),
                Path(tool.id.clone()),
                Json(
                    serde_json::from_value(json!({"name":name,"method":"GET","path":path}))
                        .unwrap(),
                ),
            )
            .await
            .unwrap()
            .0
            .id,
        );
    }
    change_test_publication(
        &db,
        &tool.id,
        tool.offering_kind,
        &ids,
        crate::models::service_endpoint::PublicationState::Published,
        &crate::services::audit_service::AuditActor::from_auth_user(&test_auth_user(&admin)),
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let server = tokio::spawn(
        axum::serve(listener, public.merge(private).with_state(state.clone())).into_future(),
    );
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&person).unwrap(),
        "openid profile email proxy write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let http = reqwest::Client::new();
    let initialized = http.post(format!("{base}/mcp")).bearer_auth(&token).header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"local-no-auth","version":"1"}}})).send().await.unwrap();
    let session = initialized.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    for public in [true, false] {
        let _ = update(&state, &admin, &tool.id, json!({"platform_key":{"enabled":true,"audience":if public {"public"} else {"restricted"},"allowed_owner_ids":[]}})).await.unwrap();
        let before = hits.load(Ordering::SeqCst);
        let response = http
            .get(format!("{base}/api/v1/proxy/s/{}/read", tool.slug))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().is_success(), public);
        let mut request = format!(
            "{}/api/v1/proxy/s/{}/socket",
            base.replacen("http://", "ws://", 1),
            tool.slug
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        match tokio_tungstenite::connect_async(request).await {
            Ok((mut socket, response)) => {
                assert!(public);
                assert_eq!(response.status(), 101);
                assert_eq!(
                    socket.next().await.unwrap().unwrap().into_text().unwrap(),
                    "no-auth-ready"
                );
            }
            Err(WsError::Http(response)) => {
                assert!(!public);
                assert!(!response.status().is_success());
            }
            Err(error) => panic!("unexpected local WS transport error: {error}"),
        }
        let response = http.post(format!("{base}/mcp")).bearer_auth(&token).header("Mcp-Session-Id",&session).header("Accept","application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nyx__call_tool","arguments":{"tool_name":format!("{}__read",tool.slug),"arguments_json":"{}"}}})).send().await.unwrap();
        let result = response.text().await.unwrap();
        if public {
            assert!(
                !result.contains("\"isError\":true") && !result.contains("\"error\":"),
                "{result}"
            );
        } else {
            assert!(result.contains("\"isError\":true") || result.contains("\"error\":"));
        }
        assert_eq!(
            hits.load(Ordering::SeqCst) - before,
            if public { 3 } else { 0 }
        );
    }
    // Provider-linked `none` is a connector contract, not provider-less no-auth.
    // Its restricted platform audience must not deny the person's own binding.
    let provider = super::providers::create_provider(State(state.clone()), test_auth_user(&admin),
        Json(serde_json::from_value(json!({"name":"Own provider", "slug":"own-none-provider", "provider_type":"api_key"})).unwrap())).await.unwrap().0;
    let linked = create(&state, &admin, json!({"name":"Own none", "slug":"own-none-linked", "base_url":vendor,"auth_method":"none"})).await.unwrap();
    // Provider-backed none uses a delegated requirement. Configure that
    // existing supported contract before linking, rather than asking creation
    // to manufacture a requirement that its safety checks deliberately reject.
    let admin_token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&admin).unwrap(),
        "openid profile email proxy write admin",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let added = http
        .post(format!("{base}/api/v1/services/{}/requirements", linked.id))
        .bearer_auth(&admin_token)
        .json(
            &json!({"provider_config_id":provider.id,"required":true,"injection_method":"bearer"}),
        )
        .send()
        .await
        .unwrap();
    assert!(added.status().is_success());
    let attached = http
        .put(format!(
            "{base}/api/v1/providers/{}/services/{}",
            provider.id, linked.id
        ))
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap();
    assert!(attached.status().is_success());
    let connected = http
        .post(format!(
            "{base}/api/v1/providers/{}/connect/api-key",
            provider.id
        ))
        .bearer_auth(&token)
        .json(&json!({"api_key":"local-owned-provider-key"}))
        .send()
        .await
        .unwrap();
    assert!(connected.status().is_success());
    let _ = update(
        &state,
        &admin,
        &linked.id,
        json!({"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[]}}),
    )
    .await
    .unwrap();
    let endpoint = crate::test_utils::test_user_endpoint(
        &Uuid::new_v4().to_string(),
        &person,
        "Own none",
        &vendor,
        None,
        Some(&linked.id),
    );
    let mut connection = crate::test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &person,
        "own-none-personal",
        &endpoint.id,
        Some(&linked.id),
        None,
    );
    connection.auth_method = "none".into();
    connection.credential_binding = Some("user".into());
    db.collection::<crate::models::user_endpoint::UserEndpoint>(
        crate::models::user_endpoint::COLLECTION_NAME,
    )
    .insert_one(endpoint)
    .await
    .unwrap();
    db.collection::<crate::models::user_service::UserService>(
        crate::models::user_service::COLLECTION_NAME,
    )
    .insert_one(connection)
    .await
    .unwrap();
    let before = hits.load(Ordering::SeqCst);
    let response = http
        .get(format!("{base}/api/v1/proxy/s/own-none-personal/read"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(hits.load(Ordering::SeqCst), before + 1);
    let usage: Vec<crate::models::usage_meter::UsageMeterRow> = db
        .collection(crate::models::usage_meter::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        usage
            .iter()
            .all(|row| row.funding.as_ref().is_none_or(|funding| funding
                .total_charge
                .is_none_or(|amount| amount == crate::models::credits::Credits::ZERO))),
        "no-auth is not charged"
    );
    server.abort();
    vendor_server.abort();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_catalog_auth_scheme_authority_validation_and_demotion() {
    use crate::models::service_endpoint::PublicationState;
    let db = connect_test_database("catalog_auth_scheme_authority")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, true).await;
    let other = seed(&db, "operator", false, true).await;
    let admin = seed(&db, "admin", false, false).await;
    let bearer = json!({"name":"Scheme", "slug":"catalog-scheme", "base_url":"https://vendor.example/api", "auth_method":"bearer", "auth_scheme":"Zoho-oauthtoken"});
    let service = create(&state, &manager, bearer.clone()).await.unwrap();
    assert_eq!(service.auth_scheme.as_deref(), Some("Zoho-oauthtoken"));
    assert_eq!(
        update(
            &state,
            &manager,
            &service.id,
            json!({"auth_scheme":"Bearer"})
        )
        .await
        .unwrap()
        .auth_scheme
        .as_deref(),
        Some("Bearer")
    );
    // D9 restricts this credential-injection field to Admin or the owning
    // marked manager, while PR 1872's shared descriptive edits stay available.
    for value in [json!("Zoho-oauthtoken"), json!(null)] {
        assert!(matches!(
            update(&state, &other, &service.id, json!({"auth_scheme":value})).await,
            Err(AppError::Forbidden(_))
        ));
    }
    assert_eq!(
        update(
            &state,
            &other,
            &service.id,
            json!({"description":"Shared metadata"})
        )
        .await
        .unwrap()
        .description
        .as_deref(),
        Some("Shared metadata")
    );
    assert!(
        update(&state, &manager, &service.id, json!({"auth_scheme":null}))
            .await
            .unwrap()
            .auth_scheme
            .is_none()
    );
    for actor in [&manager, &admin] {
        for scheme in ["", "two words", "Bearer\r\n", &"a".repeat(65)] {
            let mut body = bearer.clone();
            body["slug"] = Uuid::new_v4().to_string().into();
            body["auth_scheme"] = scheme.into();
            assert!(matches!(
                create(&state, actor, body).await,
                Err(AppError::ValidationError(_))
            ));
            assert!(matches!(
                update(&state, actor, &service.id, json!({"auth_scheme":scheme})).await,
                Err(AppError::ValidationError(_))
            ));
        }
    }
    // An Admin may make a marked row private; the creator's ordinary fallback
    // must not retain auth-scheme authority after their manager role is revoked.
    let role_ids = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &manager})
        .await
        .unwrap()
        .unwrap()
        .role_ids;
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id": &service.id},
            doc! {"$set": {"visibility": "private"}},
        )
        .await
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id": &manager}, doc! {"$set": {"role_ids": []}})
        .await
        .unwrap();
    assert!(matches!(
        update(
            &state,
            &manager,
            &service.id,
            json!({"auth_scheme":"Bearer"})
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id": &manager},
            doc! {"$set": {"role_ids": role_ids}},
        )
        .await
        .unwrap();
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id": &service.id},
            doc! {"$set": {"visibility": "public"}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &service.id},
            doc! {"$set": {"catalog_manager_created": false}},
        )
        .await
        .unwrap();
    assert!(matches!(
        update(
            &state,
            &manager,
            &service.id,
            json!({"auth_scheme":"Bearer"})
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        update(
            &state,
            &admin,
            &service.id,
            json!({"auth_scheme":"Zoho-oauthtoken"})
        )
        .await
        .unwrap()
        .auth_scheme
        .as_deref(),
        Some("Zoho-oauthtoken")
    );
    let mut invalid_method = bearer.clone();
    invalid_method["auth_method"] = "header".into();
    invalid_method["slug"] = "scheme-header".into();
    assert!(matches!(
        create(&state, &admin, invalid_method).await,
        Err(AppError::ValidationError(_))
    ));
    let mut tool_body = new("scheme-tool");
    tool_body["auth_scheme"] = "Zoho-oauthtoken".into();
    assert!(matches!(
        create(&state, &manager, tool_body.clone()).await,
        Err(AppError::Forbidden(_))
    ));
    tool_body["credential"] = "local-scheme-platform-key".into();
    let tool = create(&state, &admin, tool_body).await.unwrap();
    let operation = endpoints::create_endpoint(
        State(state.clone()),
        test_auth_user(&admin),
        Path(tool.id.clone()),
        Json(serde_json::from_value(json!({"name":"read","method":"GET","path":"/read"})).unwrap()),
    )
    .await
    .unwrap()
    .0;
    change_test_publication(
        &db,
        &tool.id,
        tool.offering_kind,
        std::slice::from_ref(&operation.id),
        PublicationState::Published,
        &crate::services::audit_service::AuditActor::from_auth_user(&test_auth_user(&admin)),
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    let published = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
    let _ = update(&state, &admin, &tool.id, json!({"auth_scheme":null}))
        .await
        .unwrap();
    let changed = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
    assert_eq!(changed[0].publication, PublicationState::Draft);
    assert_eq!(
        changed[0].operation_generation,
        published[0].operation_generation + 1
    );
    let own_tool = create(&state, &manager, new("manager-scheme-tool"))
        .await
        .unwrap();
    assert!(matches!(
        update(
            &state,
            &manager,
            &own_tool.id,
            json!({"auth_scheme":"Bearer"})
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_manager_catalog_auth_scheme_reaches_http_ws_and_mcp_user_connections() {
    use axum::{
        extract::ws::{Message, WebSocketUpgrade},
        http::HeaderMap,
        response::IntoResponse,
        routing::get,
    };
    use futures::StreamExt;
    use std::{
        future::IntoFuture,
        sync::{Arc, Mutex},
    };
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let db = connect_test_database("catalog_auth_scheme_transports")
        .await
        .unwrap();
    let manager = seed(&db, "operator", false, true).await;
    let person = seed(&db, "user", false, false).await;
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let http_seen = seen.clone();
    let ws_seen = seen.clone();
    let vendor_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let vendor = format!("http://{}", vendor_listener.local_addr().unwrap());
    let vendor_routes = axum::Router::new()
        .route(
            "/read",
            get(move |headers: HeaderMap| {
                let seen = http_seen.clone();
                async move {
                    seen.lock()
                        .unwrap()
                        .push(headers["authorization"].to_str().unwrap().to_owned());
                    Json(json!({"ok":true}))
                }
            }),
        )
        .route(
            "/socket",
            get(move |headers: HeaderMap, ws: WebSocketUpgrade| {
                let seen = ws_seen.clone();
                async move {
                    seen.lock()
                        .unwrap()
                        .push(headers["authorization"].to_str().unwrap().to_owned());
                    ws.on_upgrade(|mut socket| async move {
                        socket
                            .send(Message::Text("local-ready".into()))
                            .await
                            .unwrap();
                    })
                    .into_response()
                }
            }),
        );
    let vendor_server = tokio::spawn(axum::serve(vendor_listener, vendor_routes).into_future());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut config = crate::test_utils::test_app_config();
    config.base_url = base.clone();
    config.rate_limit_per_second = 1000;
    config.rate_limit_burst = 2000;
    let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
    let service = create(&state, &manager, json!({"name":"Scheme transports","slug":"scheme-transports","base_url":vendor,"auth_method":"bearer", "auth_scheme":"Zoho-oauthtoken", "capabilities":{"supports_websocket":true}})).await.unwrap();
    for (name, path) in [("read", "/read"), ("socket", "/socket")] {
        let _ = endpoints::create_endpoint(
            State(state.clone()),
            test_auth_user(&manager),
            Path(service.id.clone()),
            Json(serde_json::from_value(json!({"name":name,"method":"GET","path":path})).unwrap()),
        )
        .await
        .unwrap();
    }
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let server = tokio::spawn(
        axum::serve(listener, public.merge(private).with_state(state.clone())).into_future(),
    );
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&person).unwrap(),
        "openid profile email proxy write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let http = reqwest::Client::new();
    let connected = http.post(format!("{base}/api/v1/keys")).bearer_auth(&token)
        .json(&json!({"service_slug":service.slug,"slug":"scheme-personal","label":"Local scheme","credential":"local-scheme-key"})).send().await.unwrap();
    assert!(connected.status().is_success());
    let initialized = http.post(format!("{base}/mcp")).bearer_auth(&token).header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"local-scheme","version":"1"}}})).send().await.unwrap();
    assert!(initialized.status().is_success());
    let session = initialized.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let discovered = http.post(format!("{base}/mcp")).bearer_auth(&token).header("Mcp-Session-Id", &session).header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nyx__search_tools","arguments":{"query":"scheme-personal"}}})).send().await.unwrap();
    assert!(discovered.status().is_success());
    let discovered = discovered.text().await.unwrap();
    assert!(
        discovered.contains("scheme-personal__read") && !discovered.contains("\"isError\":true")
    );
    for (scheme, wire) in [
        (Some("Zoho-oauthtoken"), "Zoho-oauthtoken local-scheme-key"),
        (None, "Bearer local-scheme-key"),
    ] {
        let _ = update(&state, &manager, &service.id, json!({"auth_scheme":scheme}))
            .await
            .unwrap();
        let response = http
            .get(format!("{base}/api/v1/proxy/s/scheme-personal/read"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let mut request = format!(
            "{}/api/v1/proxy/s/scheme-personal/socket",
            base.replacen("http://", "ws://", 1)
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(response.status(), 101);
        assert_eq!(
            socket.next().await.unwrap().unwrap().into_text().unwrap(),
            "local-ready"
        );
        let response = http.post(format!("{base}/mcp")).bearer_auth(&token).header("Mcp-Session-Id", &session).header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nyx__call_tool","arguments":{"tool_name":"scheme-personal__read","arguments_json":"{}"}}})).send().await.unwrap();
        assert!(response.status().is_success());
        let result = response.text().await.unwrap();
        assert!(
            !result.contains("\"isError\":true") && !result.contains("\"error\":"),
            "MCP scheme call failed: {result}"
        );
        let captured = seen.lock().unwrap().clone();
        assert_eq!(&captured[captured.len() - 3..], &[wire, wire, wire]);
    }
    assert_eq!(seen.lock().unwrap().len(), 6);
    server.abort();
    vendor_server.abort();
    db.drop().await.unwrap();
}

fn race_operation() -> operations::EndpointInput {
    operations::EndpointInput {
        name: "read".into(),
        method: "GET".into(),
        path: "/read".into(),
        description: None,
        parameters: None,
        request_body_schema: None,
        request_content_type: None,
        request_body_required: false,
        response_description: None,
        response: Default::default(),
        risk: None,
        supports_idempotency_key: false,
        data_scope: None,
        cost_class: None,
        execution: Default::default(),
        async_operation: None,
        target_id: None,
    }
}

#[tokio::test]
async fn tools_manager_mutations_reject_stale_kind_owner_and_revoked_admin() {
    use crate::services::{audit_service::AuditActor, catalog_mutation_service as mutations};
    let db = connect_test_database("tools_parent_mutation_fences")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let catalog = seed(&db, "operator", false, true).await;
    let original = create(&state, &catalog, json!({"name":"legacy","slug":"legacy-race","base_url":"https://vendor.example/api","auth_method":"bearer"})).await.unwrap();
    let snapshot = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": &original.id})
        .await
        .unwrap()
        .unwrap();
    let actor = mutations::Authority {
        catalog_base_url: state.config.base_url.clone(),
        actor: AuditActor::from_auth_user(&test_auth_user(&catalog)),
        first_party_human: true,
        admin_required: false,
    };
    let (rows, _) = operations::commit_mutation(
        &db,
        &snapshot,
        Some(&actor),
        operations::Mutation::Create(race_operation()),
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    let op = rows[0].clone();
    for change in [doc! {"offering_kind":"tool"}, doc! {"owner_user_id":&admin}] {
        db.collection::<Document>(SERVICES)
            .update_one(doc! {"_id":&original.id}, doc! {"$set":change})
            .await
            .unwrap();
        for mutation in [
            operations::Mutation::Create(race_operation()),
            operations::Mutation::Update(
                op.id.clone(),
                operations::EndpointUpdate {
                    path: Some("/changed".into()),
                    ..Default::default()
                },
            ),
            operations::Mutation::Delete(op.id.clone()),
            operations::Mutation::Reconcile {
                inputs: vec![],
                authoritative: true,
            },
            operations::Mutation::Reconcile {
                inputs: vec![race_operation()],
                authoritative: false,
            },
        ] {
            assert!(matches!(
                operations::commit_mutation(
                    &db,
                    &snapshot,
                    Some(&actor),
                    mutation,
                    state.audit_chain_hmac_key.as_ref().as_ref()
                )
                .await,
                Err(AppError::Conflict(_))
            ));
        }
        for fields in [
            doc! {"base_url":"https://changed.example"},
            doc! {"is_active":false},
        ] {
            assert!(matches!(
                mutations::commit_update(
                    &db,
                    &snapshot,
                    &actor,
                    &fields,
                    &doc! {"_id":&snapshot.id},
                    state.audit_chain_hmac_key.as_ref().as_ref()
                )
                .await,
                Err(AppError::Conflict(_))
            ));
        }
        assert_eq!(
            operations::list_all_endpoints(&db, &original.id)
                .await
                .unwrap()[0]
                .path,
            "/read"
        );
        db.collection::<Document>(SERVICES)
            .update_one(
                doc! {"_id":&original.id},
                doc! {"$set":{"offering_kind":"ai_service","owner_user_id":bson::Bson::Null}},
            )
            .await
            .unwrap();
    }
    // Desired Tool kind must retain the original first-party auth context.
    let mut admin_auth = test_auth_user(&admin);
    admin_auth.oauth_client_id = Some(Uuid::new_v4().to_string());
    assert!(matches!(
        services::update_service(
            State(state.clone()),
            admin_auth,
            crate::telemetry::TelemetryContext::default(),
            Path(original.id.clone()),
            Json(
                serde_json::from_value(
                    json!({"offering_kind":"tool","service_category":"internal"})
                )
                .unwrap()
            )
        )
        .await,
        Err(AppError::ForbiddenWithGuidance { .. })
    ));
    let mut admin_body = new("admin-revocation-race");
    admin_body["auth_method"] = "none".into();
    let tool = create(&state, &admin, admin_body).await.unwrap();
    let tool = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":tool.id})
        .await
        .unwrap()
        .unwrap();
    let admin_authority = mutations::Authority {
        catalog_base_url: state.config.base_url.clone(),
        actor: AuditActor::from_auth_user(&test_auth_user(&admin)),
        first_party_human: true,
        admin_required: true,
    };
    // Authorized conversion is supported; stale Tool callers then conflict.
    let converted = mutations::commit_update(
        &db,
        &tool,
        &admin_authority,
        &doc! {"offering_kind":"ai_service"},
        &doc! {"_id":&tool.id},
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    assert!(matches!(
        operations::commit_mutation(
            &db,
            &tool,
            Some(&admin_authority),
            operations::Mutation::Create(race_operation()),
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let restored = mutations::commit_update(
        &db,
        &converted,
        &admin_authority,
        &doc! {"offering_kind":"tool"},
        &doc! {"_id":&tool.id},
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&admin}, doc! {"$set":{"role_ids":[]}})
        .await
        .unwrap();
    for fields in [
        doc! {"credential_encrypted":bson::Bson::Null},
        doc! {"platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]}},
        doc! {"is_active":false},
        doc! {"description":"revoked"},
    ] {
        assert!(matches!(
            mutations::commit_update(
                &db,
                &restored,
                &admin_authority,
                &fields,
                &doc! {"_id":&tool.id},
                state.audit_chain_hmac_key.as_ref().as_ref()
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_authority_matrix_and_server_owned_provenance() {
    let db = connect_test_database("tools_manager_matrix")
        .await
        .expect("local replica set required");
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", true, false).await;
    let both = seed(&db, "operator", true, true).await;
    let operator = seed(&db, "operator", false, false).await;
    let user = seed(&db, "user", true, false).await;
    let catalog = seed(&db, "operator", false, true).await;
    for (actor, allowed) in [
        (&admin, true),
        (&manager, true),
        (&both, true),
        (&operator, false),
        (&user, false),
        (&catalog, false),
    ] {
        let mut b = new(&format!("tools-{}", Uuid::new_v4()));
        if actor == &admin {
            b["platform_key"] =
                json!({"enabled":true,"audience":"restricted","allowed_owner_ids":[]});
        }
        assert_eq!(create(&state, actor, b).await.is_ok(), allowed);
    }
    let tool = create(&state, &manager, new("tools-owned")).await.unwrap();
    assert!(tool.tools_manager_created);
    assert!(!tool.tools_credential_frozen);
    assert_eq!(
        serde_json::to_value(&tool).unwrap()["platform_key"]["audience"],
        "restricted"
    );
    assert!(
        update(
            &state,
            &manager,
            &tool.id,
            json!({"supplier":"Vendor","topics":["web-search"]})
        )
        .await
        .is_ok()
    );
    assert!(
        update(&state, &both, &tool.id, json!({"name":"Foreign"}))
            .await
            .is_err()
    );
    for key in [
        "tools_manager_created",
        "tools_credential_frozen",
        "created_by",
        "owner_user_id",
        "requires_user_credential",
    ] {
        let mut b = new(&format!("tools-{}", Uuid::new_v4()));
        b[key] = true.into();
        assert!(create(&state, &manager, b).await.is_err());
    }
    let legacy = create(&state, &admin, {
        let mut b = new("tools-legacy");
        b["platform_key"] = json!({"enabled":true,"audience":"restricted","allowed_owner_ids":[]});
        b
    })
    .await
    .unwrap();
    assert!(!legacy.tools_manager_created);
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&legacy.id},
            doc! {"$set":{"created_by":&manager}},
        )
        .await
        .unwrap();
    assert!(
        update(&state, &manager, &legacy.id, json!({"name":"Legacy"}))
            .await
            .is_err()
    );
    let mut auth = test_auth_user(&manager);
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    assert!(
        authority::require_tool_writer(&state, &auth, None)
            .await
            .is_err()
    );
    for kind in [
        crate::mw::auth::AuthMethod::Delegated,
        crate::mw::auth::AuthMethod::Relay,
        crate::mw::auth::AuthMethod::ServiceAccount,
    ] {
        auth.auth_method = kind;
        assert!(
            authority::require_tool_writer(&state, &auth, None)
                .await
                .is_err()
        );
    }
    auth = test_auth_user(&manager);
    auth.oauth_client_id = Some("app".into());
    assert!(
        authority::require_tool_writer(&state, &auth, None)
            .await
            .is_err()
    );
    let role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"tools_manager"})
        .await
        .unwrap()
        .unwrap();
    role_service::revoke_role_from_user(&db, &manager, &role.id)
        .await
        .unwrap();
    assert!(
        update(&state, &manager, &tool.id, json!({"supplier":"Revoked"}))
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_rejects_all_privileged_fields_and_alternate_routes() {
    let db = connect_test_database("tools_manager_fields")
        .await
        .expect("replica set");
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, true).await;
    let mut linked = new("tools-forbidden-provider");
    linked["provider_config_id"] = Uuid::new_v4().to_string().into();
    assert!(
        matches!(
            create(&state, &manager, linked).await,
            Err(AppError::Forbidden(_))
        ),
        "Tools Manager cannot borrow provider authority even when holding both manager roles"
    );
    let tool = create(&state, &manager, new("tools-fields")).await.unwrap();
    for (key, value) in [
        ("offering_kind", json!("ai_service")),
        ("service_category", json!("internal")),
        ("is_active", json!(true)),
        ("visibility", json!("public")),
        ("billing", json!({})),
        ("inference", json!(null)),
        (
            "platform_key",
            json!({"enabled":true,"audience":"public","allowed_owner_ids":[]}),
        ),
        ("destination_targets", json!({})),
        ("import_source", json!(null)),
        ("identity_propagation_mode", json!("none")),
        ("identity_include_email", json!(false)),
        ("identity_include_user_id", json!(false)),
        ("identity_include_name", json!(false)),
        ("identity_jwt_audience", json!("aud")),
        ("forward_access_token", json!(false)),
        ("inject_delegation_token", json!(false)),
        ("delegation_token_scope", json!("proxy")),
        ("default_request_headers", json!([])),
        ("ws_frame_injections", json!([])),
        ("anonymous_endpoints", json!([])),
        ("proxy_operation_policy", json!(null)),
        ("recommended_skills", json!([])),
        ("recommended_skill_refs", json!([])),
        ("clear_skill_refs", json!(true)),
        ("skills_revision", json!(1)),
        ("skills_request_id", json!("x")),
        ("developer_app_ids", json!([])),
        ("custom_user_agent", json!("x")),
        ("asyncapi_spec_url", json!("https://vendor.example/async")),
    ] {
        let mut body = json!({});
        body[key] = value;
        let body: services::UpdateServiceRequest =
            serde_json::from_value(body).unwrap_or_else(|_| panic!("fixture field {key}"));
        assert!(authority::check_tool_update(&body).is_err(), "field {key}");
    }
    assert!(
        services::delete_service(
            State(state.clone()),
            test_auth_user(&manager),
            Path(tool.id.clone())
        )
        .await
        .is_err()
    );
    assert!(
        super::services_helpers::require_admin_or_creator(
            &state,
            &test_auth_user(&manager),
            &db.collection::<DownstreamService>(SERVICES)
                .find_one(doc! {"_id":&tool.id})
                .await
                .unwrap()
                .unwrap()
        )
        .await
        .is_err()
    );
    assert!(
        serde_json::from_value::<services::UpdateServiceRequest>(
            json!({"tools_manager_created":true})
        )
        .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_first_secret_races_and_freeze_survives_admin_clear() {
    let db = connect_test_database("tools_manager_secret_race")
        .await
        .expect("replica set");
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let tool = create(&state, &manager, new("tools-race")).await.unwrap();
    let (a, b) = tokio::join!(
        update(
            &state,
            &manager,
            &tool.id,
            json!({"credential":"fake-secret-a"})
        ),
        update(
            &state,
            &manager,
            &tool.id,
            json!({"credential":"fake-secret-b"})
        )
    );
    assert_ne!(a.is_ok(), b.is_ok(), "first set has exactly one winner");
    let stored = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.tools_credential_frozen);
    let plain = state
        .encryption_keys
        .decrypt(&stored.credential_encrypted)
        .await
        .unwrap();
    assert!(plain == b"fake-secret-a" || plain == b"fake-secret-b");
    assert!(!format!("{stored:?}").contains("fake-secret"));
    assert!(
        update(
            &state,
            &manager,
            &tool.id,
            json!({"base_url":"https://attacker.example"})
        )
        .await
        .is_err()
    );
    assert!(
        update(
            &state,
            &manager,
            &tool.id,
            json!({"credential":"replacement"})
        )
        .await
        .is_err()
    );
    update(&state, &admin, &tool.id, json!({"clear_credential":true}))
        .await
        .unwrap();
    assert!(
        update(&state, &manager, &tool.id, json!({"credential":"reopened"}))
            .await
            .is_err()
    );
    assert!(
        update(
            &state,
            &manager,
            &tool.id,
            json!({"base_url":"https://attacker.example"})
        )
        .await
        .is_err()
    );
    let tool = create(&state, &manager, new("tools-url-race"))
        .await
        .unwrap();
    assert!(
        update(
            &state,
            &manager,
            &tool.id,
            json!({"base_url":"https://other.example","credential":"fake"})
        )
        .await
        .is_err()
    );
    let observed = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    let actor =
        crate::services::audit_service::AuditActor::from_auth_user(&test_auth_user(&manager));
    let frozen = doc! {"credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:state.encryption_keys.encrypt(b"fake-race").await.unwrap()},"tools_credential_frozen":true,"updated_at":bson::DateTime::now()};
    let moved = doc! {"base_url":"https://other.example","updated_at":bson::DateTime::from_chrono(observed.updated_at+chrono::Duration::milliseconds(1))};
    let (a, b) = tokio::join!(
        crate::services::tools_manager_service::commit_manager_update(
            &db, &observed, &manager, &frozen, &[2; 32], &actor
        ),
        crate::services::tools_manager_service::commit_manager_update(
            &db, &observed, &manager, &moved, &[2; 32], &actor
        )
    );
    assert_ne!(a.is_ok(), b.is_ok(), "one destination-version winner");
    let stored = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.base_url != "https://other.example" || stored.credential_encrypted.is_empty());
    let audits: Vec<Document> = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(!format!("{audits:?}").contains("fake-race"));
    assert!(
        audits
            .iter()
            .filter(|d| d.get_str("event_type").ok() == Some("tools_manager_updated"))
            .all(|d| d.contains_key("entry_hash"))
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_twin_sanitizes_source_and_keeps_drafts() {
    use futures::TryStreamExt;
    let db = connect_test_database("tools_manager_twin")
        .await
        .expect("replica set");
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let mut source = crate::models::downstream_service::test_helpers::dummy_service();
    source.id = Uuid::new_v4().to_string();
    source.slug = "api-source".into();
    source.created_by = "foreign".into();
    source.auth_method = "bearer".into();
    source.auth_key_name = "Authorization".into();
    source.service_type = "http".into();
    source.visibility = "private".into();
    source.credential_encrypted = vec![8; 40];
    source.default_request_headers = Some(vec![
        crate::models::default_request_header::DefaultRequestHeader {
            name: "X-Secret".into(),
            value: "fake-source-secret".into(),
            sensitive: true,
            overridable: false,
        },
    ]);
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&source)
        .await
        .unwrap();
    let body =
        json!({"slug":"tools-twin","offering_kind":"tool","twin_of_service_id":"api-source"});
    assert!(create(&state, &manager, body.clone()).await.is_err());
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&source.id},
            doc! {"$set":{"visibility":"public"}},
        )
        .await
        .unwrap();
    let mut invalid = body.clone();
    invalid["name"] = json!({"invalid":true});
    assert!(matches!(
        create(&state, &manager, invalid).await,
        Err(AppError::ValidationError(_))
    ));
    let tool = create(&state, &manager, body).await.unwrap();
    assert_eq!(tool.name, source.name);
    let stored = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.credential_encrypted.is_empty());
    assert!(stored.default_request_headers.is_none());
    assert!(stored.destination_targets.is_empty());
    assert!(stored.provider_config_id.is_none());
    assert!(stored.billing.is_none());
    assert!(stored.inference.is_none());
    assert_eq!(
        stored.platform_key.unwrap().audience,
        crate::models::downstream_service::PlatformKeyAudience::Restricted
    );
    let rows: Vec<Document> = db
        .collection::<Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .find(doc! {"service_id":&tool.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        rows.iter()
            .all(|e| e.get_str("publication").ok() == Some("draft")
                && e.get_bool("is_active").ok() == Some(false))
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_publication_and_edit_demotion() {
    let db = connect_test_database("tools_manager_publication")
        .await
        .expect("replica set");
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let tool = create(&state, &manager, new("tools-publication"))
        .await
        .unwrap();
    let endpoint = endpoints::create_endpoint(
        State(state.clone()),
        test_auth_user(&manager),
        Path(tool.id.clone()),
        Json(
            serde_json::from_value(json!({"name":"search","method":"GET","path":"/search"}))
                .unwrap(),
        ),
    )
    .await
    .unwrap()
    .0;
    assert!(!endpoint.is_active);
    for status in ["validated", "published", "paused", "draft"] {
        let result = endpoints::change_publication(
            State(state.clone()),
            test_auth_user(&manager),
            Path((tool.id.clone(), endpoint.id.clone())),
            Json(serde_json::from_value(json!({"state":status})).unwrap()),
        )
        .await;
        assert_eq!(result.is_ok(), status != "published");
    }
    let _ = endpoints::change_publication(
        State(state.clone()),
        test_auth_user(&admin),
        Path((tool.id.clone(), endpoint.id.clone())),
        Json(serde_json::from_value(json!({"state":"published"})).unwrap()),
    )
    .await
    .unwrap();
    let _ = endpoints::update_endpoint(
        State(state.clone()),
        test_auth_user(&manager),
        Path((tool.id.clone(), endpoint.id.clone())),
        Json(serde_json::from_value(json!({"path":"/changed"})).unwrap()),
    )
    .await
    .unwrap();
    let e = operations::list_all_endpoints(&db, &tool.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        e.publication,
        crate::models::service_endpoint::PublicationState::Draft
    );
    assert!(!e.is_active);
    assert!(e.operation_generation > 1);
    assert!(
        endpoints::delete_endpoint(
            State(state.clone()),
            test_auth_user(&manager),
            Path((tool.id.clone(), endpoint.id.clone()))
        )
        .await
        .is_err()
    );
    assert!(
        endpoints::update_endpoint(
            State(state),
            test_auth_user(&manager),
            Path((tool.id, endpoint.id)),
            Json(serde_json::from_value(json!({"is_active":true})).unwrap())
        )
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_admin_contract_updates_demote_and_noops_preserve_publication() {
    let db = connect_test_database("tools_manager_admin_contract")
        .await
        .expect("local replica set");
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let tool = create(&state, &manager, new("tools-admin-contract"))
        .await
        .unwrap();
    let endpoint = endpoints::create_endpoint(
        State(state.clone()),
        test_auth_user(&manager),
        Path(tool.id.clone()),
        Json(
            serde_json::from_value(json!({"name":"search","method":"GET","path":"/search"}))
                .unwrap(),
        ),
    )
    .await
    .unwrap()
    .0;
    for body in [
        json!({"base_url":"https://other.example/api"}),
        json!({"base_url":"https://third.example/api","recommended_skills":null,"skills_revision":0}),
        json!({"custom_user_agent":"Local Tool Verification"}),
    ] {
        let _ = endpoints::change_publication(
            State(state.clone()),
            test_auth_user(&admin),
            Path((tool.id.clone(), endpoint.id.clone())),
            Json(serde_json::from_value(json!({"state":"published"})).unwrap()),
        )
        .await
        .unwrap();
        let before = operations::list_all_endpoints(&db, &tool.id)
            .await
            .unwrap()
            .remove(0);
        update(&state, &admin, &tool.id, body.clone())
            .await
            .unwrap();
        let after = operations::list_all_endpoints(&db, &tool.id)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(
            after.publication,
            crate::models::service_endpoint::PublicationState::Draft
        );
        assert!(!after.is_active);
        assert!(after.operation_generation > before.operation_generation);
        let _ = endpoints::change_publication(
            State(state.clone()),
            test_auth_user(&admin),
            Path((tool.id.clone(), endpoint.id.clone())),
            Json(serde_json::from_value(json!({"state":"published"})).unwrap()),
        )
        .await
        .unwrap();
        let before = operations::list_all_endpoints(&db, &tool.id)
            .await
            .unwrap()
            .remove(0);
        update(&state, &admin, &tool.id, body).await.unwrap();
        let after = operations::list_all_endpoints(&db, &tool.id)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(
            after.publication,
            crate::models::service_endpoint::PublicationState::Published
        );
        assert_eq!(after.operation_generation, before.operation_generation);
    }
    db.drop().await.unwrap();
}

#[derive(Default)]
struct LocalLago;
#[async_trait::async_trait]
impl crate::services::billing::lago_client::LagoApi for LocalLago {
    async fn ensure_customer(
        &self,
        o: &crate::services::billing::lago_client::OwnerProvisionInput,
    ) -> crate::errors::AppResult<String> {
        Ok(o.external_customer_id.clone())
    }
    async fn ensure_subscription(&self, c: &str, _: &str) -> crate::errors::AppResult<String> {
        Ok(format!("{c}:plan"))
    }
    async fn record_event(
        &self,
        e: &crate::services::billing::lago_client::LagoEvent,
    ) -> Result<
        crate::services::billing::lago_client::LagoAck,
        crate::services::billing::lago_client::LagoError,
    > {
        Ok(crate::services::billing::lago_client::LagoAck {
            transaction_id: e.transaction_id.clone(),
        })
    }
    async fn record_events_batch(
        &self,
        events: &[crate::services::billing::lago_client::LagoEvent],
    ) -> Result<
        Vec<crate::services::billing::lago_client::LagoAck>,
        crate::services::billing::lago_client::LagoError,
    > {
        Ok(events
            .iter()
            .map(|e| crate::services::billing::lago_client::LagoAck {
                transaction_id: e.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(
        &self,
        c: &str,
        s: &str,
    ) -> crate::errors::AppResult<crate::services::billing::lago_client::LagoUsage> {
        Ok(crate::services::billing::lago_client::LagoUsage {
            customer_id: c.into(),
            subscription_id: s.into(),
            raw: json!({}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> crate::errors::AppResult<i64> {
        Ok(100)
    }
    async fn entitlements(
        &self,
        _: &str,
    ) -> crate::errors::AppResult<Vec<crate::services::billing::lago_client::Entitlement>> {
        // Entitlement checks bind the resolved instance slug, including custom
        // BYOK connections rather than only catalog platform rows.
        Ok([
            "tools-fake-twin",
            "tools-fake-new",
            "tools-manager-byok",
            "my-tool-byok",
            "local-e2e-byok",
        ]
        .into_iter()
        .map(|code| crate::services::billing::lago_client::Entitlement {
            code: code.into(),
            raw: json!({}),
        })
        .collect())
    }
    async fn sync_standard_charge(
        &self,
        _: &str,
        _: &crate::services::billing::lago_client::ServicePriceSync,
    ) -> crate::errors::AppResult<()> {
        Ok(())
    }
}

async fn fund_local_person(db: &mongodb::Database, person: &str) {
    let now = bson::DateTime::now();
    db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME).insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"owner_id":person,"lago_customer_id":person,
        "lago_wallet_id":format!("local-wallet-{person}"),"lago_subscription_id":"local-plan",
        "plan_kind":"prepaid","balance_credits":crate::models::credits::Credits::from_whole(100),
        "reserved_credits":crate::models::credits::Credits::ZERO,"pending_lago_debits":crate::models::credits::Credits::ZERO,
        "overdraft_cap_credits":crate::models::credits::Credits::ZERO,"has_payment_instrument":false,
        "suspended":false,"collection_state":"good","balance_synced_at":now,"created_at":now,"updated_at":now
    }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_manager_published_byok_bills_user_without_platform_audience() {
    use crate::models::usage_meter::{CredentialClass, UsageMeterRow};
    use std::{future::IntoFuture, sync::Arc};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    let db = connect_test_database("tools_manager_byok_lanes")
        .await
        .unwrap();
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", true, false).await;
    let person = seed(&db, "user", false, false).await;
    let vendor = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/read"))
        .and(header("Authorization", "Bearer local-user-byok-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&vendor)
        .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = true;
    config.base_url = base.clone();
    config.rate_limit_per_second = 1000;
    config.rate_limit_burst = 2000;
    let mut state = crate::test_utils::test_app_state_with_config(db.clone(), config.clone());
    state.billing = Arc::new(crate::services::billing::BillingService::new_with_lago(
        db.clone(),
        Arc::new(config),
        Arc::new(LocalLago),
    ));
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let server = tokio::spawn(
        axum::serve(listener, public.merge(private).with_state(state.clone())).into_future(),
    );
    let token = |id: &str| {
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(id).unwrap(),
            "openid profile email proxy write",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let admin_token = token(&admin);
    let person_token = token(&person);
    let http = reqwest::Client::new();
    let mut body = new("tools-manager-byok");
    body["base_url"] = vendor.uri().into();
    let tool = create(&state, &manager, body).await.unwrap();
    let _ = update(
        &state,
        &manager,
        &tool.id,
        json!({"credential":"local-shared-platform-key"}),
    )
    .await
    .unwrap();
    let operation = endpoints::create_endpoint(
        State(state.clone()),
        test_auth_user(&manager),
        Path(tool.id.clone()),
        Json(serde_json::from_value(json!({"name":"read","method":"GET","path":"/read"})).unwrap()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        operation.publication,
        crate::models::service_endpoint::PublicationState::Draft
    );
    let connected = http.post(format!("{base}/api/v1/keys")).bearer_auth(&person_token)
        .json(&json!({"service_slug":tool.slug,"slug":"my-tool-byok","label":"Local BYOK", "credential":"local-user-byok-key"}))
        .send().await.unwrap();
    assert!(
        connected.status().is_success(),
        "BYOK binding must be allowed with an empty platform audience"
    );
    let connected: Value = connected.json().await.unwrap();
    assert_eq!(connected["credential_binding"], "user");
    let own_slug = connected["slug"].as_str().unwrap();
    fund_local_person(&db, &person).await;
    let _ = update(
        &state,
        &admin,
        &tool.id,
        json!({"platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]},
        "billing":{"byok_pricing":{"metric":"requests","credits_per_unit":"0.000000000001"}}}),
    )
    .await
    .unwrap();
    // Public audience makes the platform denial exercise publication, rather
    // than stopping earlier at its ACL. The BYOK binding stays independent.
    for slug in [tool.slug.as_str(), own_slug] {
        let denied = http
            .get(format!("{base}/api/v1/proxy/s/{slug}/read"))
            .bearer_auth(&person_token)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 404);
        assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    }
    assert!(vendor.received_requests().await.unwrap().is_empty());
    assert_eq!(
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let published = http
        .post(format!("{base}/api/v1/services/{}/publication", tool.id))
        .bearer_auth(&admin_token)
        .json(&json!({"state":"published","endpoint_names":["read"]}))
        .send()
        .await
        .unwrap();
    assert!(published.status().is_success());
    let _ = update(
        &state,
        &admin,
        &tool.id,
        json!({"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[]}}),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let service = db
                .collection::<DownstreamService>(SERVICES)
                .find_one(doc! {"_id":&tool.id})
                .await
                .unwrap()
                .unwrap();
            if service
                .billing
                .as_ref()
                .and_then(|b| b.byok_pricing.as_ref())
                .is_some_and(|price| {
                    price.sync_status == crate::models::service_billing::PricingSyncStatus::Synced
                })
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("local BYOK price sync");
    let denied = http
        .get(format!("{base}/api/v1/proxy/s/{}/read", tool.slug))
        .bearer_auth(&person_token)
        .send()
        .await
        .unwrap();
    assert!(
        !denied.status().is_success(),
        "restricted-empty platform lane stays denied"
    );
    assert!(vendor.received_requests().await.unwrap().is_empty());
    let called = http
        .get(format!("{base}/api/v1/proxy/s/{own_slug}/read"))
        .bearer_auth(&person_token)
        .send()
        .await
        .unwrap();
    let status = called.status();
    let body: Value = called.json().await.unwrap();
    assert_eq!(
        status, 200,
        "published BYOK ignores the platform audience: {body}"
    );
    assert_eq!(vendor.received_requests().await.unwrap().len(), 1);
    let row = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(row) = db
                .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                .find_one(doc! {"actor_user_id":&person,"credential_class":"user_owned"})
                .await
                .unwrap()
                && row.funding.as_ref().is_some_and(|funding| funding.settled)
            {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("local BYOK usage settles");
    assert_eq!(row.credential_class, CredentialClass::UserOwned);
    assert_eq!(row.billing_owner_id, person);
    assert_eq!(row.lago_metric_code, "platform_svc_tools-manager-byok_byok");
    assert_eq!(
        row.funding.unwrap().wallet_charge_credits,
        Some(crate::models::credits::Credits::from_pico(1).unwrap())
    );
    assert_ne!(row.billing_owner_id, admin);
    assert_ne!(row.billing_owner_id, manager);
    let _ = update(
        &state,
        &admin,
        &tool.id,
        json!({"platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]}}),
    )
    .await
    .unwrap();
    let paused = endpoints::change_publication_bulk(
        State(state.clone()),
        test_auth_user(&manager),
        Path(tool.id.clone()),
        Json(endpoints::BulkPublicationRequest {
            state: crate::models::service_endpoint::PublicationState::Paused,
            endpoint_names: vec!["read".into()],
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        paused.0.endpoints[0].publication,
        crate::models::service_endpoint::PublicationState::Paused
    );
    let usage = db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    for slug in [tool.slug.as_str(), own_slug] {
        let denied = http
            .get(format!("{base}/api/v1/proxy/s/{slug}/read"))
            .bearer_auth(&person_token)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 404);
        assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    }
    assert_eq!(vendor.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        usage
    );
    assert!(
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME
        )
        .find_one(doc! {"_id":connected["id"].as_str().unwrap(),"is_active":true})
        .await
        .unwrap()
        .is_some(),
        "Tool mutations preserve the BYOK binding"
    );
    server.abort();
    db.drop().await.unwrap();
}

/// Executed by scripts/tools/test-tools-manager-e2e.sh, never against a remote
/// database or vendor. The real CLI, Axum ingress, proxy and billing run here.
#[cfg(not(coverage))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the locally built CLI; run scripts/tools/test-tools-manager-e2e.sh"]
async fn tools_manager_local_fake_vendor_e2e() {
    use crate::models::usage_meter::UsageMeterRow;
    use std::future::IntoFuture;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    let cli =
        std::env::var("NYXID_TOOLS_E2E_CLI").expect("script must build and provide local CLI");
    let uri =
        std::env::var("NYXID_TEST_DATABASE_URL").expect("ephemeral local replica set required");
    assert!(
        uri.starts_with("mongodb://127.0.0.1:"),
        "E2E refuses nonlocal MongoDB"
    );
    let db = connect_test_database("tools_manager_local_e2e")
        .await
        .expect("replica set");
    db.collection::<Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"service_id":1,"name":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", false, false).await;
    let user = seed(&db, "user", false, false).await;
    let byok_user = seed(&db, "user", false, false).await;
    let vendor = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/search"))
        .and(header(
            "Authorization",
            "Bearer fake-tool-credential-local-only",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true,"data":[]})))
        .mount(&vendor)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/search"))
        .and(header("Authorization", "Bearer fake-user-byok-local-only"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true,"data":[]})))
        .mount(&vendor)
        .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = true;
    config.base_url = base.clone();
    config.rate_limit_per_second = 1000;
    config.rate_limit_burst = 2000;
    let mut state = crate::test_utils::test_app_state_with_config(db.clone(), config.clone());
    state.billing = Arc::new(crate::services::billing::BillingService::new_with_lago(
        db.clone(),
        Arc::new(config),
        Arc::new(LocalLago),
    ));
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    let server = tokio::spawn(
        axum::serve(listener, public.merge(private).with_state(state.clone())).into_future(),
    );
    let token = |id: &str| {
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(id).unwrap(),
            "openid profile email proxy write admin",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let admin_token = token(&admin);
    let manager_token = token(&manager);
    let user_token = token(&user);
    let byok_token = token(&byok_user);
    let http = reqwest::Client::new();
    let initialized = http
        .post(format!("{base}/mcp"))
        .bearer_auth(&user_token)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"local-tools-e2e","version":"1"}}}))
        .send()
        .await
        .unwrap();
    assert!(initialized.status().is_success());
    let mcp_session = initialized
        .headers()
        .get("mcp-session-id")
        .expect("local MCP initialization creates a session")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        initialized
            .json::<Value>()
            .await
            .unwrap()
            .get("error")
            .is_none()
    );
    let role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"tools_manager"})
        .await
        .unwrap()
        .unwrap();
    let grant = http
        .post(format!(
            "{base}/api/v1/admin/users/{manager}/roles/{}",
            role.id
        ))
        .bearer_auth(&admin_token)
        .json(&json!({"role_id":role.id}))
        .send()
        .await
        .unwrap();
    assert!(
        grant.status().is_success(),
        "Admin role grant through ingress"
    );
    // Seed only an ordinary source; all Tool writes go through HTTP/CLI.
    let mut source = crate::models::downstream_service::test_helpers::dummy_service();
    source.id = Uuid::new_v4().to_string();
    source.created_by = admin.clone();
    source.slug = "api-fake-vendor".into();
    source.name = "Fake Vendor".into();
    source.base_url = vendor.uri();
    source.auth_method = "bearer".into();
    source.auth_key_name = "Authorization".into();
    source.service_category = "connection".into();
    source.requires_user_credential = true;
    source.credential_encrypted = vec![];
    source.openapi_spec_url = Some(format!(
        "{base}/api/v1/catalog-specs/firecrawl/openapi.json"
    ));
    source.platform_key = None;
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&source)
        .await
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("tools.json");
    let manifest = json!({"version":1,"tools":[
        {"slug":"tools-fake-twin","name":"Fake Twin","twin_of":"api-fake-vendor","supplier":"Local fake vendor","topics":["web-search"],"credential_env":"TOOLS_LOCAL_FAKE_CREDENTIAL","operations":["search"],"lanes":["platform","byok"]},
        {"slug":"tools-fake-new","name":"Fake New API","base_url":vendor.uri(),"auth_method":"bearer","openapi_spec_url":source.openapi_spec_url,"credential_env":"TOOLS_LOCAL_FAKE_CREDENTIAL","operations":["search"]}
    ]});
    std::fs::write(&file, manifest.to_string()).unwrap();
    let run_cli = |who: String,
                   args: Vec<String>,
                   file: std::path::PathBuf,
                   base: String,
                   cli: String| async move {
        let out = tokio::process::Command::new(cli)
            .args(args)
            .arg("--file")
            .arg(file)
            .args([
                "--base-url",
                &base,
                "--output",
                "json",
                "--access-token-env",
                "TOOLS_LOCAL_ACCESS_TOKEN",
            ])
            .env("TOOLS_LOCAL_ACCESS_TOKEN", who)
            .env_remove("NYXID_ACCESS_TOKEN")
            .env_remove("NYXID_TELEMETRY_DSN")
            .env("NYXID_SHARE_ANALYTICS", "0")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env(
                "TOOLS_LOCAL_FAKE_CREDENTIAL",
                "fake-tool-credential-local-only",
            )
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("DO_NOT_TRACK", "1")
            .output()
            .await
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            !stdout.contains("fake-tool-credential-local-only")
                && !stderr.contains("fake-tool-credential-local-only"),
            "CLI secret redaction"
        );
        (out.status.code().unwrap_or(-1), stdout, stderr)
    };
    let before = db
        .collection::<Document>(SERVICES)
        .count_documents(doc! {})
        .await
        .unwrap();
    let malformed = tmp.path().join("malformed.json");
    std::fs::write(
        &malformed,
        r#"{"version":1,"tools":[{"slug":"tools-bad","credential":"DO_NOT_ECHO_LOCAL_SECRET"}]}"#,
    )
    .unwrap();
    let (code, out, err) = run_cli(
        manager_token.clone(),
        vec!["admin".into(), "tools".into(), "apply".into()],
        malformed,
        base.clone(),
        cli.clone(),
    )
    .await;
    assert_eq!(code, 1);
    assert!(!out.contains("DO_NOT_ECHO_LOCAL_SECRET") && !err.contains("DO_NOT_ECHO_LOCAL_SECRET"));
    // Generic CLI errors follow the shared stderr JSON contract; successful
    // reports (including pending readiness) use stdout.
    assert!(out.is_empty());
    assert!(serde_json::from_str::<Value>(&err).is_ok());
    let (code, out, err) = run_cli(
        manager_token.clone(),
        vec!["admin".into(), "tools".into(), "add".into()],
        file.clone(),
        base.clone(),
        cli.clone(),
    )
    .await;
    assert_eq!(code, 1, "noninteractive add requires --yes");
    assert!(out.is_empty());
    assert!(serde_json::from_str::<Value>(&err).is_ok());
    assert_eq!(
        db.collection::<Document>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    let (code, out, err) = run_cli(
        manager_token.clone(),
        vec![
            "admin".into(),
            "tools".into(),
            "apply".into(),
            "--dry-run".into(),
        ],
        file.clone(),
        base.clone(),
        cli.clone(),
    )
    .await;
    assert_eq!(code, 0, "dry-run: {err}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["kind"], "plan");
    assert_eq!(
        db.collection::<Document>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    let (code, _, err) = run_cli(
        manager_token.clone(),
        vec!["admin".into(), "tools".into(), "apply".into()],
        file.clone(),
        base.clone(),
        cli.clone(),
    )
    .await;
    assert_eq!(code, 0, "apply: {err}");
    let (code, out, _) = run_cli(
        manager_token.clone(),
        vec!["admin".into(), "tools".into(), "validate".into()],
        file.clone(),
        base.clone(),
        cli.clone(),
    )
    .await;
    assert_eq!(code, 2);
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap()["kind"],
        "validation"
    );
    assert!(out.contains("pending_admin"));
    let twin = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"slug":"tools-fake-twin"})
        .await
        .unwrap()
        .unwrap();
    let connected = http.post(format!("{base}/api/v1/keys")).bearer_auth(&byok_token)
        .json(&json!({"service_slug":twin.slug,"slug":"local-e2e-byok","label":"Local BYOK","credential":"fake-user-byok-local-only"}))
        .send().await.unwrap();
    assert!(
        connected.status().is_success(),
        "second person connects a BYOK key"
    );
    let connected: Value = connected.json().await.unwrap();
    assert_eq!(connected["credential_binding"], "user");
    let byok_slug = connected["slug"].as_str().unwrap();
    fund_local_person(&db, &byok_user).await;
    let denied = http
        .post(format!("{base}/api/v1/proxy/s/{byok_slug}/v2/search"))
        .bearer_auth(&byok_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 404);
    assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    assert!(vendor.received_requests().await.unwrap().is_empty());
    let svg_file = tmp.path().join("tool.svg");
    std::fs::write(&svg_file,r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M1 1L23 23" /></svg>"#).unwrap();
    let (code, out, err) = run_cli(
        manager_token.clone(),
        vec![
            "admin".into(),
            "tools".into(),
            "icon".into(),
            "tools-fake-twin".into(),
        ],
        svg_file,
        base.clone(),
        cli.clone(),
    )
    .await;
    assert_eq!(code, 0, "CLI Tool icon: {err}");
    let uploaded: Value = serde_json::from_str(&out).unwrap();
    let asset = http
        .get(uploaded["tools"][0]["icon_url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(asset.status(), 200);
    assert!(
        asset.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    assert!(asset.headers().contains_key("content-security-policy"));
    let rejected = http
        .post(format!("{base}/api/v1/services/{}/publication", twin.id))
        .bearer_auth(&manager_token)
        .json(&json!({"state":"published","endpoint_names":["search"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 403);
    let rejected = http
        .put(format!("{base}/api/v1/services/{}", twin.id))
        .bearer_auth(&manager_token)
        .json(&json!({"credential":"replacement-fake"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 403);
    let usage_before = db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    let denied = http
        .post(format!("{base}/api/v1/proxy/s/tools-fake-twin/v2/search"))
        .bearer_auth(&user_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    assert!(!denied.status().is_success());
    assert_eq!(
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        usage_before
    );
    for slug in ["tools-fake-twin", "tools-fake-new"] {
        let service = db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"slug":slug})
            .await
            .unwrap()
            .unwrap();
        let configured=http.put(format!("{base}/api/v1/services/{}",service.id)).bearer_auth(&admin_token).json(&json!({"platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]},"billing":{"platform_key_pricing":{"metric":"requests","credits_per_unit":"0.000000000001"},"byok_pricing":{"metric":"requests","credits_per_unit":"0.000000000001"}}})).send().await.unwrap();
        assert!(
            configured.status().is_success(),
            "Admin audience and price setup"
        );
        // A public audience does not authorize prepared operations. Exercise
        // both HTTP and the MCP ingress before Admin publication.
        let denied = http
            .post(format!("{base}/api/v1/proxy/s/{slug}/v2/search"))
            .bearer_auth(&user_token)
            .json(&json!({"query":"local"}))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 404);
        assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
        let denied = http.post(format!("{base}/mcp"))
            .bearer_auth(&user_token).header("Mcp-Session-Id", &mcp_session).header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":format!("{slug}__search"),"arguments":{"query":"local"}}}))
            .send().await.unwrap();
        let denied = denied.text().await.unwrap();
        assert!(
            denied.contains("12700"),
            "MCP unpublished operation: {denied}"
        );
        assert!(vendor.received_requests().await.unwrap().is_empty());
        assert_eq!(
            db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            usage_before
        );
        let published = http
            .post(format!("{base}/api/v1/services/{}/publication", service.id))
            .bearer_auth(&admin_token)
            .json(&json!({"state":"published","endpoint_names":["search"]}))
            .send()
            .await
            .unwrap();
        assert!(published.status().is_success());
    }
    let now = bson::DateTime::now();
    db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME).insert_one(doc!{"_id":Uuid::new_v4().to_string(),"owner_id":&user,"lago_customer_id":&user,"lago_wallet_id":"local-wallet","lago_subscription_id":"local-plan","plan_kind":"prepaid","balance_credits":crate::models::credits::Credits::from_whole(100),"reserved_credits":crate::models::credits::Credits::ZERO,"pending_lago_debits":crate::models::credits::Credits::ZERO,"overdraft_cap_credits":crate::models::credits::Credits::ZERO,"has_payment_instrument":false,"suspended":false,"collection_state":"good","balance_synced_at":now,"created_at":now,"updated_at":now}).await.unwrap();
    let called = http
        .post(format!("{base}/api/v1/proxy/s/tools-fake-twin/v2/search"))
        .bearer_auth(&user_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    let called_status = called.status();
    let called_body = called.json::<Value>().await.unwrap();
    assert_eq!(called_status, 200, "real proxy request: {called_body}");
    let row = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(row) = db
                .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                .find_one(doc! {"actor_user_id":&user})
                .await
                .unwrap()
                && row.funding.as_ref().is_some_and(|f| f.settled)
            {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("usage settles locally");
    assert_eq!(row.billing_owner_id, user);
    assert_eq!(
        row.funding.unwrap().wallet_charge_credits,
        Some(crate::models::credits::Credits::from_pico(1).unwrap())
    );
    assert_ne!(row.billing_owner_id, manager);
    assert_ne!(row.billing_owner_id, admin);
    let listed: Value = http
        .get(format!("{base}/api/v1/tools"))
        .bearer_auth(&user_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(listed.to_string().contains("tools-fake-twin"));
    let snapshot = db
        .collection::<Document>(SERVICES)
        .find(doc! {"offering_kind":"tool"})
        .sort(doc! {"slug":1})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let ops = operations::list_all_endpoints(&db, &twin.id).await.unwrap();
    let (code, out, err) = run_cli(
        manager_token.clone(),
        vec!["admin".into(), "tools".into(), "apply".into()],
        file.clone(),
        base.clone(),
        cli.clone(),
    )
    .await;
    assert_eq!(code, 0, "reapply: {err}");
    assert!(out.contains("unchanged"));
    assert_eq!(
        snapshot,
        db.collection::<Document>(SERVICES)
            .find(doc! {"offering_kind":"tool"})
            .sort(doc! {"slug":1})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
    );
    assert_eq!(
        ops.iter()
            .map(|e| e.operation_generation)
            .collect::<Vec<_>>(),
        operations::list_all_endpoints(&db, &twin.id)
            .await
            .unwrap()
            .iter()
            .map(|e| e.operation_generation)
            .collect::<Vec<_>>()
    );
    // A real WebSocket upgrade is denied for a paused GET operation before
    // any vendor handshake. Keep it outside the manifest's repeat-apply proof.
    let added = http
        .post(format!("{base}/api/v1/services/{}/endpoints", twin.id))
        .bearer_auth(&manager_token)
        .json(&json!({"name":"events","method":"GET","path":"/events"}))
        .send()
        .await
        .unwrap();
    assert!(added.status().is_success());
    for (actor, state_name) in [(&admin_token, "published"), (&manager_token, "paused")] {
        let changed = http
            .post(format!("{base}/api/v1/services/{}/publication", twin.id))
            .bearer_auth(actor)
            .json(&json!({"state":state_name,"endpoint_names":["events"]}))
            .send()
            .await
            .unwrap();
        assert!(changed.status().is_success());
    }
    let denied = http
        .get(format!("{base}/api/v1/proxy/s/tools-fake-twin/events"))
        .bearer_auth(&user_token)
        .header("Connection", "upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 404);
    assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    let paused = http
        .post(format!("{base}/api/v1/services/{}/publication", twin.id))
        .bearer_auth(&manager_token)
        .json(&json!({"state":"paused","endpoint_names":["search"]}))
        .send()
        .await
        .unwrap();
    assert!(paused.status().is_success());
    let denied = http
        .post(format!("{base}/api/v1/proxy/s/{byok_slug}/v2/search"))
        .bearer_auth(&byok_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 404);
    assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    let count = vendor
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v2/search")
        .count();
    let usage = db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    let denied = http
        .post(format!("{base}/api/v1/proxy/s/tools-fake-twin/v2/search"))
        .bearer_auth(&user_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 404);
    assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    let denied = http.post(format!("{base}/mcp"))
        .bearer_auth(&user_token).header("Mcp-Session-Id", &mcp_session).header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tools-fake-twin__search","arguments":{"query":"local"}}}))
        .send().await.unwrap().text().await.unwrap();
    assert!(denied.contains("12700"), "MCP paused operation: {denied}");
    assert_eq!(
        count,
        vendor
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/v2/search")
            .count()
    );
    assert_eq!(
        usage,
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap()
    );
    let changed = http
        .post(format!("{base}/api/v1/services/{}/publication", twin.id))
        .bearer_auth(&admin_token)
        .json(&json!({"state":"published","endpoint_names":["search"]}))
        .send()
        .await
        .unwrap();
    assert!(changed.status().is_success());
    let restricted = http.put(format!("{base}/api/v1/services/{}",twin.id))
        .bearer_auth(&admin_token).json(&json!({"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[]}}))
        .send().await.unwrap();
    assert!(restricted.status().is_success());
    let denied = http
        .post(format!("{base}/api/v1/proxy/s/tools-fake-twin/v2/search"))
        .bearer_auth(&user_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    assert!(
        !denied.status().is_success(),
        "audience removal applies to the next request"
    );
    let listed: Value = http
        .get(format!("{base}/api/v1/tools"))
        .bearer_auth(&user_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!listed.to_string().contains("tools-fake-twin"));
    assert_eq!(
        usage,
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap()
    );
    assert_eq!(
        count,
        vendor
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/v2/search")
            .count()
    );
    // Restricted-empty audience denies the platform lane while the published
    // operation remains available through the second person's own key.
    let called = http
        .post(format!("{base}/api/v1/proxy/s/{byok_slug}/v2/search"))
        .bearer_auth(&byok_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    assert_eq!(called.status(), 200);
    let row = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(row) = db
                .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                .find_one(doc! {"actor_user_id":&byok_user,"credential_class":"user_owned"})
                .await
                .unwrap()
                && row.funding.as_ref().is_some_and(|f| f.settled)
            {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("second person's BYOK usage settles");
    assert_eq!(row.billing_owner_id, byok_user);
    assert_eq!(
        row.credential_class,
        crate::models::usage_meter::CredentialClass::UserOwned
    );
    assert_eq!(row.lago_metric_code, "platform_svc_tools-fake-twin_byok");
    assert_eq!(
        row.funding.unwrap().wallet_charge_credits,
        Some(crate::models::credits::Credits::from_pico(1).unwrap())
    );
    assert_ne!(row.billing_owner_id, admin);
    assert_ne!(row.billing_owner_id, manager);
    assert_ne!(row.billing_owner_id, user);
    let requests = vendor.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r
                .headers
                .get("authorization")
                .is_some_and(|h| h == "Bearer fake-user-byok-local-only"))
            .count(),
        1
    );
    let exposure_hits = vendor.received_requests().await.unwrap().len();
    let exposure_usage = db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    // A renamed authoritative spec withdraws the old published contract in
    // both credential lanes, without dispatch or new metering.
    let withdrawal_spec = Arc::new(std::sync::RwLock::new(
        json!({"openapi":"3.0.3","info":{"title":"Local withdrawal","version":"1"},"paths":{"/v2/search":{"post":{"operationId":"search","responses":{"200":{"description":"ok"}}}}}}),
    ));
    let served_spec = withdrawal_spec.clone();
    Mock::given(method("GET"))
        .and(path("/withdrawal-spec"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(served_spec.read().unwrap().clone())
        })
        .mount(&vendor)
        .await;
    let changed=http.put(format!("{base}/api/v1/services/{}",twin.id)).bearer_auth(&admin_token)
        .json(&json!({"openapi_spec_url":format!("{}/withdrawal-spec",vendor.uri()),"platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]}})).send().await.unwrap();
    assert!(changed.status().is_success());
    // Changing the spec URL demotes the stored operations. Publish the current
    // contract, then change the vendor response at the same URL so this explicit
    // authoritative discovery proves withdrawal of a live operation.
    let republished = http
        .post(format!("{base}/api/v1/services/{}/publication", twin.id))
        .bearer_auth(&admin_token)
        .json(&json!({"state":"published","endpoint_names":["search"]}))
        .send()
        .await
        .unwrap();
    assert!(republished.status().is_success());
    withdrawal_spec.write().unwrap()["paths"]["/v2/search"]["post"]["operationId"] =
        json!("renamed_search");
    let discovered = http
        .post(format!(
            "{base}/api/v1/services/{}/discover-endpoints",
            twin.id
        ))
        .bearer_auth(&manager_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(discovered.status().is_success());
    let discovered: Value = discovered.json().await.unwrap();
    assert!(
        discovered["withdrawn"]
            .as_array()
            .unwrap()
            .contains(&json!("search"))
    );
    for (slug, bearer) in [(twin.slug.as_str(), &user_token), (byok_slug, &byok_token)] {
        let denied = http
            .post(format!("{base}/api/v1/proxy/s/{slug}/v2/search"))
            .bearer_auth(bearer)
            .json(&json!({"query":"local"}))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 404);
        assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    }
    let denied=http.post(format!("{base}/mcp")).bearer_auth(&user_token).header("Mcp-Session-Id",&mcp_session).header("Accept","application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tools-fake-twin__search","arguments":{"query":"local"}}})).send().await.unwrap().text().await.unwrap();
    assert!(
        denied.contains("12700"),
        "withdrawn MCP operation: {denied}"
    );
    // Deleting a paused narrow blocker must demote the published broad route.
    let separate = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"slug":"tools-fake-new"})
        .await
        .unwrap()
        .unwrap();
    let broad=http.post(format!("{base}/api/v1/services/{}/endpoints",separate.id)).bearer_auth(&manager_token)
        .json(&json!({"name":"broad_search","method":"POST","path":"/v2/{operation}","parameters":[{"name":"operation","in":"path","required":true,"schema":{"type":"string"}}]})).send().await.unwrap();
    assert!(broad.status().is_success());
    let changed = http
        .post(format!(
            "{base}/api/v1/services/{}/publication",
            separate.id
        ))
        .bearer_auth(&admin_token)
        .json(&json!({"state":"published","endpoint_names":["broad_search"]}))
        .send()
        .await
        .unwrap();
    assert!(changed.status().is_success());
    let paused = http
        .post(format!(
            "{base}/api/v1/services/{}/publication",
            separate.id
        ))
        .bearer_auth(&manager_token)
        .json(&json!({"state":"paused","endpoint_names":["search"]}))
        .send()
        .await
        .unwrap();
    assert!(paused.status().is_success());
    let search = operations::list_all_endpoints(&db, &separate.id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.name == "search")
        .unwrap();
    let deleted = http
        .delete(format!(
            "{base}/api/v1/services/{}/endpoints/{}",
            separate.id, search.id
        ))
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap();
    assert!(deleted.status().is_success());
    let denied = http
        .post(format!("{base}/api/v1/proxy/s/{}/v2/search", separate.slug))
        .bearer_auth(&user_token)
        .json(&json!({"query":"local"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 404);
    assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 12700);
    let rows = operations::list_all_endpoints(&db, &separate.id)
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .find(|row| row.name == "broad_search")
            .unwrap()
            .publication,
        crate::models::service_endpoint::PublicationState::Draft
    );
    assert_eq!(
        vendor.received_requests().await.unwrap().len(),
        exposure_hits + 1,
        "only explicit discovery fetched the loopback spec"
    );
    assert_eq!(
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        exposure_usage
    );
    let audits = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let audit_text = format!("{audits:?}");
    assert!(!audit_text.contains("fake-tool-credential-local-only"));
    assert!(!audit_text.contains("fake-user-byok-local-only"));
    println!(
        "LOCAL_E2E_EVIDENCE: role grant, twin/new creation, shared and second-person BYOK injection, discovery and authoritative withdrawal, delete demotion without broader-route exposure, manager publication/replacement denial, Admin setup/publication, exact personal charges=0.000000000001 on platform/BYOK lanes, draft/paused ingress denial without usage/vendor calls, restricted-empty platform denial with BYOK success, write-free CLI reapply, redacted audit: PASS"
    );
    server.abort();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_icons_and_operation_fences_recheck_live_authority() {
    let db = connect_test_database("tools_manager_icons_fences")
        .await
        .expect("local replica set");
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let foreign = seed(&db, "operator", true, false).await;
    let tool = create(&state, &manager, new("tools-icons")).await.unwrap();
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><path d=\"M1 1L23 23\" /></svg>";
    let result = super::catalog_icons::upload_tool(
        State(state.clone()),
        test_auth_user(&manager),
        Path(tool.id.clone()),
        Json(super::catalog_icons::Upload { svg: svg.into() }),
    )
    .await
    .unwrap();
    assert!(result.0.icon_url.contains("/catalog-icons/"));
    assert!(
        super::catalog_icons::upload_tool(
            State(state.clone()),
            test_auth_user(&foreign),
            Path(tool.id.clone()),
            Json(super::catalog_icons::Upload { svg: svg.into() })
        )
        .await
        .is_err()
    );
    assert!(
        super::catalog_icons::upload_tool(
            State(state.clone()),
            test_auth_user(&manager),
            Path(tool.id.clone()),
            Json(super::catalog_icons::Upload {
                svg: "<svg viewBox=\"0 0 24 24\"><script>alert(1)</script></svg>".into()
            })
        )
        .await
        .is_err()
    );
    let observed = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    let role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"tools_manager"})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&manager}, doc! {"$pull":{"role_ids":&role.id}})
        .await
        .unwrap();
    let actor =
        crate::services::audit_service::AuditActor::from_auth_user(&test_auth_user(&manager));
    assert!(
        crate::services::tools_manager_service::commit_manager_update(
            &db,
            &observed,
            &manager,
            &doc! {"icon_url":"replacement"},
            state.audit_chain_hmac_key.as_ref().as_ref(),
            &actor
        )
        .await
        .is_err()
    );
    let input = operations::EndpointInput {
        name: "search".into(),
        description: None,
        method: "GET".into(),
        path: "/search".into(),
        parameters: None,
        request_body_schema: None,
        request_content_type: None,
        request_body_required: false,
        response_description: None,
        response: Default::default(),
        risk: None,
        supports_idempotency_key: false,
        data_scope: None,
        cost_class: None,
        execution: Default::default(),
        async_operation: None,
        target_id: None,
    };
    let authority = crate::services::catalog_mutation_service::Authority {
        actor: actor.clone(),
        first_party_human: true,
        admin_required: false,
        catalog_base_url: state.config.base_url.clone(),
    };
    assert!(
        operations::commit_mutation(
            &db,
            &observed,
            Some(&authority),
            operations::Mutation::Create(input),
            state.audit_chain_hmac_key.as_ref().as_ref(),
        )
        .await
        .is_err()
    );
    assert!(
        operations::list_all_endpoints(&db, &tool.id)
            .await
            .unwrap()
            .is_empty()
    );
    let live = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.icon_url, observed.icon_url);
    let audits: Vec<Document> = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        audits
            .iter()
            .any(|a| a.get_str("event_type").ok() == Some("tools_manager_updated"))
    );
    assert!(!serde_json::to_string(&audits).unwrap().contains(svg));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_no_auth_restricted_audience_is_live() {
    let db = connect_test_database("tools_manager_no_auth")
        .await
        .expect("local replica set");
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let person = seed(&db, "user", false, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let mut body = new("tools-no-auth");
    body["auth_method"] = "none".into();
    let tool = create(&state, &manager, body).await.unwrap();
    let mut service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        crate::services::platform_key_service::require(&db, &service, &person)
            .await
            .is_err()
    );
    let _ = update(
        &state,
        &admin,
        &tool.id,
        json!({"platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]}}),
    )
    .await
    .unwrap();
    service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    crate::services::platform_key_service::require(&db, &service, &person)
        .await
        .unwrap();
    let endpoint = crate::test_utils::test_user_endpoint(
        &Uuid::new_v4().to_string(),
        &person,
        "Local no-auth Tool",
        &service.base_url,
        None,
        Some(&tool.id),
    );
    let mut connection = crate::test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &person,
        &service.slug,
        &endpoint.id,
        Some(&tool.id),
        None,
    );
    connection.credential_binding = Some("platform".into());
    db.collection::<crate::models::user_endpoint::UserEndpoint>(
        crate::models::user_endpoint::COLLECTION_NAME,
    )
    .insert_one(&endpoint)
    .await
    .unwrap();
    db.collection::<crate::models::user_service::UserService>(
        crate::models::user_service::COLLECTION_NAME,
    )
    .insert_one(&connection)
    .await
    .unwrap();
    let resolve_connection = || {
        crate::services::proxy_service::resolve_proxy_target_from_user_service(
            &db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &person,
            Some(&connection.slug),
            None,
            crate::services::proxy_service::ProxyExecutionContext::new(
                None,
                state.platform_user_rate_limit,
            ),
        )
    };
    for binding in ["platform", "user", ""] {
        db.collection::<Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(doc! {"_id":&connection.id}, doc! {"$set":{"credential_binding": if binding.is_empty() {mongodb::bson::Bson::Null} else {binding.into()}}})
            .await.unwrap();
        let resolution = resolve_connection().await.unwrap().unwrap();
        assert_eq!(resolution.target.auth_method, "none");
        assert!(resolution.target.credential.is_empty());
        assert!(!resolution.master_credential);
    }
    let _ = update(
        &state,
        &admin,
        &tool.id,
        json!({"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[]}}),
    )
    .await
    .unwrap();
    service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    for binding in ["platform", "user", ""] {
        db.collection::<Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(doc! {"_id":&connection.id}, doc! {"$set":{"credential_binding": if binding.is_empty() {mongodb::bson::Bson::Null} else {binding.into()}}})
            .await.unwrap();
        assert!(resolve_connection().await.is_err());
    }
    assert!(
        crate::services::proxy_service::resolve_catalog_platform_target(
            &db,
            &state.encryption_keys,
            &person,
            service,
            state.platform_user_rate_limit
        )
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_seed_preserves_custom_role_collision() {
    let db = connect_test_database("tools_manager_role_collision")
        .await
        .expect("local replica set");
    let roles = db.collection::<Role>(ROLES);
    roles
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"slug":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let custom = Role {
        id: Uuid::new_v4().to_string(),
        name: "Custom".into(),
        slug: "tools_manager".into(),
        description: None,
        permissions: vec!["custom:read".into()],
        is_default: false,
        is_system: false,
        client_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    roles.insert_one(&custom).await.unwrap();
    role_service::seed_system_roles(&db).await.unwrap();
    let (first, second) = tokio::join!(
        role_service::seed_system_roles(&db),
        role_service::seed_system_roles(&db)
    );
    first.unwrap();
    second.unwrap();
    let live = roles
        .find_one(doc! {"_id":&custom.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!live.is_system);
    assert_eq!(live.permissions, custom.permissions);
    assert_eq!(
        roles
            .count_documents(doc! {"slug":"tools_manager"})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_authoritative_discovery_withdraws_and_delete_demotes() {
    use crate::models::service_endpoint::{
        COLLECTION_NAME as ENDPOINTS, PublicationState, ServiceEndpoint,
    };
    use crate::services::{
        audit_service::AuditActor, proxy_authorization::CanonicalPath,
        tool_publication_service as publication,
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let db = connect_test_database("tools_discovery_withdrawal")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let vendor = MockServer::start().await;
    for discoverer in [&manager, &admin] {
        let tool = create(
            &state,
            &manager,
            new(&format!("tools-withdraw-{}", Uuid::new_v4())),
        )
        .await
        .unwrap();
        db.collection::<Document>(SERVICES)
            .update_one(
                doc! {"_id": &tool.id},
                doc! {"$set": {"openapi_spec_url": format!("{}/spec", vendor.uri())}},
            )
            .await
            .unwrap();
        let spec = |narrow: bool, rename: bool| {
            let mut paths = json!({"/items/{id}": {"get": {"operationId": if rename {"renamed"} else {"broad"}, "responses": {"200": {"description": "ok"}}}}});
            if narrow {
                paths["/items/private"] = json!({"get": {"operationId":"narrow", "responses": {"200": {"description":"ok"}}}});
            }
            json!({"openapi":"3.0.0", "info":{"title":"local","version":"1"}, "paths":paths})
        };
        vendor.reset().await;
        Mock::given(method("GET"))
            .and(path("/spec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(spec(true, false)))
            .mount(&vendor)
            .await;
        let discovered = endpoints::discover_endpoints(
            State(state.clone()),
            test_auth_user(discoverer),
            Path(tool.id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(discovered.endpoints.len(), 2);
        let ids = discovered
            .endpoints
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let actor = AuditActor::from_auth_user(&test_auth_user(&admin));
        change_test_publication(
            &db,
            &tool.id,
            tool.offering_kind,
            &ids,
            PublicationState::Published,
            &actor,
            state.audit_chain_hmac_key.as_ref().as_ref(),
        )
        .await
        .unwrap();
        let before = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
        let audits = db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
        let no_op = endpoints::discover_endpoints(
            State(state.clone()),
            test_auth_user(discoverer),
            Path(tool.id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert!(no_op.withdrawn.is_empty());
        let after = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert_eq!(
            audits,
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap()
        );
        vendor.reset().await;
        Mock::given(method("GET"))
            .and(path("/spec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(spec(false, true)))
            .mount(&vendor)
            .await;
        let result = endpoints::discover_endpoints(
            State(state.clone()),
            test_auth_user(discoverer),
            Path(tool.id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert!(result.withdrawn.contains(&"narrow".into()));
        assert!(result.withdrawn.contains(&"broad".into()));
        let rows = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
        for old in &before {
            let row = rows.iter().find(|r| r.id == old.id).unwrap();
            assert_eq!(row.operation_generation, old.operation_generation + 1);
            assert_eq!(row.publication, PublicationState::Draft);
            assert!(!row.is_active);
        }
        assert!(matches!(
            publication::require_published_operation(
                &rows,
                "GET",
                &CanonicalPath::from_rest_decoded("/items/private").unwrap()
            ),
            Err(AppError::ToolOperationNotPublished)
        ));
        assert!(
            operations::list_endpoints(&db, &tool.id)
                .await
                .unwrap()
                .is_empty()
        );
        // Delete a narrow unpublished blocker below an otherwise published route.
        let broad = rows.iter().find(|r| r.name == "renamed").unwrap();
        let narrow = rows.iter().find(|r| r.name == "narrow").unwrap();
        change_test_publication(
            &db,
            &tool.id,
            tool.offering_kind,
            std::slice::from_ref(&broad.id),
            PublicationState::Published,
            &actor,
            state.audit_chain_hmac_key.as_ref().as_ref(),
        )
        .await
        .unwrap();
        let _ = endpoints::delete_endpoint(
            State(state.clone()),
            test_auth_user(&admin),
            Path((tool.id.clone(), narrow.id.clone())),
        )
        .await
        .unwrap();
        let broad = db
            .collection::<ServiceEndpoint>(ENDPOINTS)
            .find_one(doc! {"_id": &broad.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(broad.publication, PublicationState::Draft);
        assert!(!broad.is_active);
        assert!(matches!(
            publication::gate(
                &db,
                &db.collection::<DownstreamService>(SERVICES)
                    .find_one(doc! {"_id": &tool.id})
                    .await
                    .unwrap()
                    .unwrap(),
                "GET",
                &CanonicalPath::from_rest_decoded("/items/private").unwrap()
            )
            .await,
            Err(AppError::ToolOperationNotPublished)
        ));
    }
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"tool_operation_deleted","seq":{"$exists":true}})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"tool_operation_withdrawn","seq":{"$exists":true}})
            .await
            .unwrap(),
        4
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_icon_assets_eligibility_and_request_audit() {
    let db = connect_test_database("tools_asset_eligibility")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let tool = create(&state, &manager, new("tools-asset")).await.unwrap();
    assert!(tool.tools_manager_eligible);
    let missing = format!(
        "{}/catalog-icons/{}",
        state.config.base_url.trim_end_matches('/'),
        Uuid::new_v4()
    );
    let mut body = new("tools-fabricated");
    body["icon_url"] = missing.clone().into();
    assert!(matches!(
        create(&state, &manager, body).await,
        Err(AppError::ValidationError(_))
    ));
    let mut auth = test_auth_user(&manager);
    auth.ip_address = Some("127.0.0.42".into());
    auth.user_agent = Some("local-audit-test".into());
    let icon = super::catalog_icons::upload_tool(
        State(state.clone()),
        auth.clone(),
        Path(tool.id.clone()),
        Json(super::catalog_icons::Upload {
            svg: "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"/>".into(),
        }),
    )
    .await
    .unwrap()
    .0
    .icon_url;
    for actor in [&manager, &admin] {
        assert!(matches!(
            update(&state, actor, &tool.id, json!({"icon_url":missing})).await,
            Err(AppError::ValidationError(_))
        ));
        assert_eq!(
            update(&state, actor, &tool.id, json!({"icon_url":icon}))
                .await
                .unwrap()
                .icon_url,
            Some(icon.clone())
        );
    }
    for actor in [&manager, &admin] {
        assert_eq!(
            update(&state, actor, &tool.id, json!({"icon_url":""}))
                .await
                .unwrap()
                .icon_url,
            None
        );
        assert_eq!(
            update(&state, actor, &tool.id, json!({"icon_url":icon}))
                .await
                .unwrap()
                .icon_url,
            Some(icon.clone())
        );
    }
    let operation = endpoints::create_endpoint(
        State(state.clone()),
        auth.clone(),
        Path(tool.id.clone()),
        Json(serde_json::from_value(json!({"name":"read","method":"GET","path":"/read"})).unwrap()),
    )
    .await
    .unwrap()
    .0;
    let mut admin_auth = test_auth_user(&admin);
    admin_auth.ip_address = auth.ip_address.clone();
    admin_auth.user_agent = auth.user_agent.clone();
    let _ = endpoints::change_publication(
        State(state.clone()),
        admin_auth.clone(),
        Path((tool.id.clone(), operation.id.clone())),
        Json(endpoints::PublicationRequest {
            state: crate::models::service_endpoint::PublicationState::Published,
        }),
    )
    .await
    .unwrap();
    let _ = endpoints::update_endpoint(
        State(state.clone()),
        admin_auth,
        Path((tool.id.clone(), operation.id)),
        Json(serde_json::from_value(json!({"path":"/changed"})).unwrap()),
    )
    .await
    .unwrap();
    for (event, actor) in [
        ("tools_manager_updated", &manager),
        ("tools_manager_operations_updated", &manager),
        ("tool_contract_publication_demoted", &admin),
        ("tools_manager_operations_updated", &admin),
    ] {
        assert!(db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc!{"event_type":event,"user_id":actor,"ip_address":"127.0.0.42","user_agent":"local-audit-test","seq":{"$exists":true}}).await.unwrap()>0,"{event}");
    }
    db.collection::<Document>(SERVICES).update_one(doc!{"_id":&tool.id},doc!{"$set":{"token_exchange_config":{"endpoint":"https://vendor.example/token","request_encoding":"json","request_template":{"client_id":"hidden-id","client_secret":"hidden-secret"},"token_response_path":"access_token","default_ttl_secs":300,"injection":"bearer","credential_fields":[]}}}).await.unwrap();
    let row = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!crate::services::tools_manager_service::eligible_manager_tool(&row));
    let response = super::services_helpers::service_to_response_with_viewer(None, row, None).await;
    // The presence/eligibility response must not disclose config material.
    let serialized = serde_json::to_string(&response).unwrap();
    assert!(!serialized.contains("hidden-secret"));
    assert!(!serialized.contains("token_exchange_config"));
    assert!(serialized.contains("\"tools_manager_eligible\":false"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_twin_create_retry_binds_intent_not_attempt_clock() {
    let db = connect_test_database("tools_twin_replay").await.unwrap();
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let source=create(&state,&admin,json!({"name":"Source","slug":"source-replay","base_url":"https://vendor.example/api","auth_method":"none"})).await.unwrap();
    let _ = endpoints::create_endpoint(
        State(state.clone()),
        test_auth_user(&admin),
        Path(source.id.clone()),
        Json(serde_json::from_value(json!({"name":"read","method":"GET","path":"/read"})).unwrap()),
    )
    .await
    .unwrap();
    for actor in [&manager, &admin] {
        let body = json!({"name":"Twin","slug":format!("twin-{actor}"),"twin_of_service_id":source.id,"skills_request_id":Uuid::new_v4().to_string()});
        let first = create(&state, actor, body.clone()).await.unwrap();
        let endpoints = operations::list_all_endpoints(&db, &first.id)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let replay = create(&state, actor, body.clone()).await.unwrap();
        assert_eq!(first.id, replay.id);
        assert_eq!(
            serde_json::to_value(&endpoints).unwrap(),
            serde_json::to_value(
                operations::list_all_endpoints(&db, &first.id)
                    .await
                    .unwrap()
            )
            .unwrap()
        );
        let mut changed = body;
        changed["name"] = "Different".into();
        assert!(matches!(
            create(&state, actor, changed).await,
            Err(AppError::Conflict(_))
        ));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_publication_fences_admin_legacy_kind_and_revocation() {
    use crate::{
        models::{downstream_service::OfferingKind, service_endpoint::PublicationState},
        services::audit_service::AuditActor,
    };
    let db = connect_test_database("tools_publication_live_parent")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", true, false).await;
    let mut body = new("tools-admin-publication");
    body["auth_method"] = "none".into();
    let tool = create(&state, &admin, body).await.unwrap();
    let op = endpoints::create_endpoint(
        State(state.clone()),
        test_auth_user(&admin),
        Path(tool.id.clone()),
        Json(serde_json::from_value(json!({"name":"read","method":"GET","path":"/read"})).unwrap()),
    )
    .await
    .unwrap()
    .0;
    for (stored, expected) in [
        ("ai_service", OfferingKind::Tool),
        ("tool", OfferingKind::AiService),
    ] {
        db.collection::<Document>(SERVICES)
            .update_one(
                doc! {"_id":&tool.id},
                doc! {"$set":{"offering_kind":stored}},
            )
            .await
            .unwrap();
        assert!(matches!(
            change_test_publication(
                &db,
                &tool.id,
                expected,
                std::slice::from_ref(&op.id),
                PublicationState::Published,
                &AuditActor::from_auth_user(&test_auth_user(&admin)),
                state.audit_chain_hmac_key.as_ref().as_ref()
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
    assert!(
        change_test_publication(
            &db,
            &tool.id,
            OfferingKind::Tool,
            std::slice::from_ref(&op.id),
            PublicationState::Validated,
            &AuditActor::from_auth_user(&test_auth_user(&manager)),
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await
        .is_err()
    );
    let role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"admin"})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&admin}, doc! {"$pull":{"role_ids":role.id}})
        .await
        .unwrap();
    assert!(
        change_test_publication(
            &db,
            &tool.id,
            OfferingKind::Tool,
            &[op.id],
            PublicationState::Published,
            &AuditActor::from_auth_user(&test_auth_user(&admin)),
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await
        .is_err()
    );
    assert!(
        operations::list_endpoints(&db, &tool.id)
            .await
            .unwrap()
            .is_empty()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_transfer_requires_live_human_admin_and_binds_receipt() {
    use crate::services::{
        ownership_transfer_access as access,
        ownership_transfer_service::{self as transfers, ResourceKind, TransferCommand},
    };
    let db = connect_test_database("tools_transfer_authority")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, false).await;
    let admin = seed(&db, "admin", false, false).await;
    let destination = seed(&db, "user", false, false).await;
    let tool = create(&state, &manager, new("tools-transfer"))
        .await
        .unwrap();
    for (actor, key, human) in [
        (&manager, None, true),
        (&manager, Some("owner-agent-key"), false),
        (&admin, None, false),
        (&admin, Some("admin-agent-key"), false),
    ] {
        assert!(
            access::require_resource_access(
                &db,
                actor,
                key,
                human,
                ResourceKind::Service,
                &tool.id
            )
            .await
            .is_err()
        );
        assert!(
            transfers::preview_with_agent(
                &db,
                actor,
                key,
                human,
                ResourceKind::Service,
                &tool.id,
                &destination,
                5
            )
            .await
            .is_err()
        );
    }
    let preview = transfers::preview_with_agent(
        &db,
        &admin,
        None,
        true,
        ResourceKind::Service,
        &tool.id,
        &destination,
        5,
    )
    .await
    .unwrap();
    let request = Uuid::new_v4().to_string();
    let command = |human| TransferCommand {
        actor: &admin,
        api_key_id: None,
        first_party_human: human,
        kind: ResourceKind::Service,
        resource_id: &tool.id,
        destination: &destination,
        request_id: &request,
        expected_version: &preview.version,
        capacity: 5,
    };
    assert!(transfers::transfer(&db, command(false)).await.is_err());
    let receipt = transfers::transfer(&db, command(true)).await.unwrap();
    assert!(receipt.tool_transfer);
    assert_eq!(
        transfers::transfer(&db, command(true)).await.unwrap().id,
        receipt.id
    );
    assert!(transfers::transfer(&db, command(false)).await.is_err());
    db.collection::<Document>(SERVICES)
        .delete_one(doc! {"_id":&tool.id})
        .await
        .unwrap();
    assert_eq!(
        transfers::transfer(&db, command(true)).await.unwrap().id,
        receipt.id
    );
    let admin_role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"admin"})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&admin},
            doc! {"$pull":{"role_ids":admin_role.id}},
        )
        .await
        .unwrap();
    assert!(transfers::transfer(&db, command(true)).await.is_err());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_authority_fences_avoid_system_hotspots_and_reject_revocation() {
    use crate::models::group::{COLLECTION_NAME as GROUPS, Group};
    use crate::services::tools_manager_service::fence_authority;
    let db = connect_test_database("tools_permission_fences")
        .await
        .unwrap();
    let manager = seed(&db, "operator", true, false).await;
    let system_roles: Vec<Document> = db
        .collection::<Document>(ROLES)
        .find(doc! {"is_system":true})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"tools_manager"})
        .await
        .unwrap()
        .unwrap();
    role.id = Uuid::new_v4().to_string();
    role.slug = "custom-tools".into();
    role.is_system = false;
    db.collection::<Role>(ROLES)
        .insert_one(&role)
        .await
        .unwrap();
    let group = Group {
        id: Uuid::new_v4().to_string(),
        name: "Tools editors".into(),
        slug: "tools-editors".into(),
        description: None,
        role_ids: vec![role.id.clone()],
        parent_group_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let mut unrelated = group.clone();
    unrelated.id = Uuid::new_v4().to_string();
    unrelated.slug = "unrelated".into();
    unrelated.role_ids.clear();
    db.collection::<Group>(GROUPS)
        .insert_many([&group, &unrelated])
        .await
        .unwrap();
    let operator = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"operator"})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&manager},
            doc! {"$set":{"role_ids":[&operator.id],"group_ids":[&group.id,&unrelated.id]}},
        )
        .await
        .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(!fence_authority(&db, &mut session, &manager).await.unwrap());
    session.commit_transaction().await.unwrap();
    assert_eq!(
        db.collection::<Document>(ROLES)
            .find(doc! {"is_system":true})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
        system_roles
    );
    assert!(
        db.collection::<Document>(GROUPS)
            .find_one(doc! {"_id":&unrelated.id})
            .await
            .unwrap()
            .unwrap()
            .get("authority_fence")
            .is_none()
    );
    for (collection, id, change) in [
        (ROLES, &role.id, doc! {"$set":{"permissions":[]}}),
        (GROUPS, &group.id, doc! {"$set":{"role_ids":[]}}),
        (USERS, &manager, doc! {"$set":{"role_ids":[]}}),
    ] {
        db.collection::<Document>(ROLES)
            .update_one(
                doc! {"_id":&role.id},
                doc! {"$set":{"permissions":&role.permissions}},
            )
            .await
            .unwrap();
        db.collection::<Document>(GROUPS)
            .update_one(
                doc! {"_id":&group.id},
                doc! {"$set":{"role_ids":[&role.id]}},
            )
            .await
            .unwrap();
        db.collection::<Document>(USERS)
            .update_one(
                doc! {"_id":&manager},
                doc! {"$set":{"role_ids":[&operator.id]}},
            )
            .await
            .unwrap();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        db.collection::<Document>(USERS)
            .find_one(doc! {"_id":&manager})
            .session(&mut session)
            .await
            .unwrap();
        db.collection::<Document>(collection)
            .update_one(doc! {"_id":id}, change)
            .await
            .unwrap();
        assert!(
            fence_authority(&db, &mut session, &manager).await.is_err(),
            "stale {collection} permission must conflict"
        );
        let _ = session.abort_transaction().await;
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_parent_projection_allows_billing_sync_but_binds_contract() {
    use crate::services::{audit_service::AuditActor, catalog_mutation_service as mutations};
    let db = connect_test_database("catalog_projection").await.unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let response=create(&state,&admin,json!({"name":"Projection","slug":"projection","base_url":"https://vendor.example","auth_method":"none"})).await.unwrap();
    let observed = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&response.id})
        .await
        .unwrap()
        .unwrap();
    let authority = mutations::Authority {
        catalog_base_url: state.config.base_url.clone(),
        actor: AuditActor::from_auth_user(&test_auth_user(&admin)),
        first_party_human: true,
        admin_required: true,
    };
    db.collection::<Document>(SERVICES).update_one(doc! {"_id":&observed.id},doc! {"$set":{"billing":bson::to_bson(&crate::models::service_billing::ServiceBilling {platform_pricing:Some(crate::models::service_billing::ServicePlatformPricing {credits_per_unit:"1".into(),lago_metric_code:"projection".into(),sync_status:crate::models::service_billing::PricingSyncStatus::Failed,sync_error:Some("loopback sync fixture".into())}),..Default::default()}).unwrap(),"updated_at":bson::DateTime::now()}}).await.unwrap();
    let updated = mutations::commit_update(
        &db,
        &observed,
        &authority,
        &doc! {"description":"edited"},
        &doc! {"_id":&observed.id},
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    assert_eq!(updated.description.as_deref(), Some("edited"));
    assert_eq!(
        updated
            .billing
            .as_ref()
            .unwrap()
            .platform_pricing
            .as_ref()
            .unwrap()
            .sync_status,
        crate::models::service_billing::PricingSyncStatus::Failed
    );
    for drift in [
        doc! {"base_url":"https://changed.example"},
        doc! {"owner_user_id":Uuid::new_v4().to_string()},
        doc! {"offering_kind":"tool"},
    ] {
        let current = db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&observed.id})
            .await
            .unwrap()
            .unwrap();
        db.collection::<Document>(SERVICES)
            .update_one(doc! {"_id":&observed.id}, doc! {"$set":drift})
            .await
            .unwrap();
        assert!(matches!(
            mutations::commit_update(
                &db,
                &current,
                &authority,
                &doc! {"description":"stale"},
                &doc! {"_id":&observed.id},
                state.audit_chain_hmac_key.as_ref().as_ref()
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_operation_description_demotes_one_and_route_demotes_all() {
    use crate::models::service_endpoint::PublicationState;
    let db = connect_test_database("tool_demotion_scope").await.unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let mut body = new("demotion-scope");
    body["auth_method"] = "none".into();
    let tool = create(&state, &admin, body).await.unwrap();
    let parent = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    let authority = crate::services::catalog_mutation_service::Authority {
        catalog_base_url: state.config.base_url.clone(),
        actor: crate::services::audit_service::AuditActor::from_auth_user(&test_auth_user(&admin)),
        first_party_human: true,
        admin_required: true,
    };
    for (name, path) in [("read", "/read"), ("second", "/second")] {
        let mut input = race_operation();
        input.name = name.into();
        input.path = path.into();
        operations::commit_mutation(
            &db,
            &parent,
            Some(&authority),
            operations::Mutation::Create(input),
            state.audit_chain_hmac_key.as_ref().as_ref(),
        )
        .await
        .unwrap();
    }
    let mut rows = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    for row in &rows {
        let _ = endpoints::change_publication(
            State(state.clone()),
            test_auth_user(&admin),
            Path((tool.id.clone(), row.id.clone())),
            Json(endpoints::PublicationRequest {
                state: PublicationState::Published,
            }),
        )
        .await
        .unwrap();
    }
    let before = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
    operations::commit_mutation(
        &db,
        &parent,
        Some(&authority),
        operations::Mutation::Update(
            rows[0].id.clone(),
            operations::EndpointUpdate {
                description: Some(Some("better docs".into())),
                ..Default::default()
            },
        ),
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    let after = operations::list_all_endpoints(&db, &tool.id).await.unwrap();
    let untouched = after.iter().find(|r| r.id == rows[1].id).unwrap();
    let original = before.iter().find(|r| r.id == untouched.id).unwrap();
    assert_eq!(untouched.publication, PublicationState::Published);
    assert_eq!(
        untouched.operation_generation,
        original.operation_generation
    );
    assert_eq!(untouched.updated_at, original.updated_at);
    let changed = after.iter().find(|r| r.id == rows[0].id).unwrap();
    assert_eq!(changed.publication, PublicationState::Draft);
    let parent_before = db
        .collection::<Document>(SERVICES)
        .find_one(doc! {"_id":&tool.id})
        .await
        .unwrap()
        .unwrap();
    let user_before = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id":&admin})
        .await
        .unwrap()
        .unwrap();
    let repeated = operations::commit_mutation(
        &db,
        &parent,
        Some(&authority),
        operations::Mutation::Update(
            rows[0].id.clone(),
            operations::EndpointUpdate {
                description: Some(Some("better docs".into())),
                ..Default::default()
            },
        ),
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        db.collection::<Document>(SERVICES)
            .find_one(doc! {"_id":&tool.id})
            .await
            .unwrap()
            .unwrap(),
        parent_before
    );
    assert_eq!(
        db.collection::<Document>(USERS)
            .find_one(doc! {"_id":&admin})
            .await
            .unwrap()
            .unwrap(),
        user_before
    );
    for row in &repeated {
        let original = after.iter().find(|old| old.id == row.id).unwrap();
        assert_eq!(row.updated_at, original.updated_at);
        assert_eq!(row.operation_generation, original.operation_generation);
    }

    let _ = endpoints::change_publication(
        State(state.clone()),
        test_auth_user(&admin),
        Path((tool.id.clone(), changed.id.clone())),
        Json(endpoints::PublicationRequest {
            state: PublicationState::Published,
        }),
    )
    .await
    .unwrap();
    operations::commit_mutation(
        &db,
        &parent,
        Some(&authority),
        operations::Mutation::Update(
            changed.id.clone(),
            operations::EndpointUpdate {
                path: Some("/moved".into()),
                ..Default::default()
            },
        ),
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    assert!(
        operations::list_all_endpoints(&db, &tool.id)
            .await
            .unwrap()
            .iter()
            .all(|row| row.publication == PublicationState::Draft && !row.is_active)
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_background_spec_kind_drift_creates_no_exposure() {
    let db = connect_test_database("background_kind_fence")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let response=create(&state,&admin,json!({"name":"Background","slug":"background-race","base_url":"https://vendor.example","auth_method":"none"})).await.unwrap();
    let observed = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&response.id})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&observed.id},
            doc! {"$set":{"offering_kind":"tool"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        operations::commit_mutation(
            &db,
            &observed,
            None,
            operations::Mutation::Reconcile {
                inputs: vec![race_operation()],
                authoritative: false
            },
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert!(
        operations::list_all_endpoints(&db, &observed.id)
            .await
            .unwrap()
            .is_empty()
    );
    let tool = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&observed.id})
        .await
        .unwrap()
        .unwrap();
    operations::commit_mutation(
        &db,
        &tool,
        None,
        operations::Mutation::Reconcile {
            inputs: vec![race_operation()],
            authoritative: false,
        },
        state.audit_chain_hmac_key.as_ref().as_ref(),
    )
    .await
    .unwrap();
    assert!(
        operations::list_all_endpoints(&db, &observed.id)
            .await
            .unwrap()
            .iter()
            .all(|row| !row.is_active)
    );
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&observed.id},
            doc! {"$set":{"offering_kind":"ai_service"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        operations::commit_mutation(
            &db,
            &tool,
            None,
            operations::Mutation::Reconcile {
                inputs: vec![race_operation()],
                authoritative: false
            },
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_inactive_views_are_role_scoped_and_eligibility_is_batched() {
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig};
    use axum::extract::Query;
    let db = connect_test_database("catalog_inactive_views")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let catalog = seed(&db, "operator", false, true).await;
    let manager = seed(&db, "operator", true, false).await;
    let ordinary = seed(&db, "user", false, false).await;
    let own = super::providers::create_provider(
        State(state.clone()),
        test_auth_user(&catalog),
        Json(
            serde_json::from_value(
                json!({"name":"Own","slug":"own-inactive","provider_type":"api_key"}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap()
    .0;
    let foreign = super::providers::create_provider(
        State(state.clone()),
        test_auth_user(&admin),
        Json(
            serde_json::from_value(
                json!({"name":"Foreign","slug":"foreign-inactive","provider_type":"api_key"}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap()
    .0;
    db.collection::<ProviderConfig>(PROVIDERS)
        .update_many(
            doc! {"_id":{"$in":[&own.id,&foreign.id]}},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    for (actor, expected) in [
        (&catalog, vec![own.id.clone()]),
        (&admin, vec![own.id.clone(), foreign.id.clone()]),
    ] {
        let result = super::providers::list_providers(
            State(state.clone()),
            test_auth_user(actor),
            Query(super::providers::ListProvidersQuery {
                include_inactive: Some(true),
            }),
        )
        .await
        .unwrap()
        .0;
        let ids: Vec<_> = result.providers.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids.len(), expected.len());
        assert!(expected.iter().all(|id| ids.contains(id)));
    }
    for actor in [&ordinary, &manager] {
        assert!(
            super::providers::list_providers(
                State(state.clone()),
                test_auth_user(actor),
                Query(super::providers::ListProvidersQuery {
                    include_inactive: Some(true)
                })
            )
            .await
            .is_err()
        );
    }
    let ai=create(&state,&catalog,json!({"name":"Own AI","slug":"own-ai-inactive","base_url":"https://vendor.example","auth_method":"bearer"})).await.unwrap();
    let foreign_ai=create(&state,&admin,json!({"name":"Foreign AI","slug":"foreign-ai-inactive","base_url":"https://vendor.example","auth_method":"none"})).await.unwrap();
    let tool = create(&state, &manager, new("inactive-tool"))
        .await
        .unwrap();
    db.collection::<Document>(SERVICES)
        .update_many(
            doc! {"_id":{"$in":[&ai.id,&foreign_ai.id,&tool.id]}},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    for (actor, expected) in [
        (&catalog, vec![ai.id.clone()]),
        (&manager, vec![tool.id.clone()]),
        (
            &admin,
            vec![ai.id.clone(), foreign_ai.id.clone(), tool.id.clone()],
        ),
    ] {
        let result = services::list_services(
            State(state.clone()),
            test_auth_user(actor),
            Query(services::ListServicesQuery {
                include_inactive: Some(true),
                category: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(result.services.len(), expected.len());
        assert!(
            expected
                .iter()
                .all(|id| result.services.iter().any(|row| &row.id == id))
        );
    }
    assert!(
        services::list_services(
            State(state.clone()),
            test_auth_user(&ordinary),
            Query(services::ListServicesQuery {
                include_inactive: Some(true),
                category: None
            })
        )
        .await
        .is_err()
    );
    let rows = db
        .collection::<DownstreamService>(SERVICES)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let blocked = create(&state, &manager, new("requirement-legacy-tool"))
        .await
        .unwrap();
    db.collection::<Document>(crate::models::service_provider_requirement::COLLECTION_NAME).insert_one(doc! {"_id":Uuid::new_v4().to_string(),"service_id":&blocked.id,"provider_config_id":&own.id}).await.unwrap();
    let mut rows = rows;
    rows.push(
        db.collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&blocked.id})
            .await
            .unwrap()
            .unwrap(),
    );
    let batch = crate::services::tools_manager_service::eligible_manager_tool_ids(&db, &rows)
        .await
        .unwrap();
    for row in &rows {
        assert_eq!(
            batch.contains(&row.id),
            crate::services::tools_manager_service::eligible_manager_tool_in_db(&db, row, None)
                .await
                .unwrap()
        );
    }
    assert!(!batch.contains(&blocked.id));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_provider_and_identity_alternate_paths_bind_live_parent_and_admin() {
    use crate::models::service_provider_requirement::ServiceProviderRequirement;
    use crate::services::{catalog_mutation_service as mutations, provider_link_service as links};
    let db = connect_test_database("tool_provider_identity_fences")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let admin = seed(&db, "admin", false, false).await;
    let catalog = seed(&db, "operator", false, true).await;
    let provider = super::providers::create_provider(
        State(state.clone()),
        test_auth_user(&catalog),
        Json(
            serde_json::from_value(
                json!({"name":"Loopback","slug":"loopback","provider_type":"api_key"}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap()
    .0;
    let response=create(&state,&catalog,json!({"name":"Link race","slug":"link-race","base_url":"https://vendor.example","auth_method":"bearer"})).await.unwrap();
    let observed = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&response.id})
        .await
        .unwrap()
        .unwrap();
    let authority = mutations::Authority {
        catalog_base_url: state.config.base_url.clone(),
        actor: crate::services::audit_service::AuditActor::from_auth_user(&test_auth_user(
            &catalog,
        )),
        first_party_human: true,
        admin_required: false,
    };
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&observed.id},
            doc! {"$set":{"offering_kind":"tool"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        links::link_with_authority(
            &db,
            &provider.id,
            &observed.id,
            None,
            Some((&observed, &authority))
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let tool = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&observed.id})
        .await
        .unwrap()
        .unwrap();
    assert!(tool.provider_config_id.is_none());
    let mut new_tool = tool.clone();
    new_tool.id = Uuid::new_v4().to_string();
    new_tool.slug = "new-linked-tool".into();
    assert!(
        links::link_with_authority(&db, &provider.id, &new_tool.id, Some(&new_tool), None)
            .await
            .is_err()
    );
    assert!(
        db.collection::<Document>(SERVICES)
            .find_one(doc! {"_id":&new_tool.id})
            .await
            .unwrap()
            .is_none()
    );
    let response=create(&state,&admin,json!({"name":"Requirement race","slug":"requirement-race","base_url":"https://vendor.example","auth_method":"none"})).await.unwrap();
    let observed = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&response.id})
        .await
        .unwrap()
        .unwrap();
    let authority = mutations::Authority {
        actor: crate::services::audit_service::AuditActor::from_auth_user(&test_auth_user(&admin)),
        admin_required: true,
        ..authority
    };
    let req = ServiceProviderRequirement {
        id: Uuid::new_v4().to_string(),
        service_id: observed.id.clone(),
        provider_config_id: provider.id.clone(),
        required: true,
        scopes: None,
        injection_method: "bearer".into(),
        injection_key: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&observed.id},
            doc! {"$set":{"offering_kind":"tool"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        links::add_requirement_with_authority(&db, &req, &observed, &authority).await,
        Err(AppError::Conflict(_))
    ));
    let live = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&observed.id})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        links::add_requirement_with_authority(&db, &req, &live, &authority).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        db.collection::<Document>(crate::models::service_provider_requirement::COLLECTION_NAME)
            .count_documents(doc! {"service_id":&observed.id})
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        crate::services::catalog_identity_service::force_resync(
            &db,
            &authority,
            &observed,
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&observed.id},
            doc! {"$set":{"offering_kind":"ai_service"}},
        )
        .await
        .unwrap();
    links::add_requirement_with_authority(&db, &req, &observed, &authority)
        .await
        .unwrap();
    assert!(matches!(
        mutations::commit_update(
            &db,
            &observed,
            &authority,
            &doc! {"offering_kind":"tool"},
            &doc! {"_id":&observed.id},
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"admin"})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&admin}, doc! {"$pull":{"role_ids":role.id}})
        .await
        .unwrap();
    assert!(
        links::remove_requirement_with_authority(&db, &req.id, &observed, &authority)
            .await
            .is_err()
    );
    assert!(
        crate::services::catalog_identity_service::force_resync(
            &db,
            &authority,
            &observed,
            state.audit_chain_hmac_key.as_ref().as_ref()
        )
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_shared_catalog_commits_bind_live_permission_kind_and_owner() {
    use crate::models::{downstream_service::OfferingKind, service_endpoint::PublicationState};
    use crate::services::{
        audit_service::AuditActor, catalog_mutation_service as mutations, tool_publication_service,
    };
    let db = connect_test_database("tools_shared_catalog_commit")
        .await
        .unwrap();
    let admin = seed(&db, "admin", false, false).await;
    let manager = seed(&db, "operator", false, true).await;
    let legacy = seed(&db, "operator", false, false).await;
    let mut parent = crate::models::downstream_service::test_helpers::dummy_service();
    parent.id = Uuid::new_v4().to_string();
    parent.created_by = "system".into();
    parent.offering_kind = OfferingKind::AiService;
    parent.service_category = "provider".into();
    parent.service_type = "http".into();
    parent.visibility = "public".into();
    parent.owner_user_id = None;
    parent.catalog_manager_created = false;
    parent.credential_encrypted.clear();
    parent.platform_key = None;
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&parent)
        .await
        .unwrap();
    let authority = mutations::Authority {
        catalog_base_url: "http://localhost:3001".into(),
        actor: AuditActor::from_auth_user(&test_auth_user(&manager)),
        first_party_human: true,
        admin_required: false,
    };
    let key = [2_u8; 32];
    let fields = doc! {"description":"completed shared connector"};
    mutations::commit_update(
        &db,
        &parent,
        &authority,
        &fields,
        &doc! {"_id":&parent.id},
        &key,
    )
    .await
    .unwrap();
    let (rows, _) = operations::commit_mutation(
        &db,
        &parent,
        Some(&authority),
        operations::Mutation::Create(race_operation()),
        &key,
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    tool_publication_service::change_publication(
        &db,
        &parent,
        &authority,
        &[rows[0].id.clone()],
        PublicationState::Paused,
        &key,
    )
    .await
    .unwrap();
    // Both drift directions fail before writes; the pending operation never
    // gains Tool exposure under a Catalog request's legacy authority.
    for (stored, expected) in [
        ("tool", OfferingKind::AiService),
        ("ai_service", OfferingKind::Tool),
    ] {
        db.collection::<Document>(SERVICES)
            .update_one(
                doc! {"_id":&parent.id},
                doc! {"$set":{"offering_kind":stored}},
            )
            .await
            .unwrap();
        let mut observed = parent.clone();
        observed.offering_kind = expected;
        assert!(matches!(
            mutations::commit_update(
                &db,
                &observed,
                &authority,
                &fields,
                &doc! {"_id":&parent.id},
                &key
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            operations::commit_mutation(
                &db,
                &observed,
                Some(&authority),
                operations::Mutation::Create(race_operation()),
                &key
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            tool_publication_service::change_publication(
                &db,
                &observed,
                &authority,
                &[rows[0].id.clone()],
                PublicationState::Published,
                &key
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&parent.id},
            doc! {"$set":{"owner_user_id":&admin}},
        )
        .await
        .unwrap();
    assert!(matches!(
        mutations::commit_update(
            &db,
            &parent,
            &authority,
            &fields,
            &doc! {"_id":&parent.id},
            &key
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&parent.id},
            doc! {"$set":{"owner_user_id":bson::Bson::Null}},
        )
        .await
        .unwrap();
    assert!(matches!(
        mutations::commit_update(
            &db,
            &parent,
            &authority,
            &doc! {"is_active":false},
            &doc! {"_id":&parent.id},
            &key
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&parent.id},
            doc! {"$set":{"platform_key":{"enabled":true,"audience":"public"}}},
        )
        .await
        .unwrap();
    let credential_parent = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id":&parent.id})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        mutations::commit_update(
            &db,
            &credential_parent,
            &authority,
            &doc! {"base_url":"https://other.example/api"},
            &doc! {"_id":&parent.id},
            &key
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    db.collection::<Document>(SERVICES)
        .update_one(doc! {"_id":&parent.id}, doc! {"$unset":{"platform_key":""}})
        .await
        .unwrap();
    let catalog_role = db
        .collection::<Role>(ROLES)
        .find_one(doc! {"slug":"catalog_manager"})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&manager},
            doc! {"$pull":{"role_ids":catalog_role.id}},
        )
        .await
        .unwrap();
    assert!(matches!(
        mutations::commit_update(
            &db,
            &parent,
            &authority,
            &fields,
            &doc! {"_id":&parent.id},
            &key
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        operations::commit_mutation(
            &db,
            &parent,
            Some(&authority),
            operations::Mutation::Delete(rows[0].id.clone()),
            &key
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        tool_publication_service::change_publication(
            &db,
            &parent,
            &authority,
            &[rows[0].id.clone()],
            PublicationState::Published,
            &key
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    // Legacy creators without the Catalog role keep their non-Tool writes.
    parent.created_by = legacy.clone();
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&parent.id},
            doc! {"$set":{"created_by":&legacy}},
        )
        .await
        .unwrap();
    let legacy_authority = mutations::Authority {
        actor: AuditActor::from_auth_user(&test_auth_user(&legacy)),
        ..authority
    };
    mutations::commit_update(
        &db,
        &parent,
        &legacy_authority,
        &fields,
        &doc! {"_id":&parent.id},
        &key,
    )
    .await
    .unwrap();
    assert_eq!(
        operations::list_all_endpoints(&db, &parent.id)
            .await
            .unwrap()
            .len(),
        1
    );
    // Manager-created private rows retain their legacy creator field rules.
    parent.catalog_manager_created = true;
    parent.visibility = "private".into();
    parent.credential_encrypted = vec![1, 2, 3];
    db.collection::<DownstreamService>(SERVICES)
        .replace_one(doc! {"_id":&parent.id}, &parent)
        .await
        .unwrap();
    mutations::commit_update(
        &db,
        &parent,
        &legacy_authority,
        &doc! {"base_url":"https://legacy.example/api"},
        &doc! {"_id":&parent.id},
        &key,
    )
    .await
    .unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn catalog_manager_shared_discovery_rejects_nonhuman_before_vendor_requests() {
    use crate::mw::auth::AuthMethod;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let db = connect_test_database("catalog_shared_first_party_ingress")
        .await
        .unwrap();
    let manager = seed(&db, "operator", false, true).await;
    let admin = seed(&db, "admin", false, false).await;
    let state = test_app_state(db.clone());
    let vendor = MockServer::start().await;
    Mock::given(method("GET")).and(path("/spec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"openapi":"3.0.3","info":{"title":"Shared","version":"1"},"paths":{"/read":{"get":{"operationId":"read","responses":{"200":{"description":"ok"}}}}}})))
        .mount(&vendor).await;
    let mut parent = crate::models::downstream_service::test_helpers::dummy_service();
    parent.id = Uuid::new_v4().to_string();
    parent.created_by = "system".into();
    parent.base_url = vendor.uri();
    parent.openapi_spec_url = Some(format!("{}/spec", vendor.uri()));
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&parent)
        .await
        .unwrap();
    for auth_method in [
        AuthMethod::ApiKey,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
        AuthMethod::Relay,
        AuthMethod::AccessToken,
    ] {
        let mut auth = test_auth_user(&manager);
        auth.auth_method = auth_method;
        if auth.auth_method == AuthMethod::AccessToken {
            auth.oauth_client_id = Some(Uuid::new_v4().to_string());
        }
        assert!(
            endpoints::discover_endpoints(State(state.clone()), auth, Path(parent.id.clone()))
                .await
                .is_err()
        );
        assert!(vendor.received_requests().await.unwrap().is_empty());
    }
    assert_eq!(
        db.collection::<Document>(crate::models::service_endpoint::COLLECTION_NAME)
            .count_documents(doc! {"service_id":&parent.id})
            .await
            .unwrap(),
        0
    );
    let mut human = test_auth_user(&manager);
    human.auth_method = AuthMethod::AccessToken;
    let _ = endpoints::discover_endpoints(State(state.clone()), human, Path(parent.id.clone()))
        .await
        .unwrap();
    assert_eq!(vendor.received_requests().await.unwrap().len(), 1);
    db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&parent.id},
            doc! {"$set":{"catalog_manager_created":true}},
        )
        .await
        .unwrap();
    let mut oauth_admin = test_auth_user(&admin);
    oauth_admin.auth_method = AuthMethod::AccessToken;
    oauth_admin.oauth_client_id = Some(Uuid::new_v4().to_string());
    assert!(
        endpoints::discover_endpoints(State(state.clone()), oauth_admin, Path(parent.id.clone()))
            .await
            .is_err()
    );
    assert_eq!(vendor.received_requests().await.unwrap().len(), 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tools_manager_endpoint_readers_keep_published_only_identity_fallback() {
    use crate::{
        models::service_endpoint::{PublicationState, ServiceEndpoint},
        mw::auth::AuthMethod,
    };
    use axum::response::IntoResponse;
    let db = connect_test_database("catalog_endpoint_read_capability")
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let manager = seed(&db, "operator", true, true).await;
    let admin = seed(&db, "admin", false, false).await;
    let tool = create(&state, &manager, new("reader-tool")).await.unwrap();
    let mut shared = crate::models::downstream_service::test_helpers::dummy_service();
    shared.id = Uuid::new_v4().to_string();
    shared.slug = "reader-shared".into();
    shared.created_by = "system".into();
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&shared)
        .await
        .unwrap();
    for service_id in [&tool.id, &shared.id] {
        for (name, publication) in [
            ("published", PublicationState::Published),
            ("draft", PublicationState::Draft),
            ("paused", PublicationState::Paused),
        ] {
            let mut row = crate::test_utils::test_service_endpoint(
                service_id,
                name,
                "GET",
                &format!("/{name}"),
            );
            row.publication = publication;
            row.is_active = publication == PublicationState::Published;
            db.collection::<ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME)
                .insert_one(row)
                .await
                .unwrap();
        }
        // Use an Admin's identity too: an Agent Key never inherits the human
        // writer's access to draft/paused rows. SA identities have no User row.
        for actor in [&manager, &admin] {
            for kind in [
                "agent_key",
                "agent_key_login",
                "delegated",
                "service_account",
                "oauth_app",
            ] {
                let mut auth = test_auth_user(actor);
                match kind {
                    "agent_key" | "agent_key_login" => {
                        auth.auth_method = AuthMethod::ApiKey;
                        auth.api_key_id = Some(Uuid::new_v4().to_string());
                        auth.api_key_credential_id =
                            (kind == "agent_key_login").then(|| Uuid::new_v4().to_string());
                    }
                    "delegated" => {
                        auth.auth_method = AuthMethod::Delegated;
                        auth.scope = "account:read".into();
                        auth.acting_client_id = Some(Uuid::new_v4().to_string());
                    }
                    "service_account" => {
                        auth.auth_method = AuthMethod::ServiceAccount;
                        auth.user_id = Uuid::new_v4();
                    }
                    "oauth_app" => {
                        auth.auth_method = AuthMethod::AccessToken;
                        auth.oauth_client_id = Some(Uuid::new_v4().to_string());
                    }
                    _ => unreachable!(),
                }
                let tools = super::tools::list(State(state.clone()), auth.clone())
                    .await
                    .unwrap();
                assert_eq!(tools.into_response().status(), 200, "{kind} Tool listing");
                let listed =
                    endpoints::list_endpoints(State(state.clone()), auth, Path(service_id.clone()))
                        .await
                        .unwrap();
                assert_eq!(listed.endpoints.len(), 1, "{kind}");
                assert_eq!(listed.endpoints[0].name, "published", "{kind}");
                assert_eq!(listed.into_response().status(), 200, "{kind}");
            }
        }
        let listed = endpoints::list_endpoints(
            State(state.clone()),
            test_auth_user(&manager),
            Path(service_id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(
            listed.endpoints.len(),
            3,
            "human writer sees drafts and pauses"
        );
        let missing_actor = test_auth_user(&Uuid::new_v4().to_string());
        assert_eq!(
            endpoints::list_endpoints(
                State(state.clone()),
                missing_actor,
                Path(service_id.clone())
            )
            .await
            .unwrap()
            .endpoints
            .len(),
            1
        );
    }
    let denied = super::admin_ownership::authorization(
        State(state.clone()),
        test_auth_user(&manager),
        Path((
            crate::services::ownership_transfer_service::ResourceKind::Service,
            tool.id.clone(),
        )),
    )
    .await
    .unwrap();
    assert!(!denied.can_transfer);
    assert_eq!(denied.into_response().status(), 200);
    // A malformed persisted actor produces a database error rather than a
    // successful listing with a negative capability.
    db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&manager}, doc! {"$set":{"role_ids":123}})
        .await
        .unwrap();
    assert!(matches!(
        endpoints::list_endpoints(
            State(state.clone()),
            test_auth_user(&manager),
            Path(shared.id)
        )
        .await,
        Err(AppError::DatabaseError(_))
    ));
    assert!(matches!(
        super::tools::list(State(state.clone()), test_auth_user(&manager)).await,
        Err(AppError::DatabaseError(_))
    ));
    db.drop().await.unwrap();
}
