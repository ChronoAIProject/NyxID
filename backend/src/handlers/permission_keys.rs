use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use nyxid_permissions::{Engine, Error, MAX_BODY_BYTES, MAX_RESPONSE_BYTES, Policy, Transport};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Semaphore;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{api_key::ApiKeyPurpose, permission_policy::PermissionPolicy},
    mw::auth::AuthUser,
    services::{
        audit_service,
        billing::route_inventory::{
            BillingIngress, BillingRoutePolicy, enforce_billing_egress_classification,
        },
        key_service, permission_policy_service as policies, proxy_service,
    },
};

// Engine instances bind one request's live policy. Admission is process-wide.
static EXECUTIONS: Semaphore = Semaphore::const_new(32);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    name: String,
    user_service_id: String,
    expires_at: DateTime<Utc>,
    policy: Policy,
}

#[derive(Serialize)]
pub struct PolicyResponse {
    id: String,
    user_service_id: String,
    catalog_service_id: String,
    policy: Policy,
    paused: bool,
    revision: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<PermissionPolicy> for PolicyResponse {
    fn from(p: PermissionPolicy) -> Self {
        Self {
            id: p.id,
            user_service_id: p.user_service_id,
            catalog_service_id: p.catalog_service_id,
            policy: p.policy,
            paused: p.paused,
            revision: p.revision,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Serialize)]
pub struct CreatedResponse {
    key: zeroize::Zeroizing<String>,
    expires_at: DateTime<Utc>,
    binding: PolicyResponse,
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<CreateInput>,
) -> AppResult<Response> {
    super::login_client_context::require_first_party_human(&auth)?;
    let (created, binding) = policies::create(
        &state.db,
        &state.encryption_keys,
        &auth.user_id.to_string(),
        &input.name,
        &input.user_service_id,
        input.expires_at,
        input.policy,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "permission_key_created",
        Some(json!({"permission_key_id": binding.id, "user_service_id": binding.user_service_id})),
    );
    let mut response = (
        StatusCode::CREATED,
        Json(CreatedResponse {
            key: zeroize::Zeroizing::new(created.full_key),
            expires_at: input.expires_at,
            binding: binding.into(),
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<PolicyResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        policies::get(&state.db, &auth.user_id.to_string(), &id)
            .await?
            .into(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseInput {
    revision: i64,
    paused: bool,
}

pub async fn pause(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<PauseInput>,
) -> AppResult<Json<PolicyResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let binding = policies::set_paused(
        &state.db,
        &auth.user_id.to_string(),
        &id,
        input.revision,
        input.paused,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "permission_key_pause_changed",
        Some(
            json!({"permission_key_id": id, "paused": input.paused, "revision": binding.revision}),
        ),
    );
    Ok(Json(binding.into()))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    policies::get(&state.db, &auth.user_id.to_string(), &id).await?;
    key_service::delete_api_key(&state.db, &auth.user_id.to_string(), &id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "permission_key_revoked",
        Some(json!({"permission_key_id": id})),
    );
    Ok(StatusCode::NO_CONTENT)
}

struct BackendTransport {
    state: AppState,
    auth: AuthUser,
    binding: PermissionPolicy,
    billing: BillingRoutePolicy,
}

impl BackendTransport {
    async fn forward(
        &self,
        request: &nyxid_permissions::Request,
        origin: Option<&str>,
    ) -> AppResult<Response> {
        policies::check_live(&self.state.db, &self.binding).await?;
        let resolved = proxy_service::resolve_proxy_target_by_user_service_id(
            &self.state.db,
            &self.state.encryption_keys,
            &self.binding.user_id,
            &self.binding.user_service_id,
            None,
            Some(&self.binding.catalog_service_id),
            proxy_service::ProxyExecutionContext::new(
                Some(&self.state.connection_expiry_notifier),
                self.state.platform_user_rate_limit,
            ),
        )
        .await?
        .ok_or_else(|| AppError::Forbidden("Bound connection unavailable".into()))?;
        let query = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in &request.query {
                serializer.append_pair(key, value);
            }
            serializer.finish()
        };
        let uri = if query.is_empty() {
            request.path.clone()
        } else {
            format!("{}?{query}", request.path)
        };
        let mut builder = Request::builder().method(request.method.clone()).uri(uri);
        if let Some(content_type) = &request.content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        let mut proxy_request = builder
            .body(Body::from(request.body.clone()))
            .map_err(|_| AppError::BadRequest("Invalid permission request".into()))?;
        proxy_request.extensions_mut().insert(self.billing);
        super::proxy::execute_permission_request(
            &self.state,
            &self.auth,
            &self.binding,
            origin,
            resolved,
            request.path.trim_start_matches('/'),
            proxy_request,
        )
        .await
    }
}

#[async_trait]
impl Transport for BackendTransport {
    async fn send(
        &self,
        request: &nyxid_permissions::Request,
    ) -> Result<nyxid_permissions::Response, Error> {
        self.send_to(request, None).await
    }

    async fn send_to(
        &self,
        request: &nyxid_permissions::Request,
        origin: Option<&str>,
    ) -> Result<nyxid_permissions::Response, Error> {
        let response = self
            .forward(request, origin)
            .await
            .map_err(|_| Error::Upstream)?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let body = to_bytes(response.into_body(), MAX_RESPONSE_BYTES)
            .await
            .map_err(|_| Error::ResponseTooLarge)?;
        Ok(nyxid_permissions::Response {
            status,
            content_type,
            body,
        })
    }
}

async fn prepare(
    state: &AppState,
    auth: &AuthUser,
    headers: &axum::http::HeaderMap,
    billing: Option<BillingRoutePolicy>,
) -> AppResult<Engine> {
    if auth.api_key_purpose != ApiKeyPurpose::PermissionBound
        || auth.api_key_credential_id.is_some()
    {
        return Err(AppError::Forbidden(
            "A primary permission-bound API key is required".into(),
        ));
    }
    let id = auth
        .api_key_id
        .as_deref()
        .ok_or_else(|| AppError::Forbidden("Permission key required".into()))?;
    if [header::ORIGIN, header::CONTENT_ENCODING, header::UPGRADE]
        .iter()
        .any(|h| headers.contains_key(h))
    {
        return Err(AppError::BadRequest(
            "Origins, compressed bodies and upgrades are unsupported".into(),
        ));
    }
    enforce_billing_egress_classification(billing, BillingIngress::Proxy)?;
    let binding = policies::get(&state.db, &auth.user_id.to_string(), id).await?;
    policies::check_live(&state.db, &binding).await?;
    let transport = Arc::new(BackendTransport {
        state: state.clone(),
        auth: auth.clone(),
        binding: binding.clone(),
        billing: billing.expect("classification checked"),
    });
    policies::engine(state.db.clone(), binding, transport)
}

fn permission_error(error: Error) -> Response {
    let status = match error {
        Error::Policy(_) => StatusCode::BAD_REQUEST,
        Error::Denied(_) | Error::HookDenied { .. } => StatusCode::FORBIDDEN,
        Error::HookUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        Error::Timeout => StatusCode::GATEWAY_TIMEOUT,
        Error::Busy => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_GATEWAY,
    };
    (status, Json(json!({"error": error.to_string()}))).into_response()
}

pub async fn rest(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    execute(state, auth, request, false).await
}

pub async fn mcp(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    execute(state, auth, request, true).await
}

async fn execute(
    state: AppState,
    auth: AuthUser,
    request: Request<Body>,
    mcp: bool,
) -> AppResult<Response> {
    let _permit = match EXECUTIONS.try_acquire() {
        Ok(p) => p,
        Err(_) => return Ok(permission_error(Error::Busy)),
    };
    let cancellation = crate::downstream_disconnect::request_cancellation(&request);
    let result = crate::downstream_disconnect::until_client_disconnect(
        &cancellation,
        execute_inner(&state, &auth, request, mcp),
    )
    .await
    .unwrap_or(Err(AppError::ClientDisconnected));
    let mut response = match result {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    let tool_error = response
        .extensions()
        .get::<nyxid_permissions::server::McpToolOutcome>()
        .map(|v| v.is_error);
    let status = response.status().as_u16();
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "permission_key_request",
        Some(
            json!({"permission_key_id": auth.api_key_id, "channel": if mcp {"mcp"} else {"rest"}, "response_status": status, "tool_error": tool_error}),
        ),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        header::HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

async fn execute_inner(
    state: &AppState,
    auth: &AuthUser,
    request: Request<Body>,
    mcp: bool,
) -> AppResult<Response> {
    let (parts, body) = request.into_parts();
    let prepared = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let engine = prepare(
            state,
            auth,
            &parts.headers,
            parts.extensions.get::<BillingRoutePolicy>().copied(),
        )
        .await?;
        let body =
            to_bytes(body, MAX_BODY_BYTES)
                .await
                .map_err(|_| AppError::RequestBodyTooLarge {
                    max_bytes: MAX_BODY_BYTES,
                    context: "Permission execution".into(),
                })?;
        Ok::<_, AppError>((engine, body))
    })
    .await;
    let (engine, body) = match prepared {
        Ok(result) => result?,
        Err(_) => return Ok(permission_error(Error::Timeout)),
    };
    if mcp {
        return Ok(nyxid_permissions::server::handle_mcp(&engine, body).await);
    }
    let query = match nyxid_permissions::parse_query(parts.uri.query().unwrap_or("")) {
        Ok(q) => q,
        Err(e) => return Ok(permission_error(e)),
    };
    let path = parts
        .uri
        .path()
        .strip_prefix("/api/v1/permission-execution/rest")
        .ok_or_else(|| AppError::BadRequest("Invalid permission execution path".into()))?;
    let result = engine
        .execute(nyxid_permissions::Request {
            method: parts.method,
            path: path.into(),
            query,
            body,
            content_type: parts
                .headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
        })
        .await;
    Ok(match result {
        Ok(r) => {
            let mut response = (
                StatusCode::from_u16(r.status).unwrap_or(StatusCode::BAD_GATEWAY),
                r.body,
            )
                .into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                r.content_type
                    .parse()
                    .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
            );
            response.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                header::HeaderValue::from_static("attachment"),
            );
            response
        }
        Err(e) => permission_error(e),
    })
}
