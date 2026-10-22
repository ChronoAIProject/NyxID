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
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("gateway HTTP client")
}
fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
pub(crate) async fn gateway_base(state: &AppState) -> AppResult<String> {
    let row =
        crate::services::assistant_service::resolve_admin_service_by_slug(&state.db, GATEWAY_SLUG)
            .await
            .map_err(|_| AppError::ValidationError("gateway_unavailable".into()))?;
    let base = row.base_url.trim_end_matches('/').to_owned();
    let parsed = url::Url::parse(&base)
        .map_err(|_| AppError::ValidationError("gateway_unavailable".into()))?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(parsed.scheme() == "https" || (local && parsed.scheme() == "http")) {
        return Err(AppError::ValidationError("gateway_unavailable".into()));
    }
    Ok(base)
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
    let requires_json = method != reqwest::Method::DELETE && method != reqwest::Method::HEAD;
    let mut request = GATEWAY_HTTP_CLIENT
        .request(method, format!("{base}{path}"))
        .bearer_auth(bearer)
        .timeout(Duration::from_secs(GATEWAY_TIMEOUT_SECS));
    if let Some(body) = body {
        request = request.json(body);
    }
    if let Some(key) = idempotency_key {
        request = request.header("Idempotency-Key", key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| AppError::ValidationError("gateway_unavailable".into()))?;
    let status = response.status().as_u16();
    let body = gateway_body(response, requires_json).await;
    // HTTP refusals remain classified by status even if their diagnostic body
    // cannot be read. Never parse a partial body or turn a broken 2xx into success.
    let body = match body {
        Err(_) if !(200..300).contains(&status) => Value::Null,
        result => result?,
    };
    Ok(GatewayResponse { status, body })
}

async fn gateway_body(mut response: reqwest::Response, requires_json: bool) -> AppResult<Value> {
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
                || status == reqwest::StatusCode::NO_CONTENT =>
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
            completed: false,
            claim_id: None,
            retry_at: chrono::Utc::now(),
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
