#![allow(
    clippy::too_many_arguments,
    clippy::collapsible_if,
    clippy::needless_borrow
)]

use crate::{
    AppState,
    crypto::jwt::{
        Claims, DelegationOrigin, NYXBOT_GATEWAY_CREATOR_ACTOR as CREATOR_ACTOR,
        NYXBOT_GATEWAY_CREATOR_TOKEN_TTL_SECS as CREATOR_TOKEN_TTL_SECS,
    },
    errors::{AppError, AppResult},
    models::oauth_client::OauthClient,
};
use mongodb::bson::{self, Document, doc};
use serde_json::Value;
use std::{sync::LazyLock, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;
const GATEWAY_SLUG: &str = "cmaeg";
const GATEWAY_TIMEOUT_SECS: u64 = 20;
const MAX_GATEWAY_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) static GATEWAY_HTTP_CLIENT: LazyLock<reqwest::Client> =
    LazyLock::new(gateway_http_client);

fn gateway_http_client() -> reqwest::Client {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5));
    // Like the shared proxy clients, tests must not reuse idle connections
    // driven by another test's Tokio runtime. Wiremock reuses server ports;
    // tearing down that runtime can otherwise drop an active dispatch task.
    // Production uses one long-lived runtime and retains its normal pooling.
    #[cfg(test)]
    let builder = builder.pool_max_idle_per_host(0);
    builder.build().expect("gateway HTTP client")
}
fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
pub(crate) async fn gateway_base(state: &AppState) -> AppResult<String> {
    let row =
        crate::services::assistant_service::resolve_admin_service_by_slug(&state.db, GATEWAY_SLUG)
            .await
            .map_err(|error| {
                AppError::ValidationError(
                    match error {
                        AppError::Internal(_) => "gateway_configuration_required",
                        _ => "gateway_unavailable",
                    }
                    .into(),
                )
            })?;
    validated_base(&row.base_url)
}

pub(crate) fn validated_base(value: &str) -> AppResult<String> {
    let base = value.trim_end_matches('/').to_owned();
    let parsed = url::Url::parse(&base)
        .map_err(|_| AppError::ValidationError("gateway_configuration_required".into()))?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(parsed.scheme() == "https" || (local && parsed.scheme() == "http"))
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::ValidationError(
            "gateway_configuration_required".into(),
        ));
    }
    Ok(base)
}

pub(crate) fn availability_code(error: &AppError) -> &'static str {
    if matches!(error, AppError::ValidationError(code) if code == "gateway_configuration_required")
    {
        "gateway_configuration_required"
    } else {
        "gateway_unavailable"
    }
}

const CREATOR_GATEWAY_PERMISSIONS: &[&str] = &[
    "cma-eg:channels:read",
    "cma-eg:channels:create",
    "cma-eg:channels:update",
    "cma-eg:channels:delete",
];

/// Match only the unrestricted, short-lived server-side creator mint.
pub(crate) fn is_creator_claims(claims: &Claims) -> bool {
    let Claims {
        // JWT verification and live authority/RBAC resolution own these identity
        // fields; they do not distinguish the creator mint.
        sub: _,
        iss: _,
        aud: _,
        jti: _,
        exp,
        iat,
        scope,
        token_type,
        delegated: Some(true),
        delegation_origin: Some(DelegationOrigin::Service),
        act: Some(actor),
        client_id: None,
        oauth_lineage: None,
        protected_consent: None,
        sid: None,
        sgen: None,
        cnf: None,
        assistant_forward: None,
        sa: None,
        relay: None,
        relay_api_key_id: None,
        relay_guest_assignment: None,
        relay_api_key_name: None,
        relay_allowed_service_ids: None,
        relay_allowed_node_ids: None,
        relay_allow_all_services: None,
        relay_allow_all_nodes: None,
        resources: None,
        allowed_service_ids: None,
        allow_all_services: None,
        allowed_node_ids: None,
        allow_all_nodes: None,
        roles: None,
        groups: None,
        permissions: None,
    } = claims
    else {
        return false;
    };
    token_type == "access"
        && actor.sub == CREATOR_ACTOR
        && scope == crate::mw::auth::ACCOUNT_READ_SCOPE
        && exp
            .checked_sub(*iat)
            .is_some_and(|ttl| ttl > 0 && ttl <= CREATOR_TOKEN_TTL_SECS)
        && *exp > chrono::Utc::now().timestamp()
}

pub(crate) async fn creator_gateway_permissions(
    db: &mongodb::Database,
    claims: &Claims,
    client: &OauthClient,
) -> AppResult<&'static [&'static str]> {
    if !is_creator_claims(claims) || !client.is_active || client.client_type != "confidential" {
        return Ok(&[]);
    }
    let gateway = super::assistant_service::resolve_admin_service_by_slug(db, GATEWAY_SLUG)
        .await
        .map_err(|_| AppError::OAuthAuthorityUnavailable)?;
    if !gateway
        .introspection_client_ids
        .as_ref()
        .is_some_and(|ids| ids.contains(&client.id))
    {
        return Ok(&[]);
    }
    Ok(CREATOR_GATEWAY_PERMISSIONS)
}

/// The owner's server-only bearer for gateway channel management. Legacy
/// gateways validate it through `/api/v1/users/me`; RFC 7662 gateways require
/// per-method `cma-eg:channels:*` permissions. Introspection grants those only
/// for this actor-scoped mint and the catalog's registered confidential caller.
/// The 120-second bearer is never logged or stored; the provider key is separate.
pub(crate) fn creator_bearer(state: &AppState, owner: &str) -> AppResult<Zeroizing<String>> {
    let user_id =
        Uuid::parse_str(owner).map_err(|_| AppError::Internal("Invalid channel owner".into()))?;
    crate::crypto::jwt::generate_nyxbot_gateway_creator_token(
        &state.jwt_keys,
        &state.config,
        &user_id,
    )
    .map(Zeroizing::new)
}

pub(crate) struct GatewayResponse {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) async fn gateway_call(
    state: &AppState,
    method: reqwest::Method,
    path: &str,
    bearer: &str,
    body: Option<&Value>,
    idempotency_key: Option<&str>,
) -> AppResult<GatewayResponse> {
    let base = gateway_base(state).await?;
    gateway_call_at_base(&base, method, path, bearer, body, idempotency_key).await
}

pub(crate) async fn gateway_call_at_base(
    base: &str,
    method: reqwest::Method,
    path: &str,
    bearer: &str,
    body: Option<&Value>,
    idempotency_key: Option<&str>,
) -> AppResult<GatewayResponse> {
    let response =
        exchange_at_base(base, method, path, bearer, body, idempotency_key, None).await?;
    let status = response
        .status
        .ok_or_else(|| AppError::ValidationError("gateway_unavailable".into()))?;
    let body = match response.body {
        Err(_) if !(200..300).contains(&status) => Value::Null,
        result => result?,
    };
    Ok(GatewayResponse { status, body })
}

struct Exchange {
    status: Option<u16>,
    body: AppResult<Value>,
    retry_after: Option<chrono::DateTime<chrono::Utc>>,
}

async fn exchange_at_base(
    base: &str,
    method: reqwest::Method,
    path: &str,
    bearer: &str,
    body: Option<&Value>,
    idempotency_key: Option<&str>,
    reply: Option<(chrono::DateTime<chrono::Utc>, bool)>,
) -> AppResult<Exchange> {
    let mut operation = crate::turn_trace::Operation::start(
        crate::turn_trace::TurnContext::current(),
        crate::turn_trace::Stage::Gateway,
    );
    let result: AppResult<Exchange> = crate::turn_trace::scope(operation.context(), async {
        let requires_json = method != reqwest::Method::DELETE && method != reqwest::Method::HEAD;
        let mut request = GATEWAY_HTTP_CLIENT
            .request(method, format!("{base}{path}"))
            .bearer_auth(bearer)
            .timeout(Duration::from_secs(GATEWAY_TIMEOUT_SECS));
        if let Some(trace) = crate::turn_trace::TurnContext::current() {
            // This request is built here; it has no caller correlation headers.
            request = request.header("traceparent", trace.traceparent()).header(
                crate::turn_trace::REQUEST_HEADER,
                trace.request_id().to_string(),
            );
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        super::channel_turn_preparation::effect().await?;
        if let Some((deadline, _)) = reply {
            let remaining = (deadline - chrono::Utc::now())
                .to_std()
                .ok()
                .filter(|duration| !duration.is_zero())
                .ok_or_else(|| AppError::Forbidden("Channel reply deadline expired".into()))?;
            request = request.timeout(remaining.min(Duration::from_secs(GATEWAY_TIMEOUT_SECS)));
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_builder() => {
                return Err(AppError::ValidationError("gateway_unavailable".into()));
            }
            Err(_) => {
                return Ok(Exchange {
                    status: None,
                    body: Err(AppError::ValidationError("gateway_unavailable".into())),
                    retry_after: None,
                });
            }
        };
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(retry_after);
        let body = gateway_body(
            response,
            requires_json,
            reply.is_some_and(|(_, target)| target),
        )
        .await;
        Ok(Exchange {
            status: Some(status),
            body,
            retry_after,
        })
    })
    .await;
    match &result {
        Ok(response) => operation.finish(
            if response.status.is_some_and(|s| (200..300).contains(&s)) && response.body.is_ok() {
                crate::turn_trace::Outcome::Completed
            } else {
                crate::turn_trace::Outcome::Failed
            },
            response.status,
            None,
        ),
        Err(error) => operation.finish(
            crate::turn_trace::Outcome::Failed,
            Some(error.status_code().as_u16()),
            Some(error.error_code()),
        ),
    }
    result
}

fn retry_after(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = chrono::Utc::now();
    if let Ok(seconds) = value.parse::<u64>() {
        // An unrepresentable delay must not cause an early request.
        return Some(now + chrono::Duration::seconds(seconds.min(86_400) as i64));
    }
    httpdate::parse_http_date(value).ok().map(Into::into)
}

pub(crate) async fn reply_call(
    state: &AppState,
    path: &str,
    bearer: &str,
    body: &Value,
    key: Option<&str>,
    target_operation: bool,
    deadline: chrono::DateTime<chrono::Utc>,
) -> AppResult<crate::models::gateway_reply::Reply> {
    let base = gateway_base(state).await?;
    let response = exchange_at_base(
        &base,
        reqwest::Method::POST,
        path,
        bearer,
        Some(body),
        key,
        Some((deadline, target_operation)),
    )
    .await?;
    Ok(classify_reply(response, target_operation))
}

fn classify_reply(
    response: Exchange,
    target_operation: bool,
) -> crate::models::gateway_reply::Reply {
    use crate::models::gateway_reply::{Code, Failure, Observation, Outcome, Reply};
    let failure = match (target_operation, response.status) {
        (true, None) => Some(Failure::NoResponse),
        (true, Some(500..=599)) => Some(Failure::ServerError),
        (true, Some(200..=299)) if response.body.is_err() => Some(Failure::UnreadableResponse),
        _ => None,
    };
    let body = response.body.unwrap_or(Value::Null);
    let observation = Observation {
        status: response.status,
        failure,
        code: Code::parse(&body["detail"]),
        retry_after: response.retry_after,
    };
    let outcome = if target_operation && observation.retryable() {
        Outcome::Pending
    } else if observation.admitted() || response.status.is_none_or(|s| s >= 500) {
        Outcome::Unknown
    } else if !response.status.is_some_and(|s| (200..300).contains(&s)) {
        Outcome::Rejected
    } else {
        match body
            .pointer("/delivery/outcome")
            .or_else(|| body.pointer("/message/delivery/outcome"))
            .and_then(Value::as_str)
        {
            Some("confirmed") => Outcome::Confirmed,
            Some("rejected") => Outcome::Rejected,
            _ => Outcome::Unknown,
        }
    };
    Reply {
        outcome,
        observation,
    }
}

async fn gateway_body(
    mut response: reqwest::Response,
    requires_json: bool,
    strict_json: bool,
) -> AppResult<Value> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_GATEWAY_RESPONSE_BYTES as u64)
    {
        return Err(AppError::ValidationError("gateway_unavailable".into()));
    }
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::ValidationError("gateway_unavailable".into()))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_GATEWAY_RESPONSE_BYTES {
            return Err(AppError::ValidationError("gateway_unavailable".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    // DELETE/HEAD and 204 replies need only status. Other successful calls
    // require JSON; callers validate their operation's identity/version fields.
    match serde_json::from_slice(&bytes) {
        Ok(body) => Ok(body),
        Err(_)
            if !requires_json
                || !status.is_success()
                || (!strict_json && status == reqwest::StatusCode::NO_CONTENT) =>
        {
            Ok(Value::Null)
        }
        Err(_) => Err(AppError::ValidationError("gateway_invalid_response".into())),
    }
}

pub(crate) async fn cleanup_platform_gateway(
    state: &AppState,
    task: &crate::models::platform_bot_config::PlatformBotCleanup,
) -> AppResult<()> {
    crate::services::platform_bot_config_service::admit_cleanup_effect(&state.db, task).await?;
    let channel = task
        .gateway_channel_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("Captured gateway identity unavailable".into()))?;
    let version = task
        .gateway_version
        .ok_or_else(|| AppError::Conflict("Captured gateway version unavailable".into()))?;
    let owner = task.owner_user_id.as_str();
    let creator = creator_bearer(state, owner)?;
    let path = format!("/channels/{}", urlencode(channel));
    let response = gateway_call(state, reqwest::Method::GET, &path, &creator, None, None)
        .await
        .map_err(|_| AppError::Conflict("Gateway cleanup is temporarily unavailable".into()))?;
    if response.status == 404 {
        return Ok(());
    }
    if response.status != 200 {
        return Err(AppError::Conflict(
            "Gateway cleanup read was refused".into(),
        ));
    }
    if response.body["version"].as_i64() != Some(version) {
        return Err(AppError::Conflict(
            "Gateway binding changed; review cleanup recovery".into(),
        ));
    }
    crate::services::platform_bot_config_service::admit_cleanup_effect(&state.db, task).await?;
    let deleted = gateway_call(
        state,
        reqwest::Method::DELETE,
        &format!("{path}?expected_version={version}"),
        &creator,
        None,
        None,
    )
    .await
    .map_err(|_| AppError::Conflict("Gateway cleanup outcome is uncertain".into()))?;
    if !matches!(deleted.status, 200 | 204 | 404) {
        return Err(AppError::Conflict("Gateway cleanup will retry".into()));
    }
    Ok(())
}

pub(crate) struct CleanupInspection {
    pub task_id: String,
    pub profile_generation: i64,
    pub remote_channel_id: String,
    pub captured_version: i64,
    pub observed_version: Option<i64>,
    pub remote_exists: bool,
    pub completed: bool,
}

async fn live_admin(state: &AppState, actor: &str) -> AppResult<()> {
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    super::platform_bot_config_service::fence_admin(&state.db, actor, &mut session).await?;
    session.commit_transaction().await?;
    Ok(())
}

async fn task(
    state: &AppState,
    profile: &str,
    id: &str,
) -> AppResult<crate::models::platform_bot_config::PlatformBotCleanup> {
    state.db.collection::<crate::models::platform_bot_config::PlatformBotCleanup>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
        .find_one(doc! {"_id": id, "platform_config_id": profile, "kind": "gateway", "capture_version": 1_i64}).await?
        .ok_or_else(|| AppError::NotFound("Gateway cleanup is unavailable".into()))
}

pub(crate) async fn list_cleanup(
    state: &AppState,
    actor: &str,
    profile: &str,
) -> AppResult<Vec<crate::models::platform_bot_config::PlatformBotCleanup>> {
    use futures::TryStreamExt;
    live_admin(state, actor).await?;
    super::platform_bot_config_service::get(&state.db, profile).await?;
    Ok(state
        .db
        .collection::<crate::models::platform_bot_config::PlatformBotCleanup>(
            crate::models::platform_bot_config::CLEANUP_COLLECTION,
        )
        .find(doc! {"platform_config_id": profile, "kind": "gateway", "completed": false})
        .sort(doc! {"created_at": 1_i64})
        .limit(20)
        .await?
        .try_collect()
        .await?)
}

pub(crate) async fn inspect_cleanup(
    state: &AppState,
    actor: &str,
    profile: &str,
    id: &str,
) -> AppResult<CleanupInspection> {
    live_admin(state, actor).await?;
    let profile = super::platform_bot_config_service::get(&state.db, profile).await?;
    let task = task(state, &profile.id, id).await?;
    let channel = task
        .gateway_channel_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("Captured gateway identity unavailable".into()))?;
    let captured_version = task
        .gateway_version
        .ok_or_else(|| AppError::Conflict("Captured gateway version unavailable".into()))?;
    let bearer = creator_bearer(state, &task.owner_user_id)?;
    let remote = gateway_call(
        state,
        reqwest::Method::GET,
        &format!("/channels/{}", urlencode(channel)),
        &bearer,
        None,
        None,
    )
    .await?;
    if !matches!(remote.status, 200 | 404) {
        return Err(AppError::Conflict(
            "Gateway inspection is temporarily unavailable".into(),
        ));
    }
    let observed_version = if remote.status == 200 {
        Some(
            remote.body["version"]
                .as_i64()
                .filter(|v| *v > 0)
                .ok_or_else(|| {
                    AppError::Conflict("Gateway returned no reviewable version".into())
                })?,
        )
    } else {
        None
    };
    live_admin(state, actor).await?;
    Ok(CleanupInspection {
        task_id: task.id,
        profile_generation: profile.generation,
        remote_channel_id: channel.to_owned(),
        captured_version,
        observed_version,
        remote_exists: remote.status == 200,
        completed: task.completed,
    })
}

/// Recovery never substitutes an observed version into a deletion request.
/// Changed bindings can only be explicitly preserved; missing bindings can close.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_cleanup(
    state: &AppState,
    actor: &str,
    profile: &str,
    id: &str,
    profile_generation: i64,
    captured_version: i64,
    observed_version: Option<i64>,
    action: &str,
) -> AppResult<()> {
    let review = inspect_cleanup(state, actor, profile, id).await?;
    if review.profile_generation != profile_generation
        || review.captured_version != captured_version
        || review.observed_version != observed_version
        || review.completed
    {
        return Err(AppError::Conflict(
            "Cleanup changed; inspect it again".into(),
        ));
    }
    let complete = match action {
        "retry_original" if observed_version == Some(captured_version) => false,
        "preserve_changed"
            if review.remote_exists && observed_version != Some(captured_version) =>
        {
            true
        }
        "confirm_missing" if !review.remote_exists => true,
        _ => {
            return Err(AppError::Conflict(
                "This resolution does not match the inspected binding".into(),
            ));
        }
    };
    let db = state.db.clone();
    let actor = actor.to_owned();
    let profile = profile.to_owned();
    let id = id.to_owned();
    let action = action.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            super::platform_bot_config_service::fence_admin(&db, &actor, session).await?;
            if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"_id": &profile, "generation": profile_generation},
                doc! {"$inc": {"cleanup_review_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 {return Err(AppError::Conflict("Platform configuration changed".into()));}
            if db.collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION).update_one(doc! {"_id": &id, "platform_config_id": &profile, "kind": "gateway", "completed": false,
                "gateway_channel_id": &review.remote_channel_id, "gateway_version": captured_version},
                doc! {"$set": {"completed": complete, "claim_id": bson::Bson::Null, "retry_at": bson::DateTime::now(), "reviewed_resolution": &action,
                    "reviewed_by": &actor, "reviewed_remote_version": observed_version}}).session(&mut *session).await?.matched_count != 1 {return Err(AppError::Conflict("Cleanup changed".into()));}
            Ok(())
        }.await; super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_bot_config::{CLEANUP_COLLECTION, PlatformBotCleanup};
    use crate::services::{
        platform_bot_config_service as configs, platform_onboarding_test_support as support,
    };
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path},
    };
    #[tokio::test]
    async fn turn_trace_gateway_headers_and_allowlisted_body_outcome() {
        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/events/private-capability/replies"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"content":"private-response-canary"})),
            )
            .mount(&server)
            .await;
        let turn = Uuid::new_v4().to_string();
        let context = crate::turn_trace::TurnContext::for_turn(&turn).unwrap();
        let ((first, second), records) =
            crate::turn_trace::capture(crate::turn_trace::scope(Some(context), async {
                let first = gateway_call_at_base(
                    &server.uri(),
                    reqwest::Method::POST,
                    "/events/private-capability/replies",
                    "private-bearer-canary",
                    Some(&serde_json::json!({"text":"private-body-canary"})),
                    Some("unchanged-dedup"),
                )
                .await
                .unwrap();
                let second = gateway_call_at_base(
                    &server.uri(),
                    reqwest::Method::POST,
                    "/events/private-capability/replies",
                    "private-bearer-canary",
                    None,
                    Some("unchanged-dedup"),
                )
                .await
                .unwrap();
                (first, second)
            }))
            .await;
        assert_eq!((first.status, second.status), (200, 200));
        let requests = server.received_requests().await.unwrap();
        for request in &requests {
            assert!(
                request.headers["traceparent"]
                    .to_str()
                    .unwrap()
                    .starts_with(&format!("00-{}-", turn.replace('-', "")))
            );
            assert_eq!(request.headers["idempotency-key"], "unchanged-dedup");
        }
        assert_ne!(
            requests[0].headers["x-request-id"],
            requests[1].headers["x-request-id"]
        );
        assert_eq!(records.len(), 4);
        assert_eq!(records[1]["outcome"], "completed");
        let encoded = serde_json::to_string(&records).unwrap();
        assert!(!encoded.contains("private-"));
        assert!(!encoded.contains("confirmed"));
    }

    async fn fixture() -> (support::Fixture, PlatformBotCleanup) {
        let f = support::distinct_fixture().await;
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = GATEWAY_SLUG.into();
        service.base_url = f.server.uri();
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .insert_one(service)
            .await
            .unwrap();
        let task = PlatformBotCleanup {
            id: uuid::Uuid::new_v4().to_string(),
            platform_config_id: f.profile.id.clone(),
            kind: "gateway".into(),
            resource_id: uuid::Uuid::new_v4().to_string(),
            owner_user_id: f.operator.clone(),
            capture_version: 1,
            turn_id: None,
            async_operation_ids: vec![],
            machine_jobs: vec![],
            machine_node_ids: vec![],
            gateway_channel_id: Some("old-remote".into()),
            gateway_version: Some(7),
            record_id: None,
            binding_id: None,
            agent_api_key_id: None,
            completed: false,
            claim_id: None,
            // This fixture is immediately claimable against MongoDB's $$NOW,
            // independently of small host/database clock differences.
            retry_at: chrono::DateTime::UNIX_EPOCH,
            created_at: chrono::Utc::now(),
        };
        f.state
            .db
            .collection::<PlatformBotCleanup>(CLEANUP_COLLECTION)
            .insert_one(task)
            .await
            .unwrap();
        let claimed = configs::claim_cleanup(&f.state.db).await.unwrap().unwrap();
        assert_eq!(claimed.kind, "gateway");
        (f, claimed)
    }
    async fn remote(f: &support::Fixture, version: Option<i64>, priority: u8) {
        let response = match version {
            Some(version) => {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"version": version}))
            }
            None => ResponseTemplate::new(404),
        };
        Mock::given(method("GET"))
            .and(path("/channels/old-remote"))
            .respond_with(response)
            .with_priority(priority)
            .mount(&f.server)
            .await;
    }
    #[tokio::test]
    async fn platform_onboarding_gateway_review_drift_preserve_and_stale_worker_are_fenced() {
        let (f, old) = fixture().await;
        remote(&f, Some(8), 2).await;
        let review = inspect_cleanup(&f.state, &f.operator, &f.profile.id, &old.id)
            .await
            .unwrap();
        assert_eq!(review.observed_version, Some(8));
        remote(&f, Some(9), 1).await;
        assert!(
            resolve_cleanup(
                &f.state,
                &f.operator,
                &f.profile.id,
                &old.id,
                review.profile_generation,
                7,
                Some(8),
                "preserve_changed"
            )
            .await
            .is_err()
        );
        assert!(
            !f.state
                .db
                .collection::<PlatformBotCleanup>(CLEANUP_COLLECTION)
                .find_one(doc! {"_id": &old.id})
                .await
                .unwrap()
                .unwrap()
                .completed
        );
        assert!(
            resolve_cleanup(
                &f.state,
                &f.operator,
                &f.profile.id,
                &old.id,
                review.profile_generation,
                7,
                Some(9),
                "retry_original"
            )
            .await
            .is_err()
        );
        resolve_cleanup(
            &f.state,
            &f.operator,
            &f.profile.id,
            &old.id,
            review.profile_generation,
            7,
            Some(9),
            "preserve_changed",
        )
        .await
        .unwrap();
        assert!(
            configs::admit_cleanup_effect(&f.state.db, &old)
                .await
                .is_err()
        );
        assert!(cleanup_platform_gateway(&f.state, &old).await.is_err());
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method != "DELETE")
        );
    }
    #[tokio::test]
    async fn platform_onboarding_gateway_missing_resolution_and_admin_demotion_do_not_delete() {
        let (f, old) = fixture().await;
        remote(&f, None, 1).await;
        let review = inspect_cleanup(&f.state, &f.operator, &f.profile.id, &old.id)
            .await
            .unwrap();
        assert!(!review.remote_exists);
        let roles = f
            .state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id": &f.operator})
            .await
            .unwrap()
            .unwrap()
            .get_array("role_ids")
            .unwrap()
            .clone();
        f.state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.operator},
                doc! {"$set": {"role_ids": [], "is_admin": true}},
            )
            .await
            .unwrap();
        assert!(
            resolve_cleanup(
                &f.state,
                &f.operator,
                &f.profile.id,
                &old.id,
                review.profile_generation,
                7,
                None,
                "confirm_missing"
            )
            .await
            .is_err()
        );
        f.state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.operator},
                doc! {"$set": {"role_ids": roles}},
            )
            .await
            .unwrap();
        resolve_cleanup(
            &f.state,
            &f.operator,
            &f.profile.id,
            &old.id,
            review.profile_generation,
            7,
            None,
            "confirm_missing",
        )
        .await
        .unwrap();
        assert!(
            configs::admit_cleanup_effect(&f.state.db, &old)
                .await
                .is_err()
        );
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method != "DELETE")
        );
    }
    #[tokio::test]
    async fn platform_onboarding_gateway_retry_keeps_original_version_and_fences_old_claim() {
        let (f, old) = fixture().await;
        remote(&f, Some(7), 2).await;
        let review = inspect_cleanup(&f.state, &f.operator, &f.profile.id, &old.id)
            .await
            .unwrap();
        resolve_cleanup(
            &f.state,
            &f.operator,
            &f.profile.id,
            &old.id,
            review.profile_generation,
            7,
            Some(7),
            "retry_original",
        )
        .await
        .unwrap();
        assert!(
            configs::admit_cleanup_effect(&f.state.db, &old)
                .await
                .is_err()
        );
        let row = f
            .state
            .db
            .collection::<Document>(CLEANUP_COLLECTION)
            .find_one(doc! {"_id": &old.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!row.get_bool("completed").unwrap());
        assert_eq!(row.get_i64("gateway_version").unwrap(), 7);
        assert!(matches!(row.get("claim_id"), Some(bson::Bson::Null)));
        assert!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method != "DELETE")
        );
    }
}

#[cfg(test)]
#[path = "nyxbot_gateway_creator_tests.rs"]
mod creator_tests;

#[cfg(test)]
#[path = "nyxbot_gateway_http_tests.rs"]
mod http_tests;

use crate::models::gateway_reply::{Code, Observation};
impl Code {
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}
impl Observation {
    pub fn status_class(&self) -> Option<&'static str> {
        self.status.map(|status| match status {
            200..=299 => "2xx",
            300..=399 => "3xx",
            400..=499 => "4xx",
            500..=599 => "5xx",
            _ => "other",
        })
    }
    pub fn admitted(&self) -> bool {
        self.status == Some(409) && self.code == Some(Code::RoutedReplyInProgress)
    }
    pub fn retryable(&self) -> bool {
        self.admitted()
            || self.failure.is_some()
            || (self.status == Some(503) && self.code == Some(Code::RoutedReplyUnavailable))
    }
}

#[cfg(test)]
mod reply_contract_tests {
    use super::*;
    use crate::models::gateway_reply::Outcome;
    use serde_json::json;

    #[test]
    fn gateway_reply_closed_codes_and_received_status_survive_broken_bodies() {
        for (status, body, target, expected) in [
            (
                409,
                json!({"detail":"routed_reply_in_progress"}),
                true,
                Outcome::Pending,
            ),
            (
                503,
                json!({"detail":"routed_reply_unavailable"}),
                true,
                Outcome::Pending,
            ),
            (
                503,
                json!({"detail":"routed_reply_unavailable"}),
                false,
                Outcome::Unknown,
            ),
            (
                500,
                json!({"detail":"routed_reply_unavailable"}),
                true,
                Outcome::Pending,
            ),
            (
                503,
                json!({"detail":"routed_reply_unavailable "}),
                true,
                Outcome::Pending,
            ),
            (
                409,
                json!({"detail":"routed_reply_in_progress "}),
                true,
                Outcome::Rejected,
            ),
            (
                429,
                json!({"detail":"reply_operation_limit"}),
                true,
                Outcome::Rejected,
            ),
            (
                409,
                json!({"detail":"reply_idempotency_conflict"}),
                true,
                Outcome::Rejected,
            ),
            (
                200,
                json!({"message":{"delivery":{"outcome":"confirmed"}}}),
                true,
                Outcome::Confirmed,
            ),
            (
                200,
                json!({"delivery":{"outcome":"unknown"}}),
                true,
                Outcome::Unknown,
            ),
        ] {
            let reply = classify_reply(
                Exchange {
                    status: Some(status),
                    body: Ok(body),
                    retry_after: None,
                },
                target,
            );
            assert!(reply.outcome == expected);
            assert_eq!(reply.observation.status, Some(status));
            assert_eq!(
                reply.observation.admitted(),
                status == 409 && expected == Outcome::Pending
            );
        }
        let reply = classify_reply(
            Exchange {
                status: Some(200),
                body: Err(AppError::Internal("broken".into())),
                retry_after: None,
            },
            true,
        );
        assert!(reply.outcome == Outcome::Pending);
        assert_eq!(reply.observation.status_class(), Some("2xx"));
        assert!(reply.observation.code.is_none());
    }

    #[test]
    fn gateway_reply_retry_after_delta_date_and_invalid() {
        let now = chrono::Utc::now();
        assert!(retry_after("120").unwrap() >= now + chrono::Duration::seconds(120));
        assert!(retry_after("invalid").is_none());
        assert!(retry_after("-1").is_none());
        let date = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(600));
        assert!(retry_after(&date).unwrap() > now + chrono::Duration::seconds(598));
        assert!(retry_after("18446744073709551615").unwrap() >= now + chrono::Duration::hours(24));
    }
}

// CMA 70761e9b returns UnknownProvider's inner slug verbatim. Retain the
// explicit historical response too; a validation pointer/prose is not evidence.
pub(crate) fn unknown_nyxbot_provider(body: &Value) -> bool {
    body["code"] == "invalid_request"
        && matches!(
            body["detail"].as_str().map(str::trim),
            Some("nyxbot" | "unknown provider: nyxbot" | "provider is not registered")
        )
}

/// Public management problem codes only; never retain arbitrary upstream code
/// strings (which may contain identifiers) in setup diagnostics or their audit.
pub(crate) fn setup_problem_code(value: &str) -> Option<&'static str> {
    match value {
        "invalid_request" => Some("invalid_request"),
        "provider_rejected" => Some("provider_rejected"),
        "not_authenticated" => Some("not_authenticated"),
        "forbidden" => Some("forbidden"),
        "channel_not_found" => Some("channel_not_found"),
        "conflict" => Some("conflict"),
        "rate_limited" => Some("rate_limited"),
        "provider_unavailable" => Some("provider_unavailable"),
        "internal" => Some("internal"),
        "provider_busy" => Some("provider_busy"),
        "draining" => Some("draining"),
        _ => None,
    }
}
