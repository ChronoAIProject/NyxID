//! Workload-only credential delivery; human/agent management authentication
//! does not authorize this endpoint. Its provider ticket is independently
//! verified and every response is recipient-sealed and non-cacheable.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, header},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    services::remote_machine_bootstrap as bootstrap,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRequest {
    binding: nyxid_machine::managed::Binding,
    ticket: serde_json::Value,
    recipient_public_key: String,
}

#[derive(Serialize)]
pub struct BootstrapResponse {
    binding: nyxid_machine::managed::Binding,
    writer_admitted: bool,
    sealed: nyxid_machine::bootstrap::Envelope,
    server_url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    node_id: String,
    pod_uid: String,
    ticket: serde_json::Value,
    recipient_public_key: String,
}

#[derive(Serialize)]
pub struct PrepareResponse {
    binding: nyxid_machine::managed::Binding,
}

pub async fn prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PrepareRequest>,
) -> AppResult<(HeaderMap, Json<PrepareResponse>)> {
    require_workload_auth_only(&headers)?;
    let binding = bootstrap::prepare_workload(
        &state.db,
        state.managed_bootstrap_verifier.as_ref(),
        &request.node_id,
        &request.pod_uid,
        &request.ticket,
        &request.recipient_public_key,
    )
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((headers, Json(PrepareResponse { binding })))
}

fn require_workload_auth_only(headers: &HeaderMap) -> AppResult<()> {
    if headers.contains_key(header::AUTHORIZATION)
        || headers.contains_key("x-api-key")
        || headers.contains_key(header::COOKIE)
    {
        return Err(AppError::Forbidden(
            "managed bootstrap requires workload authentication only".into(),
        ));
    }
    Ok(())
}

fn node_websocket_url(base_url: &str) -> AppResult<String> {
    let invalid_config =
        || AppError::Internal("managed node endpoint configuration is invalid".into());
    let mut url = reqwest::Url::parse(base_url).map_err(|_| invalid_config())?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => return Err(invalid_config()),
    };
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_config());
    }
    url.set_scheme(scheme).map_err(|_| invalid_config())?;
    let path = format!("{}/api/v1/nodes/ws", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url.into())
}

pub async fn bootstrap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BootstrapRequest>,
) -> AppResult<(HeaderMap, Json<BootstrapResponse>)> {
    require_workload_auth_only(&headers)?;
    // Reject deployment URL mistakes before activating a credential delivery.
    let server_url = node_websocket_url(&state.config.base_url)?;
    let result = bootstrap::bootstrap(
        &state.db,
        &state.encryption_keys,
        state.managed_bootstrap_verifier.as_ref(),
        bootstrap::BootstrapInput {
            binding: request.binding,
            ticket: request.ticket,
            recipient_public_key: request.recipient_public_key,
        },
    )
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((
        headers,
        Json(BootstrapResponse {
            binding: result.binding,
            writer_admitted: true,
            sealed: result.sealed,
            server_url,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_returns_the_actual_node_websocket_route() {
        for (base, expected) in [
            (
                "https://nyxid.example.test",
                "wss://nyxid.example.test/api/v1/nodes/ws",
            ),
            (
                "https://nyxid.example.test/",
                "wss://nyxid.example.test/api/v1/nodes/ws",
            ),
            (
                "http://localhost:3001",
                "ws://localhost:3001/api/v1/nodes/ws",
            ),
            (
                "https://nyxid.example.test/nyxid",
                "wss://nyxid.example.test/nyxid/api/v1/nodes/ws",
            ),
        ] {
            assert_eq!(node_websocket_url(base).unwrap(), expected);
        }
        for base in [
            "garbage",
            "file:///tmp/nyxid",
            "https://user:password@nyxid.example.test",
            "https://nyxid.example.test/?token=secret",
            "https://nyxid.example.test/#token",
        ] {
            assert!(node_websocket_url(base).is_err());
        }
    }

    #[test]
    fn user_and_agent_credentials_do_not_authorize_workload_bootstrap() {
        for (name, value) in [
            ("authorization", "Bearer delegated-token"),
            ("authorization", "Bearer service-account-token"),
            ("authorization", "Bearer owner-access-token"),
            ("x-api-key", "nyxid_sk_key"),
            ("cookie", "session=human-session"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static(value));
            assert!(require_workload_auth_only(&headers).is_err());
        }
        assert!(require_workload_auth_only(&HeaderMap::new()).is_ok());
    }

    #[tokio::test]
    async fn workload_route_rejects_user_credentials_and_unconfigured_provider() {
        use axum::{
            Router,
            body::Body,
            http::{Request, StatusCode},
            routing::post,
        };
        use tower::ServiceExt;
        let state = crate::test_utils::test_app_state_no_db().await;
        let app = Router::new()
            .route("/bootstrap", post(bootstrap))
            .with_state(state);
        let binding = nyxid_machine::managed::Binding {
            node_id: uuid::Uuid::new_v4().to_string(),
            sandbox_uid: uuid::Uuid::new_v4().to_string(),
            pod_uid: uuid::Uuid::new_v4().to_string(),
            runtime_generation: 1,
            credential_epoch: 1,
            writer_fence: uuid::Uuid::new_v4().to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            delivery_id: uuid::Uuid::new_v4().to_string(),
        };
        let key = nyxid_machine::bootstrap::generate_recipient().unwrap();
        let body = serde_json::json!({"binding": binding, "ticket":{"verified":true}, "recipient_public_key":key.public_key_b64u().unwrap()}).to_string();
        for (name, value) in [
            ("authorization", "Bearer delegated"),
            ("x-api-key", "nyxid_sk_key"),
            ("cookie", "session=owner"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/bootstrap")
                        .header("content-type", "application/json")
                        .header(name, value)
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let response = app
            .oneshot(
                Request::post("/bootstrap")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
