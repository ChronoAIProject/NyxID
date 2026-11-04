use super::*;
use crate::{
    errors::AppError,
    services::{
        billing::{BillingIngress, route_inventory::BillingRoutePolicy},
        invocation_diagnostic_service::tests::{fixture, mount_healthy},
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
async fn invocation_diagnostic_preserves_raw_errors_and_authoritative_broker_errors() {
    for code in [401, 403, 404, 503, 504] {
        let (state, actor, id, server) = fixture().await;
        mount_healthy(&server, &id).await;
        let mut request = request();
        let invocation = HttpInvocation::start(&state, &mut request);
        Target::record_request(&request, Some(&id));
        let original = Response::builder()
            .status(code)
            .header("content-type", "application/problem+json")
            .header("retry-after", "7")
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
        let evidence: Value = serde_json::from_str(
            response.headers()["x-nyxid-service-diagnostic"]
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(evidence["assessment"], "operation_specific_likely");
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "byte-identical original body"
        );
    }
    let (state, actor, id, server) = fixture().await;
    mount_healthy(&server, &id).await;
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
    .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::to_value(AppError::NodeProxyTimeout.response_body()).unwrap()
    );
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
            "success" => Ok(Response::new(Body::from("ok"))),
            "denied" => Err(AppError::Forbidden("local denial".into())),
            "cancelled" => {
                invocation.context.cancellation.cancel();
                Err(AppError::NodeProxyTimeout)
            }
            _ => {
                auth.acting_client_id = Some("chrono-probe".into());
                Err(AppError::NodeProxyTimeout)
            }
        };
        let response = HttpInvocation::finish(Some(invocation), &state, &auth, original).await;
        if kind == "success" {
            assert!(response.is_ok());
        } else {
            assert!(response.is_err());
        }
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invocation_diagnostic_disconnect_aborts_in_flight_http_work() {
    let (state, actor, id, server) = fixture().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(2)))
        .expect(1)
        .mount(&server)
        .await;
    let mut request = request();
    let invocation = HttpInvocation::start(&state, &mut request).unwrap();
    Target::record_request(&request, Some(&id));
    let cancellation = invocation.context.cancellation.clone();
    let original = Response::builder()
        .status(403)
        .body(Body::from("original operation error"))
        .unwrap();
    let auth = auth(&actor);
    let finished = HttpInvocation::finish(Some(invocation), &state, &auth, Ok(original));
    let disconnect = async {
        tokio::time::timeout(std::time::Duration::from_millis(700), async {
            loop {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("diagnostic HTTP request must start before disconnect");
        cancellation.cancel();
    };
    let started = std::time::Instant::now();
    let (result, ()) = tokio::join!(finished, disconnect);
    assert!(started.elapsed() < std::time::Duration::from_millis(750));
    let response = result.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        !response
            .headers()
            .contains_key("x-nyxid-service-diagnostic")
    );
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        "original operation error"
    );
}

#[tokio::test]
async fn invocation_diagnostic_rest_proxy_uses_the_actual_selected_instance() {
    for code in [401, 403, 404] {
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
        mount_healthy(&server, &id).await;
        let mut request = request();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
        let response = Box::pin(super::super::proxy::proxy_request(
            State(state),
            auth(&actor),
            TelemetryContext::default(),
            Path((id, "invalid-operation".into())),
            request,
        ))
        .await
        .unwrap();
        assert_eq!(response.status().as_u16(), code);
        assert!(
            response
                .headers()
                .contains_key("x-nyxid-service-diagnostic")
        );
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "original operation error"
        );
    }
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
                ResponseTemplate::new(403).set_body_json(
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
        mount_healthy(&server, &id).await;
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
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            response
                .headers()
                .contains_key("x-nyxid-service-diagnostic")
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("original operation permission error"));
    }
}
