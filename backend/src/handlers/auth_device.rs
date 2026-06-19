use std::net::{IpAddr, SocketAddr};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, header},
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::auth_device_service::{
    self, ApproveInput, InitiateInput, PollClaim, PreviewOutput,
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthDeviceRequestBody {
    #[serde(default)]
    pub client_label: Option<String>,
    #[serde(default)]
    pub client_user_agent: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthDeviceRequestResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub interval: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthDevicePollBody {
    pub device_code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthDevicePollResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthDeviceApproveBody {
    pub user_code: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthDevicePreviewBody {
    pub user_code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthDevicePreviewResponse {
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    pub initiated_at: String,
    pub expires_at: String,
    pub status: String,
}

#[tracing::instrument(skip_all, fields(client_ip_hash, route = "request"))]
pub async fn request_auth_device(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<AuthDeviceRequestBody>,
) -> AppResult<Json<AuthDeviceRequestResponse>> {
    let client_ip = resolve_client_ip(&headers, addr, &state)?;
    let client_ip_hash = client_ip_hash(&state, client_ip);
    tracing::Span::current().record("client_ip_hash", client_ip_hash.as_str());

    if !state.auth_device_request_limiter.check(client_ip) {
        rate_limit_hit("request", &client_ip_hash);
        return Err(AppError::AuthDeviceCodeRateLimited);
    }

    let initiated = auth_device_service::initiate(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        InitiateInput {
            client_label: body.client_label,
            client_user_agent: body.client_user_agent,
            client_ip: Some(client_ip.to_string()),
        },
    )
    .await?;

    let (verification_uri, verification_uri_complete) =
        build_verification_uris(&state.config.frontend_url, &initiated.user_code)?;

    tracing::info!(
        client_ip_hash = %client_ip_hash,
        status_code = 200_u16,
        "auth_device.handler.request"
    );

    Ok(Json(AuthDeviceRequestResponse {
        device_code: initiated.device_code,
        user_code: initiated.user_code,
        verification_uri,
        verification_uri_complete,
        expires_in: initiated.expires_in,
        interval: initiated.interval,
    }))
}

#[tracing::instrument(skip_all, fields(client_ip_hash, route = "poll"))]
pub async fn poll_auth_device(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<AuthDevicePollBody>,
) -> AppResult<Json<AuthDevicePollResponse>> {
    let client_ip = resolve_client_ip(&headers, addr, &state)?;
    let client_ip_hash = client_ip_hash(&state, client_ip);
    tracing::Span::current().record("client_ip_hash", client_ip_hash.as_str());

    if !state.auth_device_poll_limiter.check(client_ip) {
        rate_limit_hit("poll", &client_ip_hash);
        return Err(AppError::AuthDeviceCodeRateLimited);
    }

    let claim = auth_device_service::poll_and_claim(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &body.device_code,
    )
    .await?;

    match claim {
        PollClaim::Pending => {
            trace_poll_error(&client_ip_hash, "pending");
            Err(AppError::AuthDeviceCodePending)
        }
        PollClaim::SlowDown => {
            trace_poll_error(&client_ip_hash, "slow_down");
            Err(AppError::AuthDeviceCodeSlowDown)
        }
        PollClaim::Denied => {
            trace_poll_error(&client_ip_hash, "denied");
            Err(AppError::AuthDeviceCodeDenied)
        }
        PollClaim::Expired => {
            trace_poll_error(&client_ip_hash, "expired");
            Err(AppError::AuthDeviceCodeExpired)
        }
        PollClaim::AlreadyDelivered => {
            trace_poll_error(&client_ip_hash, "already_delivered");
            Err(AppError::AuthDeviceCodeAlreadyDelivered)
        }
        PollClaim::Ready {
            encrypted_access,
            encrypted_refresh,
            expires_in,
        } => {
            let (access_token, refresh_token) = auth_device_service::decrypt_tokens(
                &state.encryption_keys,
                &encrypted_access,
                &encrypted_refresh,
            )
            .await?;

            tracing::info!(
                client_ip_hash = %client_ip_hash,
                outcome = "delivered",
                "auth_device.handler.poll"
            );

            Ok(Json(AuthDevicePollResponse {
                access_token,
                refresh_token,
                token_type: "Bearer",
                expires_in,
            }))
        }
    }
}

#[tracing::instrument(skip_all, fields(client_ip_hash, route = "approve"))]
pub async fn approve_auth_device(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<AuthDeviceApproveBody>,
) -> AppResult<Json<serde_json::Value>> {
    let client_ip = resolve_client_ip(&headers, addr, &state)?;
    let client_ip_hash = client_ip_hash(&state, client_ip);
    tracing::Span::current().record("client_ip_hash", client_ip_hash.as_str());

    if !state.auth_device_approve_limiter.check(client_ip) {
        rate_limit_hit("approve", &client_ip_hash);
        return Err(AppError::AuthDeviceCodeRateLimited);
    }

    let user_key = format!("user:{}", user.user_id);
    if !state.auth_device_approve_per_user_limiter.check(&user_key) {
        rate_limit_hit("approve", &client_ip_hash);
        return Err(AppError::AuthDeviceCodeRateLimited);
    }

    auth_device_service::approve(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        ApproveInput {
            user_id: user.user_id.to_string(),
            user_code: body.user_code,
            approver_ip: Some(client_ip.to_string()),
            approver_user_agent: user_agent(&headers),
        },
    )
    .await?;

    tracing::info!(
        client_ip_hash = %client_ip_hash,
        user_id = %user.user_id,
        audit_logged = true,
        "auth_device.handler.approve"
    );

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[tracing::instrument(skip_all, fields(client_ip_hash, route = "preview"))]
pub async fn preview_auth_device(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<AuthDevicePreviewBody>,
) -> AppResult<Json<AuthDevicePreviewResponse>> {
    let client_ip = resolve_client_ip(&headers, addr, &state)?;
    let client_ip_hash = client_ip_hash(&state, client_ip);
    tracing::Span::current().record("client_ip_hash", client_ip_hash.as_str());

    if !state.auth_device_preview_limiter.check(client_ip) {
        rate_limit_hit("preview", &client_ip_hash);
        return Err(AppError::AuthDeviceCodeRateLimited);
    }

    let preview = auth_device_service::preview(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &body.user_code,
    )
    .await?;

    tracing::info!(
        client_ip_hash = %client_ip_hash,
        status_code = 200_u16,
        "auth_device.handler.preview"
    );

    Ok(Json(preview_response(preview)))
}

fn resolve_client_ip(headers: &HeaderMap, addr: SocketAddr, state: &AppState) -> AppResult<IpAddr> {
    crate::mw::rate_limit::resolve_client_ip_for_rate_limit(
        headers,
        Some(addr),
        &state.config.trusted_proxy_ips,
    )
    .or_else(|| Some(addr.ip()))
    .ok_or_else(|| AppError::Internal("unable to resolve client IP".to_string()))
}

fn client_ip_hash(state: &AppState, ip: IpAddr) -> String {
    hmac_hex(
        state.auth_device_hmac_key.as_slice(),
        ip.to_string().as_bytes(),
    )
}

fn hmac_hex(hmac_key: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(hmac_key).expect("HMAC-SHA256 accepts any key length");
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

fn build_verification_uris(frontend_url: &str, user_code: &str) -> AppResult<(String, String)> {
    let base = frontend_url.trim().trim_end_matches('/');
    if base.is_empty() {
        tracing::error!("FRONTEND_URL is empty; cannot build auth-device verification URI");
        return Err(AppError::Internal(
            "auth-device verification URI is not configured".to_string(),
        ));
    }

    let verification_uri = format!("{base}/login/device");
    let mut parsed = url::Url::parse(&verification_uri).map_err(|error| {
        tracing::error!(%error, "FRONTEND_URL is invalid; cannot build auth-device verification URI");
        AppError::Internal("auth-device verification URI is invalid".to_string())
    })?;
    parsed.query_pairs_mut().append_pair("user_code", user_code);

    Ok((verification_uri, parsed.to_string()))
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(String::from)
}

fn preview_response(preview: PreviewOutput) -> AuthDevicePreviewResponse {
    AuthDevicePreviewResponse {
        client_label: preview.client_label,
        client_user_agent: preview.client_user_agent,
        initiated_at: preview
            .initiated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        expires_at: preview
            .expires_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        status: serde_json::to_value(preview.status)
            .ok()
            .and_then(|value| value.as_str().map(String::from))
            .unwrap_or_else(|| "pending".to_string()),
    }
}

fn rate_limit_hit(route: &str, client_ip_hash: &str) {
    tracing::warn!(route, client_ip_hash, "auth_device.rate_limit_hit");
}

fn trace_poll_error(client_ip_hash: &str, outcome: &str) {
    tracing::info!(client_ip_hash, outcome, "auth_device.handler.poll");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use mongodb::bson::doc;
    use reqwest::Client;
    use serde_json::Value;
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::auth_device_service::InitiateInput;
    use crate::test_utils::{connect_test_database, test_app_config, test_app_state, test_user};

    struct TestServer {
        base_url: String,
        client: Client,
    }

    async fn spawn_test_server(state: AppState) -> TestServer {
        let (_, private) = crate::routes::build_router(
            state.config.proxy_max_body_size,
            state.config.public_proxy_max_body_size,
        );
        let app = private.with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("test server");
        });

        TestServer {
            base_url: format!("http://{addr}"),
            client: Client::new(),
        }
    }

    async fn setup_state(prefix: &str) -> Option<AppState> {
        let db = connect_test_database(prefix).await?;
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        Some(test_app_state(db))
    }

    async fn insert_user(state: &AppState, user_id: &str) {
        state
            .db
            .collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(user_id, UserType::Person))
            .await
            .expect("insert user");
    }

    fn access_token(state: &AppState, user_id: &str) -> String {
        let user_id = Uuid::parse_str(user_id).expect("valid user id");
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            "",
            None,
            None,
            None,
            None,
        )
        .expect("sign access token")
    }

    async fn post_json(
        server: &TestServer,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = server
            .client
            .post(format!("{}{}", server.base_url, path))
            .json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.expect("send request");
        let status = response.status();
        let text = response.text().await.expect("response text");
        let json = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).expect("json response")
        };
        (status, json)
    }

    async fn post_request(
        server: &TestServer,
        path: &str,
        headers: &[(&str, &str)],
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = server
            .client
            .post(format!("{}{}", server.base_url, path))
            .json(&body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await.expect("send request");
        let status = response.status();
        let json = response.json::<Value>().await.expect("json response");
        (status, json)
    }

    async fn get_json(server: &TestServer, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = server.client.get(format!("{}{}", server.base_url, path));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.expect("send request");
        let status = response.status();
        let text = response.text().await.expect("response text");
        let json = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).expect("json response")
        };
        (status, json)
    }

    async fn create_api_key(state: &AppState, user_id: &str) -> String {
        crate::services::key_service::create_api_key(
            &state.db,
            user_id,
            "codex-test",
            "read write proxy",
            None,
            None,
            None,
            None,
            Some(false),
            Some(true),
            None,
            None,
            Some("codex"),
            None,
        )
        .await
        .expect("create api key")
        .full_key
    }

    fn assert_error(json: &Value, error: &str, code: u64) {
        assert_eq!(json["error"], error);
        assert_eq!(json["error_code"], code);
    }

    #[tokio::test]
    async fn auth_device_full_happy_path_returns_valid_jwt_pair() {
        let Some(state) = setup_state("auth_device_http_happy").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_user(&state, &user_id).await;
        let token = access_token(&state, &user_id);
        let server = spawn_test_server(state.clone()).await;

        let (status, request_json) = post_json(
            &server,
            "/api/v1/auth/device/request",
            None,
            serde_json::json!({
                "client_label": "wsl-calvin",
                "client_user_agent": "nyxid-cli/0.8.0"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            request_json["verification_uri"],
            "http://localhost:3000/login/device"
        );
        assert_eq!(
            request_json["verification_uri_complete"],
            format!(
                "http://localhost:3000/login/device?user_code={}",
                request_json["user_code"].as_str().unwrap()
            )
        );
        assert_eq!(request_json["expires_in"], 600);
        assert_eq!(request_json["interval"], 5);

        let (status, approve_json) = post_json(
            &server,
            "/api/v1/auth/device/approve",
            Some(&token),
            serde_json::json!({ "user_code": request_json["user_code"].as_str().unwrap() }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(approve_json, serde_json::json!({ "ok": true }));

        let (status, poll_json) = post_json(
            &server,
            "/api/v1/auth/device/poll",
            None,
            serde_json::json!({ "device_code": request_json["device_code"].as_str().unwrap() }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(poll_json["token_type"], "Bearer");

        let access_claims = crate::crypto::jwt::verify_token(
            &state.jwt_keys,
            &state.config,
            poll_json["access_token"].as_str().unwrap(),
        )
        .expect("valid access token");
        assert_eq!(access_claims.sub, user_id);
        assert_eq!(access_claims.aud, state.config.base_url);

        let refresh_claims = crate::crypto::jwt::verify_token(
            &state.jwt_keys,
            &state.config,
            poll_json["refresh_token"].as_str().unwrap(),
        )
        .expect("valid refresh token");
        assert_eq!(refresh_claims.sub, access_claims.sub);
    }

    #[tokio::test]
    async fn auth_device_approve_rejects_api_key_auth() {
        let Some(state) = setup_state("auth_device_api_key_approve").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_user(&state, &user_id).await;
        let api_key = create_api_key(&state, &user_id).await;
        let server = spawn_test_server(state).await;

        let (status, _) = post_json(
            &server,
            "/api/v1/auth/device/approve",
            Some(&api_key),
            serde_json::json!({ "user_code": "ABCD-EFGH" }),
        )
        .await;
        assert_ne!(status, StatusCode::OK);
        assert!(matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }

    // Covers the public mount of /auth/device/preview: an anonymous caller must
    // succeed and receive the sanitized anti-phishing payload (client_label,
    // UA, timestamps, status). Approve stays human-only — see the test above.
    #[tokio::test]
    async fn auth_device_preview_accepts_anonymous_caller() {
        let Some(state) = setup_state("auth_device_preview_anon").await else {
            return;
        };
        let initiated = auth_device_service::initiate(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            InitiateInput {
                client_label: Some("kitchen-rpi".to_string()),
                client_user_agent: Some("nyxid-cli/0.7.1".to_string()),
                client_ip: Some("127.0.0.1".to_string()),
            },
        )
        .await
        .expect("initiate");
        let server = spawn_test_server(state).await;

        let (status, json) = post_json(
            &server,
            "/api/v1/auth/device/preview",
            None,
            serde_json::json!({ "user_code": initiated.user_code }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["client_label"], "kitchen-rpi");
        assert_eq!(json["client_user_agent"], "nyxid-cli/0.7.1");
        assert_eq!(json["status"], "pending");
        assert!(json["initiated_at"].is_string());
        assert!(json["expires_at"].is_string());
    }

    #[tokio::test]
    async fn auth_device_poll_unknown_device_code_returns_11200() {
        let Some(state) = setup_state("auth_device_poll_unknown").await else {
            return;
        };
        let server = spawn_test_server(state).await;
        let (status, json) = post_json(
            &server,
            "/api/v1/auth/device/poll",
            None,
            serde_json::json!({ "device_code": "nyx_adc_unknown" }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_error(&json, "auth_device_code_not_found", 11200);
    }

    #[tokio::test]
    async fn auth_device_two_concurrent_polls_only_deliver_once() {
        let Some(state) = setup_state("auth_device_concurrent_poll").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_user(&state, &user_id).await;
        let initiated = auth_device_service::initiate(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            InitiateInput {
                client_label: None,
                client_user_agent: None,
                client_ip: Some("127.0.0.1".to_string()),
            },
        )
        .await
        .expect("initiate");
        auth_device_service::approve(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            ApproveInput {
                user_id,
                user_code: initiated.user_code,
                approver_ip: Some("127.0.0.1".to_string()),
                approver_user_agent: Some("nyxid-cli/0.8.0".to_string()),
            },
        )
        .await
        .expect("approve");
        let server = spawn_test_server(state).await;

        let body = serde_json::json!({ "device_code": initiated.device_code });
        let first = post_json(&server, "/api/v1/auth/device/poll", None, body.clone());
        let second = post_json(&server, "/api/v1/auth/device/poll", None, body);
        let (a, b) = tokio::join!(first, second);
        let mut statuses = [a.0, b.0];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::GONE]);
        let gone = if a.0 == StatusCode::GONE { a.1 } else { b.1 };
        assert_error(&gone, "auth_device_already_delivered", 11205);
    }

    #[tokio::test]
    async fn auth_device_request_rate_limit_sixth_request_returns_11206() {
        let Some(state) = setup_state("auth_device_rate_limit").await else {
            return;
        };
        let server = spawn_test_server(state).await;
        let mut last = (StatusCode::OK, Value::Null);
        for _ in 0..6 {
            last = post_json(
                &server,
                "/api/v1/auth/device/request",
                None,
                serde_json::json!({}),
            )
            .await;
        }

        assert_eq!(last.0, StatusCode::TOO_MANY_REQUESTS);
        assert_error(&last.1, "auth_device_rate_limited", 11206);
    }

    #[tokio::test]
    async fn auth_device_untrusted_forwarded_for_cannot_bypass_request_rate_limit() {
        let Some(state) = setup_state("auth_device_spoof_xff").await else {
            return;
        };
        let server = spawn_test_server(state).await;
        let mut last = (StatusCode::OK, Value::Null);
        for i in 0..6 {
            let spoofed = format!("198.51.100.{i}");
            last = post_request(
                &server,
                "/api/v1/auth/device/request",
                &[("x-forwarded-for", spoofed.as_str())],
                serde_json::json!({}),
            )
            .await;
        }

        assert_eq!(last.0, StatusCode::TOO_MANY_REQUESTS);
        assert_error(&last.1, "auth_device_rate_limited", 11206);
    }

    #[tokio::test]
    async fn auth_device_approve_emits_audit_log() {
        let Some(state) = setup_state("auth_device_audit").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_user(&state, &user_id).await;
        let token = access_token(&state, &user_id);
        let server = spawn_test_server(state.clone()).await;

        let (_, request_json) = post_json(
            &server,
            "/api/v1/auth/device/request",
            None,
            serde_json::json!({}),
        )
        .await;
        let (status, _) = post_json(
            &server,
            "/api/v1/auth/device/approve",
            Some(&token),
            serde_json::json!({ "user_code": request_json["user_code"].as_str().unwrap() }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        for _ in 0..20 {
            if let Some(log) = state
                .db
                .collection::<AuditLog>(AUDIT_LOG)
                .find_one(doc! { "event_type": "auth_device_code_approved", "user_id": &user_id })
                .await
                .expect("query audit log")
            {
                assert!(log.api_key_id.is_none());
                assert_eq!(log.user_id.as_deref(), Some(user_id.as_str()));
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("expected auth_device_code_approved audit log");
    }

    #[tokio::test]
    async fn auth_device_request_with_empty_frontend_url_returns_500() {
        let Some(db) = connect_test_database("auth_device_empty_frontend").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let mut config = test_app_config();
        config.frontend_url = String::new();
        let state = crate::test_utils::test_app_state_with_config(db, config);
        let server = spawn_test_server(state).await;

        let (status, json) = post_json(
            &server,
            "/api/v1/auth/device/request",
            None,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"], "internal_error");
    }

    #[tokio::test]
    async fn e2e_full_happy_path_request_approve_poll_refresh() {
        let Some(state) = setup_state("auth_device_e2e_happy").await else {
            return;
        };
        crate::services::role_service::seed_system_roles(&state.db)
            .await
            .expect("seed roles");
        let user_id = Uuid::new_v4().to_string();
        insert_user(&state, &user_id).await;
        let approving_jwt = access_token(&state, &user_id);
        let server = spawn_test_server(state.clone()).await;

        let (status, request_json) = post_json(
            &server,
            "/api/v1/auth/device/request",
            None,
            serde_json::json!({
                "client_label": "wsl-calvin",
                "client_user_agent": "nyxid-cli/0.8.0"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            request_json["device_code"]
                .as_str()
                .is_some_and(|value| value.starts_with("nyx_adc_"))
        );
        assert!(
            request_json["user_code"]
                .as_str()
                .is_some_and(|value| value.len() == 9 && value.contains('-'))
        );

        let (status, approve_json) = post_json(
            &server,
            "/api/v1/auth/device/approve",
            Some(&approving_jwt),
            serde_json::json!({ "user_code": request_json["user_code"].as_str().unwrap() }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(approve_json, serde_json::json!({ "ok": true }));

        let (status, poll_json) = post_json(
            &server,
            "/api/v1/auth/device/poll",
            None,
            serde_json::json!({ "device_code": request_json["device_code"].as_str().unwrap() }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            poll_json["access_token"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
        assert!(
            poll_json["refresh_token"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        );
        assert_eq!(poll_json["token_type"], "Bearer");
        assert_eq!(poll_json["expires_in"], 900);

        let access_token = poll_json["access_token"].as_str().unwrap();
        let (status, me_json) = get_json(&server, "/api/v1/users/me", Some(access_token)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me_json["id"], user_id);
        assert_eq!(me_json["email"], format!("{user_id}@example.com"));
        assert_eq!(me_json["display_name"], "Test User");

        let refresh_token = poll_json["refresh_token"].as_str().unwrap();
        let (status, refresh_json) = post_json(
            &server,
            "/api/v1/auth/refresh",
            None,
            serde_json::json!({ "refresh_token": refresh_token }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            refresh_json["access_token"]
                .as_str()
                .is_some_and(|value| !value.is_empty() && value != access_token)
        );
        assert!(
            refresh_json["refresh_token"]
                .as_str()
                .is_some_and(|value| !value.is_empty() && value != refresh_token)
        );
        assert_eq!(refresh_json["expires_in"], 900);

        let (status, refreshed_me_json) = get_json(
            &server,
            "/api/v1/users/me",
            Some(refresh_json["access_token"].as_str().unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(refreshed_me_json["id"], user_id);
    }

    #[tokio::test]
    async fn e2e_concurrent_poll_after_approve_only_one_winner() {
        let Some(state) = setup_state("auth_device_e2e_concurrent_poll").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_user(&state, &user_id).await;
        let approving_jwt = access_token(&state, &user_id);
        let server = spawn_test_server(state).await;

        let (status, request_json) = post_json(
            &server,
            "/api/v1/auth/device/request",
            None,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = post_json(
            &server,
            "/api/v1/auth/device/approve",
            Some(&approving_jwt),
            serde_json::json!({ "user_code": request_json["user_code"].as_str().unwrap() }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let body =
            serde_json::json!({ "device_code": request_json["device_code"].as_str().unwrap() });
        let first = post_json(&server, "/api/v1/auth/device/poll", None, body.clone());
        let second = post_json(&server, "/api/v1/auth/device/poll", None, body);
        let (first, second) = tokio::join!(first, second);

        let ok_count = [first.0, second.0]
            .into_iter()
            .filter(|status| *status == StatusCode::OK)
            .count();
        let already_delivered = [first, second]
            .into_iter()
            .filter(|(status, json)| {
                *status == StatusCode::GONE && json["error_code"].as_u64() == Some(11205)
            })
            .count();
        assert_eq!(ok_count, 1);
        assert_eq!(already_delivered, 1);
    }

    #[tokio::test]
    async fn e2e_spoofed_xff_does_not_bypass_request_rate_limit() {
        let Some(state) = setup_state("auth_device_e2e_spoof_xff").await else {
            return;
        };
        let server = spawn_test_server(state).await;
        let mut last = (StatusCode::OK, Value::Null);
        for i in 0..6 {
            let spoofed = format!("198.51.100.{i}");
            last = post_request(
                &server,
                "/api/v1/auth/device/request",
                &[("x-forwarded-for", spoofed.as_str())],
                serde_json::json!({}),
            )
            .await;
        }

        assert_eq!(last.0, StatusCode::TOO_MANY_REQUESTS);
        assert_error(&last.1, "auth_device_rate_limited", 11206);
    }

    #[tokio::test]
    async fn e2e_api_key_auth_rejected_on_approve() {
        let Some(state) = setup_state("auth_device_e2e_api_key_reject").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        insert_user(&state, &user_id).await;
        let api_key = create_api_key(&state, &user_id).await;
        let server = spawn_test_server(state).await;

        let (approve_status, approve_json) = post_json(
            &server,
            "/api/v1/auth/device/approve",
            Some(&api_key),
            serde_json::json!({ "user_code": "ABCD-EFGH" }),
        )
        .await;
        assert_ne!(approve_status, StatusCode::OK);
        assert!(matches!(
            approve_status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
        assert_ne!(approve_json, serde_json::json!({ "ok": true }));
    }
}
