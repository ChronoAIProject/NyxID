use super::*;
use crate::{
    errors::AppError,
    services::{
        billing::{BillingIngress, route_inventory::BillingRoutePolicy},
        invocation_diagnostic_service::tests::{fixture, legacy_handle},
    },
    test_utils::{set_test_approval_preference, test_auth_user},
};
use axum::{
    body::to_bytes,
    extract::{Path, State},
    http::StatusCode,
};
use mongodb::bson::doc;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn auth(actor: &str) -> AuthUser {
    let mut auth = test_auth_user(actor);
    auth.auth_method = AuthMethod::AccessToken;
    auth.scope = "proxy account:read".into();
    auth
}

fn request() -> Request<Body> {
    Request::builder()
        .uri("/test")
        .header("authorization", "Bearer original-caller")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn proxy_only_agent_key_cannot_reach_redemption() {
    let (state, actor, id, server) = fixture().await;
    let key = crate::services::key_service::create_api_key(
        &state.db,
        &actor,
        "diagnostic agent",
        "proxy",
        None,
        None,
        None,
        None,
        Some(true),
        Some(false),
        Some(false),
        None,
        None,
        Some("codex"),
        None,
    )
    .await
    .unwrap();
    let issued = legacy_handle(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::ApiKey,
        Some(&key.id),
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("diagnostic handle must be issued");
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let response = private
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/monitoring/invocation-diagnostics")
                .header("content-type", "application/json")
                .header("x-api-key", &key.full_key)
                .body(Body::from(json!({"handle": issued.handle}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(server.received_requests().await.unwrap().is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn retained_llm_execution_preserves_typed_errors_for_delivery_classification() {
    let (state, _, _, _) = fixture().await;
    let mut request = request();
    request.extensions_mut().insert(RetainedLlmExecution);

    assert!(HttpInvocation::start(&state, &mut request).is_none());
}

#[tokio::test]
async fn invocation_diagnostic_preserves_raw_errors_and_authoritative_broker_errors() {
    for code in [500, 503, 504] {
        let (state, actor, id, server) = fixture().await;
        let mut request = request();
        let invocation = HttpInvocation::start(&state, &mut request);
        Target::record_request(&request, Some(&id));
        let original = Response::builder()
            .status(code)
            .header("content-type", "application/problem+json")
            .header("retry-after", "7")
            .header("cache-control", "private, max-age=0")
            .header(HANDLE_HEADER, "downstream-spoof")
            .body(Body::from("byte-identical original body"))
            .unwrap();
        let response = HttpInvocation::finish(invocation, &state, &auth(&actor), Ok(original))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), code);
        assert_eq!(response.headers()["retry-after"], "7");
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
        let handle = response.headers()[HANDLE_HEADER]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(handle.starts_with("nyx_diag_"));
        assert_ne!(handle, "downstream-spoof");
        assert!(!handle.contains(&id));
        assert_eq!(response.headers()[EXPIRES_HEADER], "60");
        assert_eq!(
            response
                .headers()
                .get_all("cache-control")
                .iter()
                .map(|v| v.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["private, max-age=0", "no-store"],
        );
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "byte-identical original body"
        );
        let row = state
            .db
            .collection::<crate::models::invocation_diagnostic::InvocationDiagnostic>(
                crate::models::invocation_diagnostic::COLLECTION_NAME,
            )
            .find_one(doc! {"handle_hash":crate::crypto::token::hash_token(&handle)})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.exact_user_service_id, id);
        assert_eq!(row.actor_user_id, actor);
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    let (state, actor, id, server) = fixture().await;
    let mut request = request();
    let invocation = HttpInvocation::start(&state, &mut request);
    Target::record_request(&request, Some(&id));
    let response = HttpInvocation::finish(
        invocation,
        &state,
        &auth(&actor),
        Err(AppError::NodeProxyTimeout),
    )
    .await
    .unwrap_err();
    assert_eq!(
        response.error_code(),
        AppError::NodeProxyTimeout.error_code()
    );
    assert!(
        matches!(response, AppError::InvocationDiagnostic { ref source, .. } if matches!(source.as_ref(), AppError::NodeProxyTimeout))
    );
    let response = response.into_response();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(response.headers().contains_key(HANDLE_HEADER));
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::to_value(AppError::NodeProxyTimeout.response_body()).unwrap()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invocation_diagnostic_success_denial_cancel_and_probe_callbacks_do_not_probe() {
    let (state, actor, id, server) = fixture().await;
    for kind in ["success", "denied", "cancelled", "probe_callback"] {
        let mut request = request();
        let invocation = HttpInvocation::start(&state, &mut request).unwrap();
        assert!(
            HttpInvocation::start(&state, &mut request).is_none(),
            "nested wrappers must not diagnose twice"
        );
        Target::record_request(&request, Some(&id));
        let mut auth = auth(&actor);
        let original = match kind {
            "success" => Ok(Response::builder()
                .header(HANDLE_HEADER, "downstream-spoof")
                .header(EXPIRES_HEADER, "999")
                .body(Body::from("ok"))
                .unwrap()),
            "denied" => Err(AppError::Forbidden("local denial".into())),
            "cancelled" => Err(AppError::ClientDisconnected),
            _ => {
                auth.acting_client_id = Some("chrono-probe".into());
                Err(AppError::NodeProxyTimeout)
            }
        };
        let response = HttpInvocation::finish(Some(invocation), &state, &auth, original).await;
        if kind == "success" {
            let response = response.unwrap();
            assert!(!response.headers().contains_key(HANDLE_HEADER));
            assert!(!response.headers().contains_key(EXPIRES_HEADER));
        } else {
            assert!(response.is_err());
        }
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invocation_diagnostic_failure_returns_without_probe_and_suppresses_duplicates() {
    let (state, actor, id, server) = fixture().await;
    let mut initial_request = request();
    let invocation = HttpInvocation::start(&state, &mut initial_request).unwrap();
    Target::record_request(&initial_request, Some(&id));
    let original = Response::builder()
        .status(503)
        .body(Body::from("original operation error"))
        .unwrap();
    let auth = auth(&actor);
    let started = std::time::Instant::now();
    let response = HttpInvocation::finish(Some(invocation), &state, &auth, Ok(original))
        .await
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(HANDLE_HEADER));
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        "original operation error"
    );

    let mut repeated_request = request();
    let repeated_invocation = HttpInvocation::start(&state, &mut repeated_request).unwrap();
    Target::record_request(&repeated_request, Some(&id));
    let repeated_original = Response::builder()
        .status(503)
        .body(Body::from("repeated original operation error"))
        .unwrap();
    let repeated = HttpInvocation::finish(
        Some(repeated_invocation),
        &state,
        &auth,
        Ok(repeated_original),
    )
    .await
    .unwrap();
    assert_eq!(repeated.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(!repeated.headers().contains_key(HANDLE_HEADER));
    assert!(!repeated.headers().contains_key(EXPIRES_HEADER));
    assert_eq!(
        to_bytes(repeated.into_body(), 4096).await.unwrap(),
        "repeated original operation error"
    );
    assert_eq!(
        state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::invocation_diagnostic::COLLECTION_NAME,
            )
            .count_documents(doc! {"actor_user_id":&actor,"exact_user_service_id":&id})
            .await
            .unwrap(),
        1
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn redemption_rechecks_current_execution_scope_before_handle_lookup() {
    let (state, actor, id, server) = fixture().await;
    let issued = crate::services::invocation_diagnostic_service::issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .expect("diagnostic handle must be issued");
    let mut restricted = auth(&actor);
    restricted.scope = "account:read".into();
    let error = redeem(
        State(state.clone()),
        restricted,
        TelemetryContext::default(),
        HeaderMap::from_iter([(
            axum::http::header::AUTHORIZATION,
            "Bearer narrower-caller".parse().unwrap(),
        )]),
        None,
        Json(RedeemRequest {
            handle: issued.handle,
        }),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invocation_diagnostic_rest_proxy_uses_the_actual_selected_instance() {
    for code in [500, 503, 504] {
        let (state, actor, id, server) = fixture().await;
        let downstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/invalid-operation"))
            .respond_with(ResponseTemplate::new(code).set_body_string("original operation error"))
            .expect(1)
            .mount(&downstream)
            .await;
        state
            .db
            .collection::<mongodb::bson::Document>("user_endpoints")
            .update_many(doc! {}, doc! {"$set":{"url":downstream.uri()}})
            .await
            .unwrap();
        set_test_approval_preference(&state.db, &actor, false).await;
        let mut request = request();
        *request.uri_mut() = format!("/api/v1/proxy/s/same-slug/invalid-operation?_nyxid_via={id}")
            .parse()
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
        let response = Box::pin(super::super::proxy::proxy_request_by_slug(
            State(state.clone()),
            auth(&actor),
            TelemetryContext::default(),
            Path(("same-slug".into(), "invalid-operation".into())),
            request,
        ))
        .await
        .unwrap();
        assert_eq!(response.status().as_u16(), code);
        assert!(response.headers().contains_key(HANDLE_HEADER));
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "original operation error"
        );
        let rows = state
            .db
            .collection::<crate::models::invocation_diagnostic::InvocationDiagnostic>(
                crate::models::invocation_diagnostic::COLLECTION_NAME,
            );
        assert_eq!(
            rows.find_one(doc! {"exact_user_service_id":&id})
                .await
                .unwrap()
                .unwrap()
                .exact_user_service_id,
            id
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn invocation_diagnostic_ignores_local_websocket_handshake_rejections() {
    let (state, actor, id, server) = fixture().await;
    let downstream = MockServer::start().await;
    state
        .db
        .collection::<mongodb::bson::Document>("user_endpoints")
        .update_many(doc! {}, doc! {"$set":{"url":downstream.uri()}})
        .await
        .unwrap();
    set_test_approval_preference(&state.db, &actor, false).await;
    let mut request = request();
    *request.uri_mut() = format!("/api/v1/proxy/s/same-slug/socket?_nyxid_via={id}")
        .parse()
        .unwrap();
    request
        .headers_mut()
        .insert("connection", "upgrade".parse().unwrap());
    request
        .headers_mut()
        .insert("upgrade", "websocket".parse().unwrap());
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));

    let response = Box::pin(super::super::proxy::proxy_request_by_slug(
        State(state.clone()),
        auth(&actor),
        TelemetryContext::default(),
        Path(("same-slug".into(), "socket".into())),
        request,
    ))
    .await
    .unwrap();

    assert!(response.status().is_client_error());
    assert!(!response.headers().contains_key(HANDLE_HEADER));
    assert_eq!(
        state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::invocation_diagnostic::COLLECTION_NAME,
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(downstream.received_requests().await.unwrap().is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invocation_diagnostic_llm_entries_keep_downstream_errors() {
    use crate::models::{downstream_service::DownstreamService, provider_config::ProviderConfig};
    for gateway in [false, true] {
        let (state, actor, id, server) = fixture().await;
        let downstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(
                    json!({"error":{"message":"original operation permission error"}}),
                ),
            )
            .expect(1)
            .mount(&downstream)
            .await;
        state
            .db
            .collection::<mongodb::bson::Document>("user_endpoints")
            .update_many(doc! {}, doc! {"$set":{"url":downstream.uri()}})
            .await
            .unwrap();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let provider: ProviderConfig = mongodb::bson::from_document(doc! {
            "_id": &provider_id,
            "slug": "openai",
            "name": "Test provider",
            "provider_type": "api_key",
            "is_active": true,
            "created_by": &actor,
            "created_at": mongodb::bson::DateTime::now(),
            "updated_at": mongodb::bson::DateTime::now(),
        })
        .unwrap();
        state
            .db
            .collection::<ProviderConfig>("provider_configs")
            .insert_one(provider)
            .await
            .unwrap();
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.provider_config_id = Some(provider_id);
        catalog.slug = "llm-openai".into();
        catalog.base_url = downstream.uri();
        catalog.requires_user_credential = false;
        state
            .db
            .collection::<DownstreamService>("downstream_services")
            .insert_one(&catalog)
            .await
            .unwrap();
        state
            .db
            .collection::<mongodb::bson::Document>("user_services")
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"catalog_service_id":&catalog.id}},
            )
            .await
            .unwrap();
        set_test_approval_preference(&state.db, &actor, false).await;
        let body = json!({"model":"gpt-4.1"}).to_string();
        let mut request = Request::builder()
            .method("GET")
            .uri("/api/v1/llm/models")
            .header("authorization", "Bearer original-caller")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(if gateway {
                BillingIngress::LlmGateway
            } else {
                BillingIngress::LlmProvider
            }));
        let response = if gateway {
            super::super::llm_gateway::gateway_request(
                State(state),
                auth(&actor),
                Path("models".into()),
                request,
            )
            .await
        } else {
            Box::pin(super::super::llm_gateway::llm_proxy_request(
                State(state),
                auth(&actor),
                Path(("openai".into(), "models".into())),
                request,
            ))
            .await
        }
        .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(response.headers().contains_key(HANDLE_HEADER));
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("original operation permission error"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn invocation_diagnostic_flag_off_and_non_delegable_callers_never_issue() {
    let (mut state, actor, id, server) = fixture().await;
    for (enabled, method) in [
        (false, AuthMethod::AccessToken),
        (true, AuthMethod::ApiKey),
        (true, AuthMethod::Session),
        (true, AuthMethod::ServiceAccount),
        (true, AuthMethod::Delegated),
        (true, AuthMethod::Relay),
    ] {
        state.config.invocation_diagnostics_enabled = enabled;
        let mut request = request();
        let invocation = HttpInvocation::start(&state, &mut request);
        if !enabled {
            assert!(invocation.is_none());
            assert!(request.extensions().get::<Target>().is_none());
        }
        Target::record_request(&request, Some(&id));
        let mut caller = auth(&actor);
        caller.auth_method = method.clone();
        let response = HttpInvocation::finish(
            invocation,
            &state,
            &caller,
            Ok(Response::builder()
                .status(503)
                .body(Body::from("original"))
                .unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!response.headers().contains_key(HANDLE_HEADER));
        assert!(!response.headers().contains_key(EXPIRES_HEADER));
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "original"
        );
        assert!(
            diagnostics::issue(&state.db, &state.config, &actor, &method, None, None, &id)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::invocation_diagnostic::COLLECTION_NAME
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn invocation_diagnostic_downstream_client_errors_do_not_issue() {
    let (state, actor, id, server) = fixture().await;
    for status in [400, 401, 403, 404, 422, 429] {
        let mut request = request();
        let invocation = HttpInvocation::start(&state, &mut request);
        Target::record_request(&request, Some(&id));
        let response = HttpInvocation::finish(
            invocation,
            &state,
            &auth(&actor),
            Ok(Response::builder()
                .status(status)
                .body(Body::from("original"))
                .unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert!(!response.headers().contains_key(HANDLE_HEADER));
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "original"
        );
    }
    assert_eq!(
        state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::invocation_diagnostic::COLLECTION_NAME
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn invocation_diagnostic_sender_constrained_bearer_is_not_forwarded() {
    let (state, actor, id, server) = fixture().await;
    let mut request = request();
    request
        .extensions_mut()
        .insert(crate::mw::auth::SenderConstrainedToken);
    assert!(HttpInvocation::start(&state, &mut request).is_none());
    let issued = diagnostics::issue(
        &state.db,
        &state.config,
        &actor,
        &AuthMethod::AccessToken,
        None,
        None,
        &id,
    )
    .await
    .unwrap()
    .unwrap();
    let response = redeem(
        State(state.clone()),
        auth(&actor),
        TelemetryContext::default(),
        request.headers().clone(),
        Some(axum::Extension(crate::mw::auth::SenderConstrainedToken)),
        Json(RedeemRequest {
            handle: issued.handle,
        }),
    )
    .await
    .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["outcome"], "non_delegable");
    assert_eq!(body["observation"], "unknown");
    assert!(server.received_requests().await.unwrap().is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn invocation_diagnostic_errors_retain_numeric_code_in_turn_trace() {
    let (state, actor, id, _) = fixture().await;
    let mut request = request();
    let invocation = HttpInvocation::start(&state, &mut request);
    Target::record_request(&request, Some(&id));
    let context = crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
    let (result, records) = crate::turn_trace::capture(crate::turn_trace::response(
        context,
        crate::turn_trace::Stage::Proxy,
        HttpInvocation::finish(
            invocation,
            &state,
            &auth(&actor),
            Err(AppError::NodeProxyTimeout),
        ),
    ))
    .await;
    assert!(result.is_err());
    let records = serde_json::to_value(records).unwrap();
    assert!(
        records
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["error_code"] == 8002),
        "{records}"
    );
    state.db.drop().await.unwrap();
}
