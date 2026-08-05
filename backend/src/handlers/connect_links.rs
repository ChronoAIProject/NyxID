use std::net::{IpAddr, SocketAddr};

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::connect_link::ConnectLinkStatus;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, connect_link_service, user_token_service};

#[derive(Deserialize, ToSchema)]
pub struct CreateConnectLinkRequest {
    pub service_slug: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub callback_url: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct CreateConnectLinkResponse {
    pub id: String,
    pub connect_url: String,
    pub expires_at: String,
}

#[derive(Deserialize, ToSchema)]
pub struct PreviewConnectLinkRequest {
    pub token: String,
}

impl std::fmt::Debug for PreviewConnectLinkRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewConnectLinkRequest")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewConnectLinkResponse {
    pub service_name: String,
    pub service_slug: String,
    pub label: Option<String>,
    pub requested_by: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub status: String,
    pub connect_method: String,
    pub auth_key_name: String,
    pub credential_mode: Option<String>,
    pub has_platform_oauth_credentials: bool,
    pub requires_gateway_url: bool,
    pub api_key_url: Option<String>,
    pub api_key_instructions: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectedServiceResponse {
    pub id: String,
    pub slug: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectLinkStatusResponse {
    pub id: String,
    pub status: String,
    pub service_name: String,
    pub service_slug: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_service: Option<ConnectedServiceResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CompleteConnectLinkRequest {
    pub token: String,
    #[serde(default)]
    pub credential: Option<String>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub oauth_client_secret: Option<String>,
    #[serde(default)]
    pub device_state: Option<String>,
}

impl std::fmt::Debug for CompleteConnectLinkRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompleteConnectLinkRequest")
            .field("token", &"[REDACTED]")
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "endpoint_url",
                &self.endpoint_url.as_ref().map(|value| value.len()),
            )
            .field(
                "oauth_client_id",
                &self.oauth_client_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "oauth_client_secret",
                &self.oauth_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "device_state",
                &self.device_state.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Serialize, ToSchema)]
pub struct CompleteConnectLinkResponse {
    pub id: String,
    pub status: String,
    pub service_slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_user_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_verification_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_interval: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/connect-links",
    request_body = CreateConnectLinkRequest,
    responses((status = 200, body = CreateConnectLinkResponse)),
    tag = "Connect Links"
)]
pub async fn create_connect_link(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateConnectLinkRequest>,
) -> AppResult<Json<CreateConnectLinkResponse>> {
    let actor_id = auth_user.user_id.to_string();
    let rate_key = auth_user
        .api_key_id
        .as_deref()
        .map_or_else(|| format!("user:{actor_id}"), |id| format!("api-key:{id}"));
    if !state.connect_link_create_limiter.check(&rate_key) {
        return Err(AppError::ConnectLinkRateLimited);
    }

    let created = connect_link_service::create(
        &state.db,
        connect_link_service::CreateInput {
            user_id: actor_id,
            service_slug: body.service_slug,
            label: body.label,
            requested_by: auth_user.api_key_name.clone().or(body.requested_by),
            callback_url: body.callback_url,
            ttl_secs: body.expires_in,
        },
    )
    .await?;
    let connect_url =
        connect_link_service::build_connect_url(&state.config.frontend_url, &created.raw_token)?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "connect_link_created",
        Some(serde_json::json!({
            "connect_link_id": &created.link.id,
            "service_id": &created.link.service_id,
            "service_slug": &created.link.service_slug,
            "expires_at": created.link.expires_at.to_rfc3339(),
            "has_callback_url": created.link.callback_url.is_some(),
        })),
    );

    Ok(Json(CreateConnectLinkResponse {
        id: created.link.id,
        connect_url,
        expires_at: created.link.expires_at.to_rfc3339(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/connect-links/{id}",
    responses((status = 200, body = ConnectLinkStatusResponse)),
    tag = "Connect Links"
)]
pub async fn get_connect_link(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ConnectLinkStatusResponse>> {
    let view =
        connect_link_service::get_for_actor(&state.db, &auth_user.user_id.to_string(), &id).await?;
    Ok(Json(status_response(view)))
}

#[utoipa::path(
    post,
    path = "/api/v1/connect-links/{id}/cancel",
    responses((status = 200, body = ConnectLinkStatusResponse)),
    tag = "Connect Links"
)]
pub async fn cancel_connect_link(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ConnectLinkStatusResponse>> {
    let view = connect_link_service::cancel(&state.db, &auth_user.user_id.to_string(), &id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "connect_link_cancelled",
        Some(serde_json::json!({
            "connect_link_id": &view.link.id,
            "service_id": &view.link.service_id,
            "service_slug": &view.link.service_slug,
        })),
    );
    Ok(Json(status_response(view)))
}

#[utoipa::path(
    post,
    path = "/api/v1/connect-links/preview",
    request_body = PreviewConnectLinkRequest,
    responses((status = 200, body = PreviewConnectLinkResponse)),
    tag = "Connect Links"
)]
pub async fn preview_connect_link(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PreviewConnectLinkRequest>,
) -> AppResult<Json<PreviewConnectLinkResponse>> {
    let client_ip = resolve_client_ip(&headers, addr, &state)?;
    if !state.connect_link_preview_limiter.check(client_ip) {
        return Err(AppError::ConnectLinkRateLimited);
    }
    let view = connect_link_service::preview(&state.db, &body.token).await?;
    let connect_method = view.service.connect_method().to_string();
    Ok(Json(PreviewConnectLinkResponse {
        service_name: view.service.service_name,
        service_slug: view.service.service_slug,
        label: view.link.label,
        requested_by: view.link.requested_by,
        created_at: view.link.created_at.to_rfc3339(),
        expires_at: view.link.expires_at.to_rfc3339(),
        status: status_name(view.link.status).to_string(),
        connect_method,
        auth_key_name: view.service.auth_key_name,
        credential_mode: view.service.credential_mode,
        has_platform_oauth_credentials: view.service.has_platform_oauth_credentials,
        requires_gateway_url: view.service.requires_gateway_url,
        api_key_url: view.service.api_key_url,
        api_key_instructions: view.service.api_key_instructions,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/connect-links/complete",
    request_body = CompleteConnectLinkRequest,
    responses((status = 200, body = CompleteConnectLinkResponse)),
    tag = "Connect Links"
)]
pub async fn complete_connect_link(
    State(state): State<AppState>,
    auth_user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CompleteConnectLinkRequest>,
) -> AppResult<Json<CompleteConnectLinkResponse>> {
    let client_ip = resolve_client_ip(&headers, addr, &state)?;
    if !state.connect_link_complete_limiter.check(client_ip) {
        return Err(AppError::ConnectLinkRateLimited);
    }
    let actor_id = auth_user.user_id.to_string();
    let result = connect_link_service::complete(
        &state.db,
        &state.encryption_keys,
        &actor_id,
        &body.token,
        connect_link_service::CompleteInput {
            credential: body.credential.as_deref(),
            endpoint_url: body.endpoint_url.as_deref(),
            oauth_client_id: body.oauth_client_id.as_deref(),
            oauth_client_secret: body.oauth_client_secret.as_deref(),
        },
        state.config.is_production(),
    )
    .await?;

    match result {
        connect_link_service::CompleteResult::Completed(view) => {
            audit_completed(&state, &auth_user, &view);
            Ok(Json(completion_response(view, "completed")))
        }
        connect_link_service::CompleteResult::OauthRequired {
            view,
            provider_id,
            connection_id,
        } => {
            let redirect_path = format!("/connect/return/{}", view.link.id);
            let on_behalf_of =
                (view.link.user_id != actor_id).then_some(view.link.user_id.as_str());
            let authorization_url = user_token_service::initiate_oauth_connect(
                &state.db,
                &state.encryption_keys,
                &state.config.base_url,
                &actor_id,
                &provider_id,
                on_behalf_of,
                Some(&redirect_path),
                &[],
                None,
                Some(&connection_id),
                Some(&view.link.id),
            )
            .await?;
            let mut response = completion_response(view, "oauth_required");
            response.authorization_url = Some(authorization_url);
            Ok(Json(response))
        }
        connect_link_service::CompleteResult::DeviceCodeRequired {
            view,
            provider_id,
            connection_id,
        } => {
            if let Some(device_state) = body.device_state.as_deref() {
                let poll = user_token_service::poll_device_code(
                    &state.db,
                    &state.encryption_keys,
                    &actor_id,
                    &provider_id,
                    device_state,
                )
                .await?;
                return match poll.status.as_str() {
                    "complete" => {
                        let completed = connect_link_service::complete_oauth_callback(
                            &state.db,
                            &view.link.id,
                            &view.link.user_id,
                            &connection_id,
                        )
                        .await?;
                        audit_completed(&state, &auth_user, &completed);
                        Ok(Json(completion_response(completed, "completed")))
                    }
                    "pending" | "slow_down" => {
                        let mut response = completion_response(view, "device_code_required");
                        response.device_state = Some(device_state.to_string());
                        response.device_interval = poll.interval;
                        response.device_status = Some(poll.status);
                        Ok(Json(response))
                    }
                    "expired" | "denied" => Err(AppError::BadRequest(format!(
                        "Provider device authorization {}",
                        poll.status
                    ))),
                    _ => Err(AppError::Internal(
                        "Provider returned an unknown device authorization status".to_string(),
                    )),
                };
            }
            let on_behalf_of =
                (view.link.user_id != actor_id).then_some(view.link.user_id.as_str());
            let device = user_token_service::request_device_code(
                &state.db,
                &state.encryption_keys,
                &actor_id,
                &provider_id,
                on_behalf_of,
                &[],
                None,
                Some(&connection_id),
            )
            .await?;
            let mut response = completion_response(view, "device_code_required");
            response.device_user_code = Some(device.user_code);
            response.device_verification_uri = Some(device.verification_uri);
            response.device_state = Some(device.state);
            response.device_interval = Some(device.interval);
            response.device_status = Some("pending".to_string());
            Ok(Json(response))
        }
    }
}

fn audit_completed(state: &AppState, auth_user: &AuthUser, view: &connect_link_service::LinkView) {
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "connect_link_completed",
        Some(serde_json::json!({
            "connect_link_id": &view.link.id,
            "service_id": &view.link.service_id,
            "service_slug": &view.link.service_slug,
            "user_service_id": &view.link.completed_user_service_id,
        })),
    );
}

fn completion_response(
    view: connect_link_service::LinkView,
    status: &str,
) -> CompleteConnectLinkResponse {
    CompleteConnectLinkResponse {
        id: view.link.id,
        status: status.to_string(),
        service_slug: view
            .completed_service_slug
            .unwrap_or(view.service.service_slug),
        user_service_id: view.link.completed_user_service_id,
        authorization_url: None,
        device_user_code: None,
        device_verification_uri: None,
        device_state: None,
        device_interval: None,
        device_status: None,
        callback_url: view.link.callback_url,
    }
}

fn status_response(view: connect_link_service::LinkView) -> ConnectLinkStatusResponse {
    let connected_service = match (
        view.link.completed_user_service_id.as_ref(),
        view.completed_service_slug,
    ) {
        (Some(id), Some(slug)) => Some(ConnectedServiceResponse {
            id: id.clone(),
            slug,
        }),
        _ => None,
    };
    ConnectLinkStatusResponse {
        id: view.link.id,
        status: status_name(view.link.status).to_string(),
        service_name: view.service.service_name,
        service_slug: view.service.service_slug,
        expires_at: view.link.expires_at.to_rfc3339(),
        completed_at: view.link.completed_at.map(|date| date.to_rfc3339()),
        connected_service,
        callback_url: view.link.callback_url,
    }
}

fn status_name(status: ConnectLinkStatus) -> &'static str {
    match status {
        ConnectLinkStatus::Pending => "pending",
        ConnectLinkStatus::Completed => "completed",
        ConnectLinkStatus::Expired => "expired",
        ConnectLinkStatus::Cancelled => "cancelled",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, test_helpers::dummy_service,
    };
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user};

    #[test]
    fn completion_request_debug_redacts_all_secret_inputs() {
        let request = CompleteConnectLinkRequest {
            token: "nyx_clk_secret".to_string(),
            credential: Some("api-secret".to_string()),
            endpoint_url: Some("https://gateway.example.test".to_string()),
            oauth_client_id: Some("client-id".to_string()),
            oauth_client_secret: Some("client-secret".to_string()),
            device_state: Some("device-state".to_string()),
        };
        let debug = format!("{request:?}");
        for secret in [
            "nyx_clk_secret",
            "api-secret",
            "client-id",
            "client-secret",
            "device-state",
        ] {
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn preview_request_debug_redacts_token() {
        let request = PreviewConnectLinkRequest {
            token: "nyx_clk_preview-secret".to_string(),
        };
        assert!(!format!("{request:?}").contains("preview-secret"));
    }

    #[test]
    fn status_names_are_wire_stable() {
        assert_eq!(status_name(ConnectLinkStatus::Pending), "pending");
        assert_eq!(status_name(ConnectLinkStatus::Completed), "completed");
        assert_eq!(status_name(ConnectLinkStatus::Expired), "expired");
        assert_eq!(status_name(ConnectLinkStatus::Cancelled), "cancelled");
    }

    #[tokio::test]
    async fn create_and_public_preview_handler_round_trip() {
        let Some(db) = connect_test_database("connect_link_handler_round_trip").await else {
            return;
        };
        let mut service: DownstreamService = dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = format!("connect-handler-{}", uuid::Uuid::new_v4());
        service.name = "Handler Test Service".to_string();
        service.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert service");
        let state = test_app_state(db);
        let actor_id = uuid::Uuid::new_v4().to_string();

        let Json(created) = create_connect_link(
            State(state.clone()),
            test_auth_user(&actor_id),
            Json(CreateConnectLinkRequest {
                service_slug: service.slug.clone(),
                label: Some("Agent setup".to_string()),
                requested_by: Some("handler-test".to_string()),
                callback_url: None,
                expires_in: None,
            }),
        )
        .await
        .expect("create response");
        let token = created
            .connect_url
            .rsplit('/')
            .next()
            .expect("token path segment")
            .to_string();

        let Json(preview) = preview_connect_link(
            State(state),
            ConnectInfo("127.0.0.1:43123".parse().unwrap()),
            HeaderMap::new(),
            Json(PreviewConnectLinkRequest { token }),
        )
        .await
        .expect("preview response");
        assert_eq!(preview.service_slug, service.slug);
        assert_eq!(preview.requested_by.as_deref(), Some("handler-test"));
        assert_eq!(preview.status, "pending");
    }

    #[tokio::test]
    async fn api_key_completion_is_owner_scoped_and_single_use() {
        let Some(db) = connect_test_database("connect_link_handler_complete").await else {
            return;
        };
        let mut service: DownstreamService = dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = format!("connect-complete-{}", uuid::Uuid::new_v4());
        service.name = "Completion Test Service".to_string();
        service.auth_method = "bearer".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert service");
        let state = test_app_state(db.clone());
        let actor_id = uuid::Uuid::new_v4().to_string();
        let created = connect_link_service::create(
            &db,
            connect_link_service::CreateInput {
                user_id: actor_id.clone(),
                service_slug: service.slug.clone(),
                label: Some("Completion test".to_string()),
                requested_by: None,
                callback_url: None,
                ttl_secs: None,
            },
        )
        .await
        .expect("create link");

        let denied = complete_connect_link(
            State(state.clone()),
            test_auth_user(&uuid::Uuid::new_v4().to_string()),
            ConnectInfo("127.0.0.1:43124".parse().unwrap()),
            HeaderMap::new(),
            Json(CompleteConnectLinkRequest {
                token: created.raw_token.clone(),
                credential: Some("test-secret".to_string()),
                endpoint_url: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                device_state: None,
            }),
        )
        .await;
        assert!(matches!(denied, Err(AppError::ConnectLinkNotFound)));

        let Json(completed) = complete_connect_link(
            State(state.clone()),
            test_auth_user(&actor_id),
            ConnectInfo("127.0.0.1:43125".parse().unwrap()),
            HeaderMap::new(),
            Json(CompleteConnectLinkRequest {
                token: created.raw_token.clone(),
                credential: Some("test-secret".to_string()),
                endpoint_url: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                device_state: None,
            }),
        )
        .await
        .expect("complete link");
        assert_eq!(completed.status, "completed");
        assert!(completed.user_service_id.is_some());

        let replay = complete_connect_link(
            State(state),
            test_auth_user(&actor_id),
            ConnectInfo("127.0.0.1:43126".parse().unwrap()),
            HeaderMap::new(),
            Json(CompleteConnectLinkRequest {
                token: created.raw_token,
                credential: Some("test-secret".to_string()),
                endpoint_url: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                device_state: None,
            }),
        )
        .await;
        assert!(matches!(replay, Err(AppError::ConnectLinkAlreadyCompleted)));
    }
}
