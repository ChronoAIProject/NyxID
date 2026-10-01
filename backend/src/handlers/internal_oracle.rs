//! Internal identity endpoints for the standalone oracle service.
//!
//! Mounted on the private listener (`INTERNAL_BIND_ADDR`) only when
//! `ORACLE_INTERNAL_SECRET` is set. Every request must carry
//! `Authorization: Bearer <ORACLE_INTERNAL_SECRET>`; the comparison is
//! constant time. Responses are never cacheable.
//!
//! The oracle service keeps no users, keys or org memberships of its own. It
//! asks these endpoints and caches the answers for a few seconds, see
//! `docs/ORACLE_RELAY.md`, "Standalone oracle service".

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequestParts, Path, Request, State},
    http::{HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::SecondsFormat;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::crypto::jwt;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS};
use crate::models::org_membership::OrgMembership;
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::mw::auth::{AuthMethod, AuthUser, is_jwt_delegated};
use crate::services::{audit_service, org_service};

/// Oracle consumer path used to run the `AuthUser` extractor. Path-based
/// policy (API-key purpose, scheduled keys, service-account route gates)
/// therefore matches what a real `/api/v1/oracle` request sees today.
const CONSUMER_PROBE_URI: &str = "/api/v1/oracle/pools";

/// Delegated tokens are refused on every oracle path, so the first probe
/// cannot tell a valid delegated token from garbage. A second probe on a
/// delegated-native path verifies the signature and user so the response
/// can say `token_class: "delegated"` and let the oracle service answer 403.
const DELEGATED_PROBE_URI: &str = "/api/v1/delegation/oracle-introspect";

/// Internal bodies are small JSON documents.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Build the internal oracle router. `state.config.oracle_internal_secret`
/// must be set; the caller mounts this only in that case.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/internal/oracle/introspect", post(introspect))
        .route(
            "/api/v1/internal/oracle/principals/{user_id}",
            get(principal),
        )
        .route("/api/v1/internal/oracle/audit", post(audit))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_internal_secret,
        ))
        .layer(middleware::map_response(no_store))
        .with_state(state)
}

/// Constant-time bearer check against `ORACLE_INTERNAL_SECRET`.
async fn require_internal_secret(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let expected = state.config.oracle_internal_secret.as_deref();
    let authorized = match (presented, expected) {
        (Some(presented), Some(expected)) => {
            bool::from(presented.as_bytes().ct_eq(expected.as_bytes()))
        }
        _ => false,
    };
    if !authorized {
        return AppError::Unauthorized("Invalid internal secret".to_string()).into_response();
    }
    next.run(request).await
}

async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

// ---------------------------------------------------------------------------
// Shared response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MembershipInfo {
    pub org_user_id: String,
    pub membership_id: String,
    pub role: &'static str,
    /// RFC 3339 with millisecond precision, the precision MongoDB stores.
    pub created_at: String,
    pub active: bool,
}

impl From<&OrgMembership> for MembershipInfo {
    fn from(row: &OrgMembership) -> Self {
        Self {
            org_user_id: row.org_user_id.clone(),
            membership_id: row.id.clone(),
            role: row.role.as_str(),
            created_at: row.created_at.to_rfc3339_opts(SecondsFormat::Millis, false),
            active: row.is_active(),
        }
    }
}

/// Principal classes reported to the oracle service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrincipalKind {
    Person,
    Org,
    ServiceAccount,
}

impl PrincipalKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Org => "org",
            Self::ServiceAccount => "service_account",
        }
    }
}

/// Every membership row that involves `user_id`: the orgs a person belongs
/// to, or the members of an org. Revoked rows are included with
/// `active: false` so the oracle service can match enrolment timestamps.
async fn memberships_for(
    db: &mongodb::Database,
    user_id: &str,
    kind: PrincipalKind,
) -> AppResult<Vec<MembershipInfo>> {
    let rows = match kind {
        PrincipalKind::Person => {
            org_service::list_memberships_for_member(db, user_id, true).await?
        }
        PrincipalKind::Org => org_service::list_members_for_org(db, user_id, true).await?,
        PrincipalKind::ServiceAccount => Vec::new(),
    };
    Ok(rows.iter().map(MembershipInfo::from).collect())
}

fn principal_kind(user: &User) -> PrincipalKind {
    match user.user_type {
        UserType::Person => PrincipalKind::Person,
        UserType::Org => PrincipalKind::Org,
    }
}

// ---------------------------------------------------------------------------
// POST /api/v1/internal/oracle/introspect
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct IntrospectRequest {
    /// The `Authorization` header exactly as the client sent it.
    #[serde(default)]
    pub authorization: Option<String>,
    /// The `X-API-Key` header exactly as the client sent it.
    #[serde(default)]
    pub x_api_key: Option<String>,
    /// Client IP for audit attribution.
    #[serde(default)]
    pub ip: Option<String>,
    /// Client User-Agent for audit attribution.
    #[serde(default)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IntrospectPrincipal {
    pub user_id: String,
    pub user_type: &'static str,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct IntrospectApiKey {
    pub id: String,
    pub name: String,
    pub purpose: &'static str,
}

#[derive(Debug, Serialize)]
pub struct IntrospectResponse {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_class: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<IntrospectPrincipal>,
    /// Space-separated scopes; `null` for a session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Present (possibly `null`) on every active answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<Option<IntrospectApiKey>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memberships: Option<Vec<MembershipInfo>>,
    /// Token expiry as Unix seconds; `null` for a key without expiry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<Option<i64>>,
}

impl IntrospectResponse {
    fn inactive(reason: &'static str) -> Self {
        Self {
            active: false,
            reason: Some(reason),
            token_class: None,
            principal: None,
            scope: None,
            api_key: None,
            memberships: None,
            exp: None,
        }
    }
}

/// Map the extractor's classification to the wire `token_class`.
pub fn token_class(method: &AuthMethod) -> &'static str {
    match method {
        AuthMethod::Session | AuthMethod::AccessToken => "session",
        AuthMethod::ApiKey => "api_key",
        AuthMethod::ServiceAccount => "service_account",
        AuthMethod::Delegated => "delegated",
        AuthMethod::Relay => "relay",
    }
}

fn api_key_purpose(purpose: ApiKeyPurpose) -> &'static str {
    match purpose {
        ApiKeyPurpose::General => "general",
        ApiKeyPurpose::ScheduledInvocation => "scheduled_invocation",
    }
}

/// Map an extractor failure to the wire `reason`.
fn inactive_reason(err: &AppError) -> &'static str {
    match err {
        AppError::TokenExpired => "expired",
        AppError::Unauthorized(message) if message.contains("inactive") => "inactive_user",
        AppError::Unauthorized(message) if message.contains("revoked") => "revoked",
        _ => "invalid",
    }
}

/// Bearer value of an `Authorization` header (`Bearer` or `DPoP` scheme).
fn bearer_value(authorization: &str) -> Option<&str> {
    authorization
        .strip_prefix("Bearer ")
        .or_else(|| authorization.strip_prefix("DPoP "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Run the real `AuthUser` extractor over synthetic request parts that carry
/// only the forwarded credential headers.
async fn authenticate_credentials(
    state: &AppState,
    body: &IntrospectRequest,
    uri: &'static str,
) -> AppResult<AuthUser> {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    for (name, value) in [
        (header::AUTHORIZATION, body.authorization.as_deref()),
        (
            header::HeaderName::from_static("x-api-key"),
            body.x_api_key.as_deref(),
        ),
        (
            header::HeaderName::from_static("x-forwarded-for"),
            body.ip.as_deref(),
        ),
        (header::USER_AGENT, body.user_agent.as_deref()),
    ] {
        if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
            let value = HeaderValue::from_str(value)
                .map_err(|_| AppError::Unauthorized("Invalid credential header".to_string()))?;
            builder = builder.header(name, value);
        }
    }
    let request = builder
        .body(())
        .map_err(|_| AppError::Unauthorized("Invalid credential header".to_string()))?;
    let (mut parts, ()) = request.into_parts();
    let original: Uri = uri.parse().expect("static probe uri parses");
    parts
        .extensions
        .insert(axum::extract::OriginalUri(original));
    AuthUser::from_request_parts(&mut parts, state).await
}

/// Expiry of the presented credential, if it has one.
async fn credential_expiry(
    state: &AppState,
    body: &IntrospectRequest,
    user: &AuthUser,
) -> AppResult<Option<i64>> {
    if user.auth_method == AuthMethod::ApiKey {
        let Some(key_id) = user.api_key_id.as_deref() else {
            return Ok(None);
        };
        let key = state
            .db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": key_id })
            .await?;
        return Ok(key.and_then(|k| k.expires_at).map(|t| t.timestamp()));
    }
    let Some(token) = body.authorization.as_deref().and_then(bearer_value) else {
        return Ok(None);
    };
    Ok(jwt::verify_token(&state.jwt_keys, &state.config, token)
        .ok()
        .map(|claims| claims.exp))
}

pub async fn introspect(
    State(state): State<AppState>,
    Json(body): Json<IntrospectRequest>,
) -> AppResult<Json<IntrospectResponse>> {
    let has_credential = body
        .authorization
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
        || body
            .x_api_key
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty());
    if !has_credential {
        return Ok(Json(IntrospectResponse::inactive("invalid")));
    }

    let mut outcome = authenticate_credentials(&state, &body, CONSUMER_PROBE_URI).await;
    if matches!(outcome, Err(AppError::Forbidden(_)))
        && body
            .authorization
            .as_deref()
            .and_then(bearer_value)
            .is_some_and(is_jwt_delegated)
    {
        outcome = authenticate_credentials(&state, &body, DELEGATED_PROBE_URI).await;
    }

    let user = match outcome {
        Ok(user) => user,
        Err(err @ (AppError::Internal(_) | AppError::DatabaseError(_))) => return Err(err),
        Err(err) => {
            tracing::debug!(error = %err, "oracle introspect rejected credential");
            return Ok(Json(IntrospectResponse::inactive(inactive_reason(&err))));
        }
    };

    let user_id = user.user_id.to_string();
    let kind = if user.auth_method == AuthMethod::ServiceAccount {
        PrincipalKind::ServiceAccount
    } else {
        let Some(row) = state
            .db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await?
        else {
            return Ok(Json(IntrospectResponse::inactive("invalid")));
        };
        if !row.is_active {
            return Ok(Json(IntrospectResponse::inactive("inactive_user")));
        }
        principal_kind(&row)
    };

    let memberships = memberships_for(&state.db, &user_id, kind).await?;
    let exp = credential_expiry(&state, &body, &user).await?;
    let api_key = match (&user.auth_method, user.api_key_id.as_deref()) {
        (AuthMethod::ApiKey, Some(id)) => Some(IntrospectApiKey {
            id: id.to_string(),
            name: user.api_key_name.clone().unwrap_or_default(),
            purpose: api_key_purpose(user.api_key_purpose),
        }),
        _ => None,
    };

    Ok(Json(IntrospectResponse {
        active: true,
        reason: None,
        token_class: Some(token_class(&user.auth_method)),
        principal: Some(IntrospectPrincipal {
            user_id,
            user_type: kind.as_str(),
            is_active: true,
        }),
        scope: Some(user.scope.clone()).filter(|s| !s.trim().is_empty()),
        api_key: Some(api_key),
        memberships: Some(memberships),
        exp: Some(exp),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/v1/internal/oracle/principals/{user_id}
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PrincipalResponse {
    pub user_id: String,
    pub user_type: &'static str,
    pub is_active: bool,
    pub memberships: Vec<MembershipInfo>,
}

pub async fn principal(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<PrincipalResponse>> {
    if let Some(user) = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
    {
        let kind = principal_kind(&user);
        let memberships = memberships_for(&state.db, &user_id, kind).await?;
        return Ok(Json(PrincipalResponse {
            user_id,
            user_type: kind.as_str(),
            is_active: user.is_active,
            memberships,
        }));
    }

    if let Some(sa) = state
        .db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "_id": &user_id })
        .await?
    {
        return Ok(Json(PrincipalResponse {
            user_id,
            user_type: PrincipalKind::ServiceAccount.as_str(),
            is_active: sa.is_active,
            memberships: Vec::new(),
        }));
    }

    Ok(Json(PrincipalResponse {
        user_id,
        user_type: "missing",
        is_active: false,
        memberships: Vec::new(),
    }))
}

// ---------------------------------------------------------------------------
// POST /api/v1/internal/oracle/audit
// ---------------------------------------------------------------------------

const MAX_EVENT_TYPE_LEN: usize = 128;

#[derive(Debug, Deserialize)]
pub struct AuditForwardRequest {
    pub event_type: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub api_key_id: Option<String>,
    #[serde(default)]
    pub api_key_name: Option<String>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub event_data: Option<serde_json::Value>,
}

/// Best-effort forwarded audit event. The write is fire-and-forget, exactly
/// like `audit_service::log_for_user`, with the actor fields the oracle
/// service recorded at request time.
pub async fn audit(
    State(state): State<AppState>,
    Json(body): Json<AuditForwardRequest>,
) -> AppResult<StatusCode> {
    let event_type = body.event_type.trim();
    if event_type.is_empty() || event_type.len() > MAX_EVENT_TYPE_LEN {
        return Err(AppError::BadRequest(format!(
            "event_type must be 1 to {MAX_EVENT_TYPE_LEN} characters"
        )));
    }
    audit_service::log_async(
        state.db.clone(),
        body.user_id.filter(|v| !v.trim().is_empty()),
        event_type.to_string(),
        body.event_data,
        body.ip,
        body.user_agent,
        body.api_key_id,
        body.api_key_name,
    );
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use mongodb::bson::doc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_app_state_no_db, test_membership, test_user,
    };

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn with_secret(mut state: AppState) -> AppState {
        state.config.oracle_internal_secret = Some(SECRET.to_string());
        state
    }

    async fn call(
        app: &Router,
        request: Request<Body>,
    ) -> (StatusCode, HeaderValue, serde_json::Value) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let cache = response
            .headers()
            .get(header::CACHE_CONTROL)
            .cloned()
            .unwrap_or_else(|| HeaderValue::from_static("missing"));
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = if body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (status, cache, json)
    }

    fn introspect_request(secret: Option<&str>, body: serde_json::Value) -> Request<Body> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/internal/oracle/introspect")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(secret) = secret {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    fn principal_request(secret: Option<&str>, user_id: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method(Method::GET)
            .uri(format!("/api/v1/internal/oracle/principals/{user_id}"));
        if let Some(secret) = secret {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn token_class_mapping() {
        assert_eq!(token_class(&AuthMethod::Session), "session");
        assert_eq!(token_class(&AuthMethod::AccessToken), "session");
        assert_eq!(token_class(&AuthMethod::ApiKey), "api_key");
        assert_eq!(token_class(&AuthMethod::ServiceAccount), "service_account");
        assert_eq!(token_class(&AuthMethod::Delegated), "delegated");
        assert_eq!(token_class(&AuthMethod::Relay), "relay");
    }

    #[test]
    fn inactive_reason_mapping() {
        assert_eq!(inactive_reason(&AppError::TokenExpired), "expired");
        assert_eq!(
            inactive_reason(&AppError::Unauthorized("User account is inactive".into())),
            "inactive_user"
        );
        assert_eq!(
            inactive_reason(&AppError::Unauthorized("Session expired or revoked".into())),
            "revoked"
        );
        assert_eq!(
            inactive_reason(&AppError::Unauthorized("Invalid token".into())),
            "invalid"
        );
        assert_eq!(inactive_reason(&AppError::Forbidden("x".into())), "invalid");
    }

    #[test]
    fn membership_info_shape() {
        let mut row = test_membership("org-1", "person-1", OrgRole::Viewer, None);
        row.created_at = chrono::DateTime::parse_from_rfc3339("2026-03-01T10:00:00.250Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let info = MembershipInfo::from(&row);
        assert_eq!(info.org_user_id, "org-1");
        assert_eq!(info.membership_id, row.id);
        assert_eq!(info.role, "viewer");
        assert_eq!(info.created_at, "2026-03-01T10:00:00.250+00:00");
        assert!(info.active);
        row.revoked_at = Some(chrono::Utc::now());
        assert!(!MembershipInfo::from(&row).active);
    }

    #[tokio::test]
    async fn rejects_missing_and_wrong_secret() {
        let app = router(with_secret(test_app_state_no_db().await));

        let (status, cache, json) = call(&app, principal_request(None, "u1")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{json}");
        assert_eq!(cache, "no-store");
        assert_eq!(json["error"], "unauthorized");

        // Same length, last character differs.
        let wrong = format!("{}0", &SECRET[..63]);
        let (status, _, json) = call(&app, principal_request(Some(&wrong), "u1")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(json["error"], "unauthorized");

        let (status, _, _) = call(&app, principal_request(Some(""), "u1")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _, json) = call(
            &app,
            introspect_request(None, serde_json::json!({"authorization": "Bearer x"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(json["error"], "unauthorized");
    }

    #[tokio::test]
    async fn introspect_without_credential_is_inactive() {
        let app = router(with_secret(test_app_state_no_db().await));
        let (status, cache, json) = call(
            &app,
            introspect_request(Some(SECRET), serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cache, "no-store");
        assert_eq!(
            json,
            serde_json::json!({"active": false, "reason": "invalid"})
        );

        let (status, _, json) = call(
            &app,
            introspect_request(Some(SECRET), serde_json::json!({"authorization": "  "})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["active"], false);
    }

    #[tokio::test]
    async fn audit_requires_event_type() {
        let app = router(with_secret(test_app_state_no_db().await));
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/internal/oracle/audit")
            .header(header::AUTHORIZATION, format!("Bearer {SECRET}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"event_type": "  "}"#))
            .unwrap();
        let (status, _, json) = call(&app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "bad_request");
    }

    #[tokio::test]
    async fn introspect_classifies_session_api_key_delegated_and_expired() {
        let Some(db) = connect_test_database("h_internal_oracle_introspect").await else {
            return;
        };
        let state = with_secret(test_app_state(db.clone()));
        let person = Uuid::new_v4();
        let org = Uuid::new_v4();
        db.collection::<User>(USERS)
            .insert_one(test_user(&person.to_string(), UserType::Person))
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org.to_string(), UserType::Org))
            .await
            .unwrap();
        let membership =
            test_membership(&org.to_string(), &person.to_string(), OrgRole::Admin, None);
        let mut revoked = test_membership(
            &Uuid::new_v4().to_string(),
            &person.to_string(),
            OrgRole::Member,
            None,
        );
        revoked.revoked_at = Some(chrono::Utc::now());
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([membership.clone(), revoked.clone()])
            .await
            .unwrap();

        let app = router(state.clone());

        // Session JWT.
        let session = jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &person,
            "account:read",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (status, cache, json) = call(
            &app,
            introspect_request(
                Some(SECRET),
                serde_json::json!({"authorization": format!("Bearer {session}"), "ip": "203.0.113.9"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(cache, "no-store");
        assert_eq!(json["active"], true);
        assert_eq!(json["token_class"], "session");
        assert_eq!(json["principal"]["user_id"], person.to_string());
        assert_eq!(json["principal"]["user_type"], "person");
        assert_eq!(json["principal"]["is_active"], true);
        assert_eq!(json["scope"], "account:read");
        assert!(json["api_key"].is_null());
        assert!(json["exp"].as_i64().unwrap() > chrono::Utc::now().timestamp());
        let memberships = json["memberships"].as_array().unwrap();
        assert_eq!(memberships.len(), 2);
        let active = memberships
            .iter()
            .find(|m| m["membership_id"] == membership.id)
            .unwrap();
        assert_eq!(active["org_user_id"], org.to_string());
        assert_eq!(active["role"], "admin");
        assert_eq!(active["active"], true);
        assert!(
            chrono::DateTime::parse_from_rfc3339(active["created_at"].as_str().unwrap()).is_ok()
        );
        let gone = memberships
            .iter()
            .find(|m| m["membership_id"] == revoked.id)
            .unwrap();
        assert_eq!(gone["active"], false);

        // Agent API key, sent as X-API-Key and as a bearer.
        let created = crate::services::key_service::create_api_key(
            &db,
            &person.to_string(),
            "ci-bot",
            "proxy",
            None,
            None,
            None,
            None,
            Some(true),
            None,
            Some(true),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        for body in [
            serde_json::json!({"x_api_key": created.full_key}),
            serde_json::json!({"authorization": format!("Bearer {}", created.full_key)}),
        ] {
            let (status, _, json) = call(&app, introspect_request(Some(SECRET), body)).await;
            assert_eq!(status, StatusCode::OK, "{json}");
            assert_eq!(json["active"], true, "{json}");
            assert_eq!(json["token_class"], "api_key");
            assert_eq!(json["principal"]["user_id"], person.to_string());
            assert_eq!(json["api_key"]["id"], created.id);
            assert_eq!(json["api_key"]["name"], "ci-bot");
            assert_eq!(json["api_key"]["purpose"], "general");
            assert_eq!(json["scope"], "proxy");
            assert!(json["exp"].is_null());
            assert_eq!(json["memberships"].as_array().unwrap().len(), 2);
        }

        // Delegated token: verified, classified, not refused here.
        let delegated = jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &person,
            "proxy:* account:read",
            "aevatar",
            60,
            None,
        )
        .unwrap();
        let (status, _, json) = call(
            &app,
            introspect_request(
                Some(SECRET),
                serde_json::json!({"authorization": format!("Bearer {delegated}")}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["active"], true, "{json}");
        assert_eq!(json["token_class"], "delegated");

        // Expired token.
        let expired = jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &person,
            "account:read",
            None,
            Some(-3600),
            None,
            None,
            None,
        )
        .unwrap();
        let (status, _, json) = call(
            &app,
            introspect_request(
                Some(SECRET),
                serde_json::json!({"authorization": format!("Bearer {expired}")}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!({"active": false, "reason": "expired"})
        );

        // Garbage.
        let (status, _, json) = call(
            &app,
            introspect_request(
                Some(SECRET),
                serde_json::json!({"authorization": "Bearer nyxid_ag_not_a_key"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!({"active": false, "reason": "invalid"})
        );

        // Inactive user: an otherwise valid session is refused.
        db.collection::<User>(USERS)
            .update_one(
                doc! { "_id": person.to_string() },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();
        let (status, _, json) = call(
            &app,
            introspect_request(
                Some(SECRET),
                serde_json::json!({"authorization": format!("Bearer {session}")}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!({"active": false, "reason": "inactive_user"})
        );
    }

    #[tokio::test]
    async fn principal_resolves_person_org_and_missing() {
        let Some(db) = connect_test_database("h_internal_oracle_principal").await else {
            return;
        };
        let state = with_secret(test_app_state(db.clone()));
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&person, UserType::Person))
            .await
            .unwrap();
        let mut org_user = test_user(&org, UserType::Org);
        org_user.is_active = false;
        db.collection::<User>(USERS)
            .insert_one(org_user)
            .await
            .unwrap();
        let membership = test_membership(&org, &person, OrgRole::Member, None);
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(&membership)
            .await
            .unwrap();
        let app = router(state);

        let (status, cache, json) = call(&app, principal_request(Some(SECRET), &person)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cache, "no-store");
        assert_eq!(json["user_id"], person);
        assert_eq!(json["user_type"], "person");
        assert_eq!(json["is_active"], true);
        assert_eq!(json["memberships"][0]["org_user_id"], org);
        assert_eq!(json["memberships"][0]["membership_id"], membership.id);
        assert_eq!(json["memberships"][0]["role"], "member");

        let (status, _, json) = call(&app, principal_request(Some(SECRET), &org)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["user_type"], "org");
        assert_eq!(json["is_active"], false);
        assert_eq!(json["memberships"][0]["membership_id"], membership.id);

        let missing = Uuid::new_v4().to_string();
        let (status, _, json) = call(&app, principal_request(Some(SECRET), &missing)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!({
                "user_id": missing,
                "user_type": "missing",
                "is_active": false,
                "memberships": []
            })
        );
    }

    #[tokio::test]
    async fn audit_forwards_event_with_actor_fields() {
        use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOGS};

        let Some(db) = connect_test_database("h_internal_oracle_audit").await else {
            return;
        };
        let app = router(with_secret(test_app_state(db.clone())));
        let user_id = Uuid::new_v4().to_string();
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/internal/oracle/audit")
            .header(header::AUTHORIZATION, format!("Bearer {SECRET}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "event_type": "oracle_task_submitted",
                    "user_id": user_id,
                    "api_key_id": "key-1",
                    "api_key_name": "ci-bot",
                    "ip": "203.0.113.9",
                    "user_agent": "nyxid-cli/1.0",
                    "event_data": {"task_id": "t-1"}
                })
                .to_string(),
            ))
            .unwrap();
        let (status, cache, _) = call(&app, request).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(cache, "no-store");

        let mut row = None;
        for _ in 0..50 {
            row = db
                .collection::<AuditLog>(AUDIT_LOGS)
                .find_one(doc! { "user_id": &user_id })
                .await
                .unwrap();
            if row.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let row = row.expect("forwarded audit row written");
        assert_eq!(row.event_type, "oracle_task_submitted");
        assert_eq!(row.api_key_id.as_deref(), Some("key-1"));
        assert_eq!(row.api_key_name.as_deref(), Some("ci-bot"));
        assert_eq!(row.ip_address.as_deref(), Some("203.0.113.9"));
    }
}
