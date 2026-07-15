use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
// axum-extra's Form/Query decode repeated keys (`resource=a&resource=b`,
// `allowed_service_ids=...`) into Vec fields via serde_html_form; axum's
// built-in serde_urlencoded extractors reject them (#1115).
use axum_extra::extract::{Form, Query, QueryRejection};
use base64::Engine as _;
use chrono::Utc;
use jsonwebtoken::{Algorithm, Header, Validation, decode, encode};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent};
use crate::models::authorization_code::{ExternalSubjectRef, validate_external_subject_params};
// Keep both import surfaces: this handler resolves consent-scoped resources and
// service-account principals in the OAuth endpoints.
use crate::models::consent::Consent;
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::service_account_token::{COLLECTION_NAME as SA_TOKENS, ServiceAccountToken};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_service::UserService;
// USER_SERVICES collection const is only referenced from the #[cfg(test)] module
// now that production consent resolution goes through user_service_service.
#[cfg(test)]
use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
use crate::mw::auth::{AuthMethod, AuthUser, OptionalAuthUser};
use crate::services::{
    audit_service, consent_service, oauth_broker_service, oauth_client_service,
    oauth_resource_service, oauth_service, par_service, service_account_service,
    social_token_exchange_service, token_exchange_service, user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event, hash_short_id};

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    #[serde(default)]
    pub response_type: String,
    pub client_id: String,
    #[serde(default)]
    pub redirect_uri: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub nonce: Option<String>,
    pub external_subject_platform: Option<String>,
    pub external_subject_tenant: Option<String>,
    pub external_subject_external_user_id: Option<String>,
    /// SHA-256 identifier of an existing broker binding whose service grant
    /// the authenticated owner is reviewing. The raw binding credential is
    /// never sent through the browser.
    pub binding_grant_id: Option<String>,
    /// OIDC prompt parameter: "none", "login", "consent", or space-separated combo.
    pub prompt: Option<String>,
    pub request_uri: Option<String>,
    #[serde(default)]
    pub resource: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConsentDecisionForm {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub nonce: Option<String>,
    pub external_subject_platform: Option<String>,
    pub external_subject_tenant: Option<String>,
    pub external_subject_external_user_id: Option<String>,
    pub binding_grant_id: Option<String>,
    pub prompt: Option<String>,
    #[serde(default = "default_allow_all_services_form")]
    pub allow_all_services: bool,
    #[serde(default)]
    pub allowed_service_ids: Vec<String>,
    pub consent_request: Option<String>,
    #[serde(default)]
    pub resource: Vec<String>,
    pub decision: String,
}

fn default_allow_all_services_form() -> bool {
    false
}

#[derive(Debug, Serialize)]
pub struct AuthorizeResponse {
    pub redirect_url: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    /// RFC 8693 Token Exchange: the user's access token
    pub subject_token: Option<String>,
    /// RFC 8693 Token Exchange: must be "urn:ietf:params:oauth:token-type:access_token"
    pub subject_token_type: Option<String>,
    /// Requested scope (used by token exchange)
    pub scope: Option<String>,
    /// Social provider hint for external token exchange ("google" or "github")
    pub provider: Option<String>,
    #[serde(default)]
    pub resource: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_updated: Option<bool>,
    /// RFC 8693: Indicates the type of the issued token (only for token exchange grant).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_token_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UserinfoResponse {
    pub sub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
}

// --- Introspection / Revocation types ---

#[derive(Debug, Deserialize)]
pub struct IntrospectRequest {
    pub token: String,
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IntrospectResponse {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeRequest {
    pub token: String,
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetBindingQuery {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GetBindingResponse {
    pub binding_id: String,
    pub client_id: String,
    pub nyx_subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_subject_ref: Option<crate::models::authorization_code::ExternalSubjectRef>,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Deserialize)]
pub struct ListBindingsByExternalSubjectQuery {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub external_subject_platform: Option<String>,
    pub external_subject_tenant: Option<String>,
    pub external_subject_external_user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BindingSummary {
    pub binding_hash: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_subject_ref: Option<crate::models::authorization_code::ExternalSubjectRef>,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Serialize)]
pub struct ListBindingsResponse {
    pub bindings: Vec<BindingSummary>,
}

#[derive(Debug, Deserialize)]
pub struct PushedAuthorizationRequestForm {
    pub response_type: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub nonce: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub resource: Vec<String>,
    pub external_subject_platform: Option<String>,
    pub external_subject_tenant: Option<String>,
    pub external_subject_external_user_id: Option<String>,
    pub binding_grant_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PushedAuthorizationRequestResponse {
    pub request_uri: String,
    pub expires_in: i64,
}

// --- RFC 6749 §5.2 OAuth Error Response ---

/// RFC 6749 §5.2 compliant error body for the token endpoint.
/// Standard OAuth clients expect `error` + `error_description`, not our
/// internal `ErrorResponse` format which carries `error_code` / `message`.
#[derive(Debug, Serialize)]
struct OAuthErrorBody {
    error: &'static str,
    error_description: String,
}

const CONSENT_REQUEST_AUDIENCE: &str = "nyxid/oauth-consent";
const CONSENT_REQUEST_TOKEN_TYPE: &str = "oauth_consent_request";
const CONSENT_REQUEST_TTL_SECS: i64 = 15 * 60;

#[derive(Debug, Serialize, Deserialize)]
struct ConsentRequestClaims {
    sub: String,
    iss: String,
    aud: String,
    exp: i64,
    iat: i64,
    token_type: String,
    response_type: String,
    client_id: String,
    redirect_uri: String,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    nonce: Option<String>,
    external_subject_platform: Option<String>,
    external_subject_tenant: Option<String>,
    external_subject_external_user_id: Option<String>,
    binding_grant_id: Option<String>,
    prompt: Option<String>,
    resource: Vec<String>,
}

/// Map internal `AppError` to an RFC 6749 §5.2 JSON error response.
/// Uses `AppError::oauth_error_code()` and `AppError::oauth_status()` —
/// each variant declares its own OAuth semantics, no string matching.
fn oauth_error_response(err: AppError) -> Response {
    let status = err.oauth_status();
    let oauth_error = err.oauth_error_code();

    let description = match &err {
        AppError::Internal(_) | AppError::DatabaseError(_) => {
            "An internal error occurred".to_string()
        }
        other => other.to_string(),
    };

    (
        status,
        axum::Json(OAuthErrorBody {
            error: oauth_error,
            error_description: description,
        }),
    )
        .into_response()
}

fn parse_basic_client_credentials(headers: &HeaderMap) -> AppResult<Option<(String, String)>> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|_| AppError::Unauthorized("Invalid client credentials".to_string()))?;
    let Some(encoded) = raw
        .strip_prefix("Basic ")
        .or_else(|| raw.strip_prefix("basic "))
    else {
        return Ok(None);
    };

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| AppError::Unauthorized("Invalid client credentials".to_string()))?;
    let decoded = String::from_utf8(decoded)
        .map_err(|_| AppError::Unauthorized("Invalid client credentials".to_string()))?;
    let (client_id, client_secret) = decoded
        .split_once(':')
        .ok_or_else(|| AppError::Unauthorized("Invalid client credentials".to_string()))?;

    let client_id = urlencoding::decode(client_id)
        .map_err(|_| AppError::Unauthorized("Invalid client credentials".to_string()))?
        .into_owned();
    let client_secret = urlencoding::decode(client_secret)
        .map_err(|_| AppError::Unauthorized("Invalid client credentials".to_string()))?
        .into_owned();

    Ok(Some((client_id, client_secret)))
}

fn client_credentials_from_basic_or_params(
    basic: Option<(String, String)>,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Option<(String, Option<String>)> {
    match (basic, client_id, client_secret) {
        (Some((id, _)), Some(query_id), _) if query_id != id => None,
        (Some((id, secret)), _, _) => Some((id, Some(secret))),
        (None, Some(id), secret) => Some((id, secret)),
        _ => None,
    }
}

fn non_empty_resources(resources: &[String]) -> Option<&[String]> {
    (!resources.is_empty()).then_some(resources)
}

fn response_resources(resources: Vec<String>) -> Option<Vec<String>> {
    (!resources.is_empty()).then_some(resources)
}

fn dpop_token_error(err: AppError) -> AppError {
    match err {
        AppError::Unauthorized(message) => AppError::InvalidDpopProof(message),
        other => other,
    }
}

fn sender_constraint_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> AppResult<Option<crate::crypto::jwt::Cnf>> {
    let dpop_jkt = match headers.get("dpop") {
        Some(value) => {
            let proof = value
                .to_str()
                .map_err(|_| AppError::InvalidDpopProof("invalid DPoP proof".to_string()))?;
            let htu =
                crate::crypto::dpop::htu_from_base_and_path(&state.config.base_url, "/oauth/token")
                    .map_err(dpop_token_error)?;
            Some(
                crate::crypto::dpop::validate_proof(proof, "POST", &htu, &state.dpop_jti_cache)
                    .map_err(dpop_token_error)?,
            )
        }
        None => None,
    };

    let mtls_header_name = state
        .config
        .mtls_client_cert_header
        .as_deref()
        .filter(|header| !header.trim().is_empty());
    let mtls_x5t_s256 = match (dpop_jkt.as_ref(), mtls_header_name) {
        (Some(_), Some(header_name)) => {
            if headers.get(header_name).is_some() {
                tracing::debug!(
                    "DPoP and mTLS client certificate headers both present; using DPoP binding"
                );
            }
            None
        }
        (None, Some(header_name)) => match headers.get(header_name) {
            Some(value) => {
                let cert = value.to_str().map_err(|_| {
                    AppError::Unauthorized("invalid mTLS client certificate header".to_string())
                })?;
                if cert.trim().is_empty() {
                    None
                } else {
                    Some(crate::crypto::mtls::cert_thumbprint_from_header(cert)?)
                }
            }
            None => None,
        },
        (_, None) => None,
    };

    Ok(oauth_broker_service::sender_constraint_from_proofs(
        dpop_jkt.as_deref(),
        mtls_x5t_s256.as_deref(),
    ))
}

fn intersect_service_ids(left: &[String], right: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for id in left {
        if right.iter().any(|granted| granted == id) && !out.iter().any(|existing| existing == id) {
            out.push(id.clone());
        }
    }
    out
}

fn consent_requires_prompt(
    consent: Option<&Consent>,
    resolved_resources: Option<&oauth_resource_service::ResolvedOAuthResources>,
) -> bool {
    let Some(grant) = consent else {
        return true;
    };

    if grant.allowed_service_ids.is_none() && !grant.allow_all_services {
        return true;
    }

    if grant.allow_all_services {
        return false;
    }

    let Some(resolved) = resolved_resources else {
        return false;
    };
    let Some(consented_ids) = grant.allowed_service_ids.as_ref() else {
        return true;
    };

    !resolved.service_ids.iter().all(|service_id| {
        consented_ids
            .iter()
            .any(|consented_id| consented_id == service_id)
    })
}

async fn resolve_binding_grant_review(
    state: &AppState,
    params: &AuthorizeQuery,
    user_id: &str,
    external_subject: Option<&ExternalSubjectRef>,
) -> AppResult<Option<oauth_broker_service::BindingGrantSnapshot>> {
    let Some(binding_hash) = params.binding_grant_id.as_deref() else {
        return Ok(None);
    };
    let external_subject = external_subject.ok_or_else(|| {
        AppError::InvalidTarget("binding grant review requires an external subject".to_string())
    })?;

    oauth_broker_service::resolve_binding_grant(
        &state.db,
        &params.client_id,
        user_id,
        external_subject,
        binding_hash,
    )
    .await
    .map(Some)
}

fn params_from_consent_form(form: &ConsentDecisionForm) -> AuthorizeQuery {
    AuthorizeQuery {
        response_type: form.response_type.clone(),
        client_id: form.client_id.clone(),
        redirect_uri: form.redirect_uri.clone(),
        scope: form.scope.clone(),
        state: form.state.clone(),
        code_challenge: form.code_challenge.clone(),
        code_challenge_method: form.code_challenge_method.clone(),
        nonce: form.nonce.clone(),
        external_subject_platform: form.external_subject_platform.clone(),
        external_subject_tenant: form.external_subject_tenant.clone(),
        external_subject_external_user_id: form.external_subject_external_user_id.clone(),
        binding_grant_id: form.binding_grant_id.clone(),
        prompt: form.prompt.clone(),
        resource: form.resource.clone(),
        request_uri: None,
    }
}

fn selected_resources_are_subset(requested: &[String], selected: &[String]) -> bool {
    selected
        .iter()
        .all(|resource| requested.iter().any(|allowed| allowed == resource))
}

fn sign_consent_request(
    state: &AppState,
    user_id: &str,
    params: &AuthorizeQuery,
    validated_scope: &str,
) -> AppResult<String> {
    let now = Utc::now().timestamp();
    let claims = ConsentRequestClaims {
        sub: user_id.to_string(),
        iss: state.config.jwt_issuer.clone(),
        aud: CONSENT_REQUEST_AUDIENCE.to_string(),
        exp: now + CONSENT_REQUEST_TTL_SECS,
        iat: now,
        token_type: CONSENT_REQUEST_TOKEN_TYPE.to_string(),
        response_type: params.response_type.clone(),
        client_id: params.client_id.clone(),
        redirect_uri: params.redirect_uri.clone(),
        scope: Some(validated_scope.to_string()),
        state: params.state.clone(),
        code_challenge: params.code_challenge.clone(),
        code_challenge_method: params.code_challenge_method.clone(),
        nonce: params.nonce.clone(),
        external_subject_platform: params.external_subject_platform.clone(),
        external_subject_tenant: params.external_subject_tenant.clone(),
        external_subject_external_user_id: params.external_subject_external_user_id.clone(),
        binding_grant_id: params.binding_grant_id.clone(),
        prompt: params.prompt.clone(),
        resource: params.resource.clone(),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(state.jwt_keys.kid.clone());

    encode(&header, &claims, &state.jwt_keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode consent request: {e}")))
}

fn verify_consent_request(
    state: &AppState,
    token: &str,
    user_id: &str,
) -> AppResult<AuthorizeQuery> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[&state.config.jwt_issuer]);
    validation.set_audience(&[CONSENT_REQUEST_AUDIENCE]);

    let claims = decode::<ConsentRequestClaims>(token, &state.jwt_keys.decoding, &validation)
        .map_err(|_| AppError::BadRequest("Invalid consent request".to_string()))?
        .claims;

    if claims.token_type != CONSENT_REQUEST_TOKEN_TYPE || claims.sub != user_id {
        return Err(AppError::BadRequest("Invalid consent request".to_string()));
    }

    Ok(AuthorizeQuery {
        response_type: claims.response_type,
        client_id: claims.client_id,
        redirect_uri: claims.redirect_uri,
        scope: claims.scope,
        state: claims.state,
        code_challenge: claims.code_challenge,
        code_challenge_method: claims.code_challenge_method,
        nonce: claims.nonce,
        external_subject_platform: claims.external_subject_platform,
        external_subject_tenant: claims.external_subject_tenant,
        external_subject_external_user_id: claims.external_subject_external_user_id,
        binding_grant_id: claims.binding_grant_id,
        prompt: claims.prompt,
        resource: claims.resource,
        request_uri: None,
    })
}

// --- Handlers ---

/// GET /oauth/authorize
///
/// OAuth 2.0 Authorization Endpoint (dual-mode).
///
/// **Browser mode** (Accept: text/html, default): Used by MCP clients that open
/// a browser. Unauthenticated requests are 302-redirected to the frontend login
/// page with a `return_to` parameter. Authenticated requests receive a 302
/// redirect to the client's `redirect_uri` with the authorization code.
///
/// **API mode** (Accept: application/json): Used by the frontend SPA.
/// Requires a pre-authenticated session/token. Returns a JSON body with the
/// redirect URL. This preserves backward compatibility.
///
/// Requires PKCE (code_challenge) for all requests. Only S256 method is supported.
pub async fn authorize(
    State(state): State<AppState>,
    opt_auth: OptionalAuthUser,
    headers: HeaderMap,
    query_result: Result<Query<AuthorizeQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let is_browser_mode = !accepts_json(&headers);

    let params = match query_result {
        Ok(Query(p)) => p,
        Err(rejection) => {
            if is_browser_mode {
                let error_url = format!(
                    "{}/error?code=invalid_request&message={}",
                    state.config.frontend_url,
                    urlencoding::encode(&rejection.body_text()),
                );
                return Ok(redirect_302(&error_url));
            }
            return Err(AppError::BadRequest(rejection.body_text()));
        }
    };

    if params.request_uri.is_some() && opt_auth.0.is_none() {
        if is_browser_mode {
            let return_to = build_authorize_url(&state.config.frontend_url, &params);
            let login_url = format!(
                "{}/login?return_to={}",
                state.config.frontend_url,
                urlencoding::encode(&return_to),
            );
            return Ok(redirect_302(&login_url));
        }
        return Err(AppError::Unauthorized(
            "Authentication required".to_string(),
        ));
    }

    let params = match resolve_pushed_authorize_params(&state, params).await {
        Ok(params) => params,
        Err(err) if is_browser_mode => {
            let error_url = format!(
                "{}/error?code={}&message={}",
                state.config.frontend_url,
                urlencoding::encode(err.error_key()),
                urlencoding::encode(&err.to_string()),
            );
            return Ok(redirect_302(&error_url));
        }
        Err(err) => return Err(err),
    };

    let external_subject = validate_external_subject_params(
        params.external_subject_platform.as_deref(),
        params.external_subject_tenant.as_deref(),
        params.external_subject_external_user_id.as_deref(),
    );

    let is_authenticated = opt_auth.0.is_some();
    tracing::info!(
        client_id = %params.client_id,
        is_browser_mode,
        is_authenticated,
        redirect_uri = %params.redirect_uri,
        "OAuth authorize endpoint hit"
    );

    let result = match external_subject {
        Ok(external_subject) => {
            authorize_inner(
                &state,
                opt_auth,
                &params,
                is_browser_mode,
                external_subject.as_ref(),
            )
            .await
        }
        Err(err) => Err(err),
    };

    match result {
        Ok(response) => Ok(response),
        Err(ref err) if is_browser_mode => {
            tracing::warn!(
                client_id = %params.client_id,
                error = %err,
                "OAuth authorize failed, redirecting to error page"
            );
            let error_url = format!(
                "{}/error?code={}&message={}",
                state.config.frontend_url,
                urlencoding::encode(err.error_key()),
                urlencoding::encode(&err.to_string()),
            );
            Ok(redirect_302(&error_url))
        }
        Err(err) => Err(err),
    }
}

/// POST /oauth/authorize/decision
///
/// Browser consent decision endpoint. Accepts allow/deny from the consent page
/// and either issues an authorization code or redirects with access_denied.
pub async fn authorize_decision(
    State(state): State<AppState>,
    opt_auth: OptionalAuthUser,
    tele: TelemetryContext,
    Form(form): Form<ConsentDecisionForm>,
) -> Result<Response, AppError> {
    let auth_user = match opt_auth.0 {
        Some(user) => user,
        None => {
            let params = params_from_consent_form(&form);
            let return_to = build_authorize_url(&state.config.frontend_url, &params);
            let login_url = format!(
                "{}/login?return_to={}",
                state.config.frontend_url,
                urlencoding::encode(&return_to),
            );
            return Ok(redirect_302(&login_url));
        }
    };

    let user_id_str = auth_user.user_id.to_string();
    let params = verify_consent_request(
        &state,
        form.consent_request
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("Missing consent request".to_string()))?,
        &user_id_str,
    )?;
    let external_subject = validate_external_subject_params(
        params.external_subject_platform.as_deref(),
        params.external_subject_tenant.as_deref(),
        params.external_subject_external_user_id.as_deref(),
    )?;

    let (_client, validated_scope) = validate_authorize_request(&state, &params).await?;

    if form.decision == "deny" {
        let redirect_url = build_callback_error_url(
            &params,
            "access_denied",
            "The resource owner denied the request",
        );
        return Ok(redirect_302(&redirect_url));
    }

    if form.decision != "allow" {
        return Err(AppError::BadRequest("Invalid consent decision".to_string()));
    }

    // Re-resolve the reviewed binding after the consent round-trip. The
    // signed consent request carries only its hash; current ownership and
    // active grant state remain authoritative in the database.
    resolve_binding_grant_review(&state, &params, &user_id_str, external_subject.as_ref()).await?;

    if !selected_resources_are_subset(&params.resource, &form.resource) {
        return Err(AppError::InvalidTarget(
            "selected resource was not in the original authorization request".to_string(),
        ));
    }

    let consent_allowed_service_ids = if form.allow_all_services {
        None
    } else {
        let mut selected_service_ids = normalize_allowed_service_ids(form.allowed_service_ids);
        let required_service_ids = oauth_resource_service::resolve_resource_service_ids_for_user(
            &state.db,
            &state.config,
            &user_id_str,
            &params.resource,
        )
        .await?;
        for service_id in required_service_ids {
            if !selected_service_ids
                .iter()
                .any(|selected| selected == &service_id)
            {
                selected_service_ids.push(service_id);
            }
        }
        validate_allowed_service_ids(&state.db, &user_id_str, &selected_service_ids).await?;
        Some(selected_service_ids)
    };
    let consent = consent_service::grant_consent_with_services(
        &state.db,
        &user_id_str,
        &params.client_id,
        &validated_scope,
        consent_allowed_service_ids,
    )
    .await?;

    let code = issue_authorization_code(
        &state,
        &auth_user,
        &params,
        &validated_scope,
        &consent,
        external_subject.as_ref(),
    )
    .await?;
    let redirect_url = build_callback_url(&params, &code);

    // OAuth consent submits from multiple client types (web consent form,
    // native desktop / mobile via custom scheme, CLI via loopback). The
    // browser form POST does not carry `X-NyxID-Client`, so the
    // request-derived `tele.surface` defaults to `"backend"` which would
    // misattribute every grant. Derive surface from the redirect URI
    // instead: loopback -> CLI, non-http(s) custom scheme -> SDK/native,
    // everything else -> web UI.
    let consent_surface: &'static str = match url::Url::parse(&params.redirect_uri).ok().map(|u| {
        let scheme = u.scheme().to_string();
        let host = u.host_str().map(str::to_string);
        (scheme, host)
    }) {
        Some((scheme, _)) if scheme != "http" && scheme != "https" => "sdk",
        Some((scheme, host))
            if scheme == "http"
                && matches!(host.as_deref(), Some("127.0.0.1" | "localhost" | "[::1]")) =>
        {
            "cli"
        }
        _ => "ui",
    };
    let tele_consent = TelemetryContext {
        surface: consent_surface,
        client_version: None,
    };
    emit_event(
        state.telemetry.as_deref(),
        &user_id_str,
        auth_user.api_key_id.as_deref(),
        &tele_consent,
        TelemetryEvent::OauthAuthorizationGranted {
            // Raw UUID would be scrubbed to `[UUID_REDACTED]`, collapsing
            // every OAuth client onto a single bucket. Hash keeps
            // per-client analysis possible without leaking the UUID.
            client_id: hash_short_id(&params.client_id),
            grant_type: "authorization_code".to_string(),
        },
    );
    let _ = &tele;

    if needs_success_page(&params.redirect_uri) {
        Ok(oauth_success_page(&redirect_url))
    } else {
        Ok(redirect_302(&redirect_url))
    }
}

/// Parse the `prompt` parameter into a set of prompt values (OIDC Core §3.1.2.1).
fn parse_prompt(prompt: Option<&str>) -> std::collections::HashSet<&str> {
    prompt
        .map(|p| p.split_whitespace().collect())
        .unwrap_or_default()
}

async fn authorize_inner(
    state: &AppState,
    opt_auth: OptionalAuthUser,
    params: &AuthorizeQuery,
    is_browser_mode: bool,
    external_subject: Option<&ExternalSubjectRef>,
) -> Result<Response, AppError> {
    let (client, validated_scope) = validate_authorize_request(state, params).await?;
    let prompts = parse_prompt(params.prompt.as_deref());

    // OIDC Core §3.1.2.1: prompt=none is incompatible with login/consent.
    if prompts.contains("none") && (prompts.contains("login") || prompts.contains("consent")) {
        return Err(AppError::BadRequest(
            "prompt=none cannot be combined with login or consent".to_string(),
        ));
    }

    let force_login = prompts.contains("login");
    let force_consent = prompts.contains("consent");

    if is_browser_mode {
        // prompt=login: treat as unauthenticated to force re-login
        let effective_auth = if force_login { None } else { opt_auth.0 };

        match effective_auth {
            None => {
                // prompt=none + unauthenticated → error, not redirect
                if prompts.contains("none") {
                    let redirect_url = build_callback_error_url(
                        params,
                        "login_required",
                        "User is not authenticated",
                    );
                    return Ok(redirect_302(&redirect_url));
                }

                let return_to = build_authorize_url(&state.config.frontend_url, params);
                let login_url = format!(
                    "{}/login?return_to={}",
                    state.config.frontend_url,
                    urlencoding::encode(&return_to),
                );
                tracing::info!(
                    client_id = %params.client_id,
                    "Unauthenticated OAuth request, redirecting to login"
                );
                Ok(redirect_302(&login_url))
            }
            Some(auth_user) => {
                let user_id_str = auth_user.user_id.to_string();

                let consent = consent_service::check_consent(
                    &state.db,
                    &user_id_str,
                    &params.client_id,
                    &validated_scope,
                )
                .await?;

                let resolved_resources = oauth_resource_service::resolve_requested_resources(
                    &state.db,
                    &state.config,
                    &user_id_str,
                    non_empty_resources(&params.resource),
                )
                .await?;
                let binding_grant =
                    resolve_binding_grant_review(state, params, &user_id_str, external_subject)
                        .await?;

                let needs_consent = binding_grant.is_some()
                    || consent_requires_prompt(consent.as_ref(), resolved_resources.as_ref())
                    || force_consent;

                if needs_consent {
                    // prompt=none + needs consent → error, not redirect
                    if prompts.contains("none") {
                        let redirect_url = build_callback_error_url(
                            params,
                            "consent_required",
                            "User consent is required",
                        );
                        return Ok(redirect_302(&redirect_url));
                    }

                    let mut default_service_hints =
                        resolve_app_default_service_hints(state, &client, &user_id_str).await?;
                    default_service_hints
                        .include_flow_grants(resolved_resources.as_ref(), binding_grant.as_ref());
                    let consent_url = build_consent_url(
                        &state.config.frontend_url,
                        params,
                        &client.client_name,
                        &validated_scope,
                        Some(&sign_consent_request(
                            state,
                            &user_id_str,
                            params,
                            &validated_scope,
                        )?),
                        &default_service_hints,
                    );
                    return Ok(redirect_302(&consent_url));
                }

                let consent = consent.ok_or_else(|| {
                    AppError::Internal("Consent check unexpectedly returned no grant".to_string())
                })?;

                let code = issue_authorization_code(
                    state,
                    &auth_user,
                    params,
                    &validated_scope,
                    &consent,
                    external_subject,
                )
                .await?;
                let redirect_url = build_callback_url(params, &code);

                if needs_success_page(&params.redirect_uri) {
                    Ok(oauth_success_page(&redirect_url))
                } else {
                    Ok(redirect_302(&redirect_url))
                }
            }
        }
    } else {
        let auth_user = opt_auth
            .0
            .ok_or_else(|| AppError::Unauthorized("Authentication required".to_string()))?;

        let user_id_str = auth_user.user_id.to_string();

        let consent = consent_service::check_consent(
            &state.db,
            &user_id_str,
            &params.client_id,
            &validated_scope,
        )
        .await?;

        let resolved_resources = oauth_resource_service::resolve_requested_resources(
            &state.db,
            &state.config,
            &user_id_str,
            non_empty_resources(&params.resource),
        )
        .await?;
        let binding_grant =
            resolve_binding_grant_review(state, params, &user_id_str, external_subject).await?;

        if binding_grant.is_some()
            || consent_requires_prompt(consent.as_ref(), resolved_resources.as_ref())
            || force_consent
        {
            let mut default_service_hints =
                resolve_app_default_service_hints(state, &client, &user_id_str).await?;
            default_service_hints
                .include_flow_grants(resolved_resources.as_ref(), binding_grant.as_ref());
            let consent_url = build_consent_url(
                &state.config.frontend_url,
                params,
                &client.client_name,
                &validated_scope,
                Some(&sign_consent_request(
                    state,
                    &user_id_str,
                    params,
                    &validated_scope,
                )?),
                &default_service_hints,
            );
            return Err(AppError::ConsentRequired { consent_url });
        }

        let consent = consent.ok_or_else(|| {
            AppError::Internal("Consent check unexpectedly returned no grant".to_string())
        })?;

        let code = issue_authorization_code(
            state,
            &auth_user,
            params,
            &validated_scope,
            &consent,
            external_subject,
        )
        .await?;
        let redirect_url = build_callback_url(params, &code);
        Ok(Json(AuthorizeResponse { redirect_url }).into_response())
    }
}

async fn validate_authorize_request(
    state: &AppState,
    params: &AuthorizeQuery,
) -> AppResult<(crate::models::oauth_client::OauthClient, String)> {
    if params.response_type != "code" {
        return Err(AppError::BadRequest(
            "Only response_type=code is supported".to_string(),
        ));
    }

    if params.code_challenge.is_none() {
        return Err(AppError::BadRequest(
            "code_challenge is required (PKCE)".to_string(),
        ));
    }

    match params.code_challenge_method.as_deref() {
        Some("S256") => {}
        Some(_) => {
            return Err(AppError::BadRequest(
                "Only S256 code_challenge_method is supported".to_string(),
            ));
        }
        None => {
            return Err(AppError::BadRequest(
                "code_challenge_method is required (must be S256)".to_string(),
            ));
        }
    }

    let client =
        oauth_service::validate_client(&state.db, &params.client_id, &params.redirect_uri).await?;
    let validated_scope =
        oauth_service::resolve_authorize_scope(params.scope.as_deref(), &client.allowed_scopes)?;

    Ok((client, validated_scope))
}

/// Build a 302 Found response (RFC 6749 requires 302, not 307).
/// Includes Referrer-Policy: no-referrer to prevent leaking the authorization
/// code or other query parameters via the Referer header.
fn redirect_302(uri: &str) -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, uri)
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(axum::body::Body::empty())
        .unwrap()
}

/// Check whether a redirect URI targets a loopback address (MCP/CLI clients).
/// Returns true for redirect URIs where the browser should show a friendly
/// success page instead of a bare 302. This covers:
/// - Loopback redirects (http://127.0.0.1/...) where the local callback server
///   typically renders a blank page
/// - Custom URI schemes (cursor://, vscode://) where the browser can't render
///   anything after the OS handles the protocol
fn needs_success_page(uri: &str) -> bool {
    let Ok(parsed) = url::Url::parse(uri) else {
        return false;
    };
    // Custom URI scheme (cursor://, vscode://, claude-code://, etc.)
    if !matches!(parsed.scheme(), "http" | "https") {
        return true;
    }
    // Loopback redirect
    parsed.scheme() == "http"
        && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

/// Render a branded HTML page that confirms authentication succeeded and
/// auto-redirects to the callback URI.  The MCP client's local callback
/// server receives the code via the redirect while the user sees a clear
/// success message instead of a blank white page.
///
/// Overrides the global CSP to allow inline style/script for this one-off
/// HTML page (the global CSP is `default-src 'none'` which blocks them).
fn oauth_success_page(redirect_url: &str) -> Response {
    let escaped = redirect_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let js_escaped = redirect_url.replace('\\', "\\\\").replace('\'', "\\'");

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="2;url={escaped}">
<meta name="referrer" content="no-referrer">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>NyxID — Authenticated</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;display:flex;align-items:center;justify-content:center;flex-direction:column;min-height:100vh;background:#0a0a0b;color:#e4e4e7}}
.wrap{{display:flex;flex-direction:column;align-items:center;gap:2rem;width:100%;max-width:26rem;padding:1.5rem}}
.logo{{display:flex;align-items:center;gap:.6rem}}
.logo svg{{width:28px;height:28px}}
.logo span{{font-size:1.2rem;font-weight:700;letter-spacing:-.02em;background:linear-gradient(135deg,#c084fc,#818cf8);-webkit-background-clip:text;-webkit-text-fill-color:transparent}}
.card{{width:100%;text-align:center;padding:2.5rem 2rem;border:1px solid #27272a;border-radius:.75rem;background:#18181b}}
.icon{{width:3rem;height:3rem;margin:0 auto 1.25rem;border-radius:50%;background:rgba(52,211,153,.12);display:flex;align-items:center;justify-content:center}}
.icon svg{{width:1.25rem;height:1.25rem;color:#34d399}}
h1{{font-size:1.125rem;font-weight:600;margin-bottom:.375rem}}
.sub{{font-size:.8125rem;color:#a1a1aa;line-height:1.5}}
.bar{{margin-top:1.5rem;height:3px;border-radius:2px;background:#27272a;overflow:hidden}}
.bar .fill{{height:100%;width:0;border-radius:2px;background:linear-gradient(90deg,#818cf8,#c084fc);animation:progress 1.8s ease-in-out forwards}}
@keyframes progress{{to{{width:100%}}}}
.foot{{font-size:.6875rem;color:#52525b}}
</style>
</head>
<body>
<div class="wrap">
  <div class="logo">
    <svg viewBox="0 0 32 32" fill="none"><circle cx="16" cy="16" r="14" stroke="url(#pg)" stroke-width="2.5"/><circle cx="16" cy="16" r="4" fill="url(#pg)"/><defs><linearGradient id="pg" x1="4" y1="4" x2="28" y2="28"><stop stop-color="#c084fc"/><stop offset="1" stop-color="#818cf8"/></linearGradient></defs></svg>
    <span>NyxID</span>
  </div>
  <div class="card">
    <div class="icon">
      <svg fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
    </div>
    <h1>Authentication Successful</h1>
    <p class="sub">Redirecting you back to the application&hellip;</p>
    <div class="bar"><div class="fill"></div></div>
  </div>
  <p class="foot">You can close this tab if it doesn&rsquo;t redirect automatically.</p>
</div>
<script>setTimeout(function(){{window.location.replace('{js_escaped}')}},1800)</script>
</body>
</html>"##
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/html; charset=utf-8")
        .header(header::REFERRER_POLICY, "no-referrer")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; frame-ancestors 'none'",
        )
        .body(axum::body::Body::from(html))
        .unwrap()
}

/// Returns true when the request explicitly asks for JSON (API / XHR clients).
fn accepts_json(headers: &HeaderMap) -> bool {
    headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/json"))
        .unwrap_or(false)
}

fn has_non_par_authorize_params(params: &AuthorizeQuery) -> bool {
    !params.response_type.is_empty()
        || !params.redirect_uri.is_empty()
        || params.scope.is_some()
        || params.state.is_some()
        || params.code_challenge.is_some()
        || params.code_challenge_method.is_some()
        || params.nonce.is_some()
        || params.external_subject_platform.is_some()
        || params.external_subject_tenant.is_some()
        || params.external_subject_external_user_id.is_some()
        || params.binding_grant_id.is_some()
        || params.prompt.is_some()
        || !params.resource.is_empty()
}

async fn resolve_pushed_authorize_params(
    state: &AppState,
    params: AuthorizeQuery,
) -> AppResult<AuthorizeQuery> {
    let Some(request_uri) = params.request_uri.as_deref() else {
        return Ok(params);
    };

    if has_non_par_authorize_params(&params) {
        tracing::warn!(
            client_id = %params.client_id,
            "Ignoring authorize query parameters supplied alongside request_uri"
        );
    }

    let record = par_service::consume_request(&state.db, request_uri, &params.client_id).await?;
    let external_subject_platform = record
        .external_subject
        .as_ref()
        .map(|subject| subject.platform.clone());
    let external_subject_tenant = record
        .external_subject
        .as_ref()
        .and_then(|subject| subject.tenant.clone());
    let external_subject_external_user_id = record
        .external_subject
        .as_ref()
        .map(|subject| subject.external_user_id.clone());

    Ok(AuthorizeQuery {
        response_type: record.response_type,
        client_id: record.client_id,
        redirect_uri: record.redirect_uri,
        scope: record.scope,
        state: record.state,
        code_challenge: record.code_challenge,
        code_challenge_method: record.code_challenge_method,
        nonce: record.nonce,
        external_subject_platform,
        external_subject_tenant,
        external_subject_external_user_id,
        binding_grant_id: record.binding_grant_id,
        prompt: record.prompt,
        resource: record.resources,
        request_uri: None,
    })
}

/// Reconstruct the full authorize URL so it can be used as a `return_to` target
/// after the user logs in on the frontend.
fn build_authorize_url(base_url: &str, params: &AuthorizeQuery) -> String {
    if let Some(request_uri) = params.request_uri.as_deref() {
        return format!(
            "{}/oauth/authorize?client_id={}&request_uri={}",
            base_url,
            urlencoding::encode(&params.client_id),
            urlencoding::encode(request_uri),
        );
    }

    let mut url = format!(
        "{}/oauth/authorize?response_type={}&client_id={}&redirect_uri={}",
        base_url,
        urlencoding::encode(&params.response_type),
        urlencoding::encode(&params.client_id),
        urlencoding::encode(&params.redirect_uri),
    );

    if let Some(ref scope) = params.scope {
        url.push_str(&format!("&scope={}", urlencoding::encode(scope)));
    }
    if let Some(ref state) = params.state {
        url.push_str(&format!("&state={}", urlencoding::encode(state)));
    }
    if let Some(ref cc) = params.code_challenge {
        url.push_str(&format!("&code_challenge={}", urlencoding::encode(cc)));
    }
    if let Some(ref ccm) = params.code_challenge_method {
        url.push_str(&format!(
            "&code_challenge_method={}",
            urlencoding::encode(ccm)
        ));
    }
    if let Some(ref nonce) = params.nonce {
        url.push_str(&format!("&nonce={}", urlencoding::encode(nonce)));
    }
    if let Some(ref platform) = params.external_subject_platform
        && !platform.is_empty()
    {
        url.push_str(&format!(
            "&external_subject_platform={}",
            urlencoding::encode(platform)
        ));
    }
    if let Some(ref tenant) = params.external_subject_tenant
        && !tenant.is_empty()
    {
        url.push_str(&format!(
            "&external_subject_tenant={}",
            urlencoding::encode(tenant)
        ));
    }
    if let Some(ref external_user_id) = params.external_subject_external_user_id
        && !external_user_id.is_empty()
    {
        url.push_str(&format!(
            "&external_subject_external_user_id={}",
            urlencoding::encode(external_user_id)
        ));
    }
    if let Some(ref binding_grant_id) = params.binding_grant_id {
        url.push_str(&format!(
            "&binding_grant_id={}",
            urlencoding::encode(binding_grant_id)
        ));
    }
    if let Some(ref prompt) = params.prompt {
        url.push_str(&format!("&prompt={}", urlencoding::encode(prompt)));
    }
    for resource in &params.resource {
        url.push_str(&format!("&resource={}", urlencoding::encode(resource)));
    }

    url
}

/// Build the callback redirect URL with code and optional state.
fn build_callback_url(params: &AuthorizeQuery, code: &str) -> String {
    let mut url = format!("{}?code={}", params.redirect_uri, urlencoding::encode(code),);
    if let Some(ref state_param) = params.state {
        url.push_str(&format!("&state={}", urlencoding::encode(state_param)));
    }
    url
}

fn build_callback_error_url(params: &AuthorizeQuery, error: &str, description: &str) -> String {
    let mut url = format!(
        "{}?error={}&error_description={}",
        params.redirect_uri,
        urlencoding::encode(error),
        urlencoding::encode(description),
    );
    if let Some(ref state_param) = params.state {
        url.push_str(&format!("&state={}", urlencoding::encode(state_param)));
    }
    url
}

fn normalize_allowed_service_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();

    for id in ids {
        let trimmed = id.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            normalized.push(trimmed.to_string());
        }
    }

    normalized
}

async fn validate_allowed_service_ids(
    db: &mongodb::Database,
    user_id: &str,
    allowed_service_ids: &[String],
) -> AppResult<()> {
    if !oauth_resource_service::validate_grantable_service_ids(db, user_id, allowed_service_ids)
        .await?
    {
        return Err(AppError::BadRequest(
            "Selected service access contains an unknown service".to_string(),
        ));
    }

    Ok(())
}

/// Per-user resolution of an app's declared default catalog services,
/// computed when the consent redirect is built. Pure UI hint: the consent
/// page seeds its summary/selection from these, but the decision POST is
/// still ownership-validated server-side.
#[derive(Debug, Default)]
struct AppDefaultServiceHints {
    /// Proxyable personal or org-inherited UserService ids matching the
    /// app's declared catalog slugs -- pre-selected on the consent screen.
    preselect_service_ids: Vec<String>,
    /// Human-readable names of declared catalog services the user has no
    /// matching service for -- shown as unmatched on the consent screen.
    unmatched_names: Vec<String>,
    /// Services resolved from RFC 8707 resource parameters. These are required
    /// by the app and cannot be removed independently of denying the request.
    required_service_ids: Vec<String>,
    /// Service grant currently held by the reviewed broker binding.
    current_binding_service_ids: Vec<String>,
    current_binding_allow_all_services: bool,
    binding_review: bool,
}

impl AppDefaultServiceHints {
    fn include_flow_grants(
        &mut self,
        resolved_resources: Option<&oauth_resource_service::ResolvedOAuthResources>,
        binding_grant: Option<&oauth_broker_service::BindingGrantSnapshot>,
    ) {
        self.required_service_ids = resolved_resources
            .map(|resolved| resolved.service_ids.clone())
            .unwrap_or_default();
        if let Some(binding_grant) = binding_grant {
            self.binding_review = true;
            self.current_binding_service_ids = binding_grant.allowed_service_ids.clone();
            self.current_binding_allow_all_services = binding_grant.allow_all_services;
        }
    }
}

/// Resolve `client.default_service_catalog_slugs` against the consenting
/// user's grantable services: catalog slug -> DownstreamService id ->
/// visible/proxyable UserServices with that `catalog_service_id`.
async fn resolve_app_default_service_hints(
    state: &AppState,
    client: &crate::models::oauth_client::OauthClient,
    user_id: &str,
) -> AppResult<AppDefaultServiceHints> {
    use futures::TryStreamExt as _;

    if client.default_service_catalog_slugs.is_empty() {
        return Ok(AppDefaultServiceHints::default());
    }

    let catalog_docs: Vec<crate::models::downstream_service::DownstreamService> = state
        .db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {
            "slug": { "$in": &client.default_service_catalog_slugs },
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;

    let catalog_ids: Vec<String> = catalog_docs.iter().map(|d| d.id.clone()).collect();
    let user_services: Vec<UserService> = if catalog_ids.is_empty() {
        Vec::new()
    } else {
        user_service_service::list_user_services_with_sources(&state.db, user_id)
            .await?
            .into_iter()
            .filter_map(|entry| {
                let allowed = match &entry.source {
                    user_service_service::CredentialSource::Personal => true,
                    user_service_service::CredentialSource::Org { allowed, .. } => *allowed,
                };
                let catalog_id = entry.service.catalog_service_id.as_deref()?;
                if allowed && catalog_ids.iter().any(|id| id == catalog_id) {
                    Some(entry.service)
                } else {
                    None
                }
            })
            .collect()
    };

    let matched_catalog_ids: std::collections::HashSet<&str> = user_services
        .iter()
        .filter_map(|s| s.catalog_service_id.as_deref())
        .collect();
    let unmatched_names = catalog_docs
        .iter()
        .filter(|d| !matched_catalog_ids.contains(d.id.as_str()))
        .map(|d| d.name.clone())
        .collect();

    Ok(AppDefaultServiceHints {
        preselect_service_ids: user_services.into_iter().map(|s| s.id).collect(),
        unmatched_names,
        ..AppDefaultServiceHints::default()
    })
}

fn build_consent_url(
    frontend_url: &str,
    params: &AuthorizeQuery,
    client_name: &str,
    validated_scope: &str,
    consent_request: Option<&str>,
    default_service_hints: &AppDefaultServiceHints,
) -> String {
    let mut url = format!(
        "{}/oauth-consent?response_type={}&client_id={}&client_name={}&redirect_uri={}",
        frontend_url,
        urlencoding::encode(&params.response_type),
        urlencoding::encode(&params.client_id),
        urlencoding::encode(client_name),
        urlencoding::encode(&params.redirect_uri),
    );

    url.push_str(&format!("&scope={}", urlencoding::encode(validated_scope)));

    if let Some(ref state) = params.state {
        url.push_str(&format!("&state={}", urlencoding::encode(state)));
    }
    if let Some(ref cc) = params.code_challenge {
        url.push_str(&format!("&code_challenge={}", urlencoding::encode(cc)));
    }
    if let Some(ref ccm) = params.code_challenge_method {
        url.push_str(&format!(
            "&code_challenge_method={}",
            urlencoding::encode(ccm),
        ));
    }
    if let Some(ref nonce) = params.nonce {
        url.push_str(&format!("&nonce={}", urlencoding::encode(nonce)));
    }
    if let Some(ref platform) = params.external_subject_platform
        && !platform.is_empty()
    {
        url.push_str(&format!(
            "&external_subject_platform={}",
            urlencoding::encode(platform)
        ));
    }
    if let Some(ref tenant) = params.external_subject_tenant
        && !tenant.is_empty()
    {
        url.push_str(&format!(
            "&external_subject_tenant={}",
            urlencoding::encode(tenant)
        ));
    }
    if let Some(ref external_user_id) = params.external_subject_external_user_id
        && !external_user_id.is_empty()
    {
        url.push_str(&format!(
            "&external_subject_external_user_id={}",
            urlencoding::encode(external_user_id)
        ));
    }
    if let Some(ref binding_grant_id) = params.binding_grant_id {
        url.push_str(&format!(
            "&binding_grant_id={}",
            urlencoding::encode(binding_grant_id)
        ));
    }
    if let Some(ref prompt) = params.prompt {
        url.push_str(&format!("&prompt={}", urlencoding::encode(prompt)));
    }
    for resource in &params.resource {
        url.push_str(&format!("&resource={}", urlencoding::encode(resource)));
    }
    for service_id in &default_service_hints.preselect_service_ids {
        url.push_str(&format!(
            "&preselect_service_ids={}",
            urlencoding::encode(service_id)
        ));
    }
    for name in &default_service_hints.unmatched_names {
        url.push_str(&format!(
            "&unmatched_defaults={}",
            urlencoding::encode(name)
        ));
    }
    for service_id in &default_service_hints.required_service_ids {
        url.push_str(&format!(
            "&required_service_ids={}",
            urlencoding::encode(service_id)
        ));
    }
    for service_id in &default_service_hints.current_binding_service_ids {
        url.push_str(&format!(
            "&current_binding_service_ids={}",
            urlencoding::encode(service_id)
        ));
    }
    if default_service_hints.current_binding_allow_all_services {
        url.push_str("&current_binding_allow_all_services=true");
    }
    if default_service_hints.binding_review {
        url.push_str("&binding_review=true");
    }
    if let Some(consent_request) = consent_request {
        url.push_str(&format!(
            "&consent_request={}",
            urlencoding::encode(consent_request)
        ));
    }

    url
}

/// Create an authorization code for the given user and OAuth parameters.
async fn issue_authorization_code(
    state: &AppState,
    auth_user: &crate::mw::auth::AuthUser,
    params: &AuthorizeQuery,
    validated_scope: &str,
    consent: &Consent,
    external_subject: Option<&ExternalSubjectRef>,
) -> AppResult<String> {
    let user_id_str = auth_user.user_id.to_string();
    let resolved_resources = oauth_resource_service::resolve_requested_resources(
        &state.db,
        &state.config,
        &user_id_str,
        non_empty_resources(&params.resource),
    )
    .await?;
    let (resource_uris, allowed_service_ids, service_restricted) =
        match (resolved_resources, consent.allowed_service_ids.as_ref()) {
            (Some(resolved), _) if consent.allow_all_services => {
                (resolved.resource_uris, resolved.service_ids, true)
            }
            (Some(resolved), Some(consented_ids)) => {
                let allowed = intersect_service_ids(&resolved.service_ids, consented_ids);
                let resources = resolved
                    .resource_uris
                    .into_iter()
                    .zip(resolved.service_ids)
                    .filter_map(|(resource, service_id)| {
                        allowed
                            .iter()
                            .any(|id| id == &service_id)
                            .then_some(resource)
                    })
                    .collect::<Vec<_>>();
                (resources, allowed, true)
            }
            (Some(_), None) => (Vec::new(), Vec::new(), true),
            (None, _) if consent.allow_all_services => (Vec::new(), Vec::new(), false),
            (None, Some(consented_ids)) => (Vec::new(), consented_ids.clone(), true),
            (None, None) => (Vec::new(), Vec::new(), true),
        };
    let code = oauth_service::create_authorization_code(
        &state.db,
        &params.client_id,
        &user_id_str,
        &params.redirect_uri,
        validated_scope,
        params.code_challenge.as_deref(),
        params.code_challenge_method.as_deref(),
        params.nonce.as_deref(),
        external_subject,
        params.binding_grant_id.as_deref(),
        &resource_uris,
        &allowed_service_ids,
        // AuthorizationCode stores the allow-all flag, while this path
        // tracks whether the grant is service-restricted.
        !service_restricted,
    )
    .await?;

    let mut event_data = serde_json::json!({
        "client_id": params.client_id,
        "scope": validated_scope,
        "allow_all_services": !service_restricted,
        "allowed_service_ids_count": allowed_service_ids.len(),
    });
    if !resource_uris.is_empty()
        && let Some(obj) = event_data.as_object_mut()
    {
        obj.insert("resources".to_string(), serde_json::json!(resource_uris));
        obj.insert(
            "allowed_service_ids".to_string(),
            serde_json::json!(allowed_service_ids),
        );
    }
    if let Some(external_subject) = external_subject
        && let Some(obj) = event_data.as_object_mut()
    {
        obj.insert(
            "external_subject_platform".to_string(),
            serde_json::Value::String(external_subject.platform.clone()),
        );
    }

    // Audit log the authorization code issuance
    let _ = user_id_str;
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "oauth_code_issued",
        Some(event_data),
    );

    Ok(code)
}

/// POST /oauth/par
pub async fn pushed_authorization_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(body): Form<PushedAuthorizationRequestForm>,
) -> AppResult<(StatusCode, Json<PushedAuthorizationRequestResponse>)> {
    let basic = parse_basic_client_credentials(&headers)?;
    let (client_id, client_secret) =
        match (basic, body.client_id.clone(), body.client_secret.clone()) {
            (Some((id, _)), Some(form_id), _) if form_id != id => {
                return Err(AppError::BadRequest(
                    "client_id does not match authenticated client".to_string(),
                ));
            }
            (Some((id, secret)), _, _) => (id, secret),
            (None, Some(id), Some(secret)) => (id, secret),
            _ => {
                return Err(AppError::Unauthorized(
                    "Missing client credentials".to_string(),
                ));
            }
        };

    oauth_service::authenticate_client(&state.db, &client_id, Some(&client_secret)).await?;
    oauth_service::validate_client(&state.db, &client_id, &body.redirect_uri).await?;

    if body.response_type != "code" {
        return Err(AppError::BadRequest(
            "Unsupported response_type".to_string(),
        ));
    }

    if let Some(method) = body.code_challenge_method.as_deref()
        && method != "S256"
    {
        return Err(AppError::BadRequest(
            "Only S256 code_challenge_method is supported".to_string(),
        ));
    }

    let external_subject = validate_external_subject_params(
        body.external_subject_platform.as_deref(),
        body.external_subject_tenant.as_deref(),
        body.external_subject_external_user_id.as_deref(),
    )?;
    if let Some(binding_grant_id) = body.binding_grant_id.as_deref()
        && (external_subject.is_none() || !oauth_broker_service::is_binding_hash(binding_grant_id))
    {
        return Err(AppError::InvalidTarget(
            "binding grant review requires a valid external subject and binding hash".to_string(),
        ));
    }
    for resource in &body.resource {
        oauth_resource_service::validate_resource_uri(resource)?;
    }

    let (request_uri, expires_in) = par_service::create_request(
        &state.db,
        &client_id,
        &body.response_type,
        &body.redirect_uri,
        body.scope.as_deref(),
        body.state.as_deref(),
        body.code_challenge.as_deref(),
        body.code_challenge_method.as_deref(),
        body.nonce.as_deref(),
        body.prompt.as_deref(),
        &body.resource,
        external_subject,
        body.binding_grant_id.as_deref(),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(PushedAuthorizationRequestResponse {
            request_uri,
            expires_in,
        }),
    ))
}

/// POST /oauth/token
///
/// OAuth 2.0 Token Endpoint (RFC 6749 §5).
///
/// Error responses use RFC 6749 §5.2 format (`error` + `error_description`)
/// instead of the application's internal `ErrorResponse` format, because
/// standard OAuth/OIDC client libraries depend on the standard error shape.
pub async fn token(
    State(state): State<AppState>,
    tele: TelemetryContext,
    headers: HeaderMap,
    Form(body): Form<TokenRequest>,
) -> Response {
    match token_inner(&state, &tele, &headers, body).await {
        Ok(json) => json.into_response(),
        Err(err) => oauth_error_response(err),
    }
}

// TODO(telemetry): all grant branches blocked — see TELEMETRY.md §6.5
// (oauth.token_issued). The four `/oauth/token` grant-type branches
// (`authorization_code`, `refresh_token`, `client_credentials`, social
// `urn:ietf:params:oauth:grant-type:token-exchange`) do not emit
// `OauthTokenIssued` in Part 2. Lift this once §6.5 is resolved.
async fn token_inner(
    state: &AppState,
    tele: &TelemetryContext,
    headers: &HeaderMap,
    body: TokenRequest,
) -> AppResult<Json<TokenResponse>> {
    match body.grant_type.as_str() {
        "authorization_code" => {
            let code = body
                .code
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing code parameter".to_string()))?;

            let redirect_uri = body.redirect_uri.as_deref().ok_or_else(|| {
                AppError::BadRequest("Missing redirect_uri parameter".to_string())
            })?;

            let client_id_str = body
                .client_id
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing client_id parameter".to_string()))?;

            let sender_constraint = sender_constraint_from_headers(state, headers)?;
            let dpop_jkt = sender_constraint
                .as_ref()
                .and_then(|cnf| cnf.jkt.as_deref());
            let mtls_x5t_s256 = sender_constraint
                .as_ref()
                .and_then(|cnf| cnf.x5t_s256.as_deref());

            let exchanged = oauth_service::exchange_authorization_code(
                &state.db,
                &state.config,
                &state.jwt_keys,
                state.broker_require_admin_capability(),
                code,
                client_id_str,
                redirect_uri,
                body.code_verifier.as_deref(),
                body.client_secret.as_deref(),
                Some(oauth_broker_service::BROKER_ACCESS_TTL_SECS),
                non_empty_resources(&body.resource),
                dpop_jkt,
                mtls_x5t_s256,
            )
            .await?;

            if exchanged.broker_capability_enabled {
                let granted_scopes: Vec<String> = exchanged
                    .granted_scope
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                let broker_require_sender_constraint = state.broker_require_sender_constraint();
                if broker_require_sender_constraint && sender_constraint.is_none() {
                    state
                        .db
                        .collection::<crate::models::refresh_token::RefreshToken>(
                            crate::models::refresh_token::COLLECTION_NAME,
                        )
                        .delete_one(doc! {
                            "jti": &exchanged.refresh_token_jti,
                            "client_id": client_id_str,
                            "user_id": &exchanged.user_id,
                        })
                        .await?;
                    audit_service::log_async(
                        state.db.clone(),
                        Some(exchanged.user_id.clone()),
                        "oauth_broker_binding_unpinned_create_rejected".to_string(),
                        Some(serde_json::json!({
                            "client_id": client_id_str,
                            "reason": "sender_constraint_required",
                            "scope_count": granted_scopes.len(),
                        })),
                        None,
                        None,
                        None,
                        None,
                    );
                    return Err(AppError::ExternalTokenInvalid("invalid_grant".to_string()));
                }
                let return_refresh_token = granted_scopes
                    .iter()
                    .any(|scope| scope == oauth_client_service::OFFLINE_ACCESS_SCOPE);
                let binding_refresh = if return_refresh_token {
                    oauth_service::issue_oauth_refresh_token(
                        &state.db,
                        &state.config,
                        &state.jwt_keys,
                        client_id_str,
                        &exchanged.user_id,
                        &exchanged.granted_scope,
                        &exchanged.refresh_token_resource_uris,
                        &exchanged.refresh_token_allowed_service_ids,
                        exchanged.refresh_token_allow_all_services,
                    )
                    .await?
                } else {
                    oauth_service::IssuedOAuthRefreshToken {
                        refresh_token: exchanged.refresh_token.clone(),
                        refresh_token_jti: exchanged.refresh_token_jti.clone(),
                    }
                };
                let (binding_id, binding_hash, binding_updated) =
                    if let Some(binding_hash) = exchanged.binding_grant_id.as_deref() {
                        oauth_broker_service::update_binding_grant(
                            &state.db,
                            &state.encryption_keys,
                            client_id_str,
                            &exchanged.user_id,
                            binding_hash,
                            &binding_refresh.refresh_token,
                            &binding_refresh.refresh_token_jti,
                            &granted_scopes,
                            exchanged.external_subject.as_ref(),
                        )
                        .await?;
                        (None, binding_hash.to_string(), true)
                    } else {
                        let (binding_id, binding_hash) = oauth_broker_service::create_binding(
                            &state.db,
                            &state.encryption_keys,
                            client_id_str,
                            &exchanged.user_id,
                            &binding_refresh.refresh_token,
                            &binding_refresh.refresh_token_jti,
                            &granted_scopes,
                            exchanged.external_subject.as_ref(),
                            sender_constraint.clone(),
                            broker_require_sender_constraint,
                        )
                        .await?;
                        (Some(binding_id), binding_hash, false)
                    };
                audit_service::log_async(
                    state.db.clone(),
                    Some(exchanged.user_id.clone()),
                    if binding_updated {
                        "oauth_broker_binding_grant_updated".to_string()
                    } else {
                        "oauth_broker_binding_issued".to_string()
                    },
                    Some(serde_json::json!({
                        "client_id": client_id_str,
                        "binding_hash": oauth_broker_service::binding_hash_prefix(&binding_hash),
                        "scope": &exchanged.granted_scope,
                        "sender_constraint": oauth_broker_service::sender_constraint_kind(
                            sender_constraint.as_ref(),
                        ),
                        "external_subject_platform": exchanged
                            .external_subject
                            .as_ref()
                            .map(|external_subject| external_subject.platform.clone()),
                    })),
                    crate::handlers::admin_helpers::extract_ip(headers),
                    crate::handlers::admin_helpers::extract_user_agent(headers),
                    None,
                    None,
                );

                return Ok(Json(TokenResponse {
                    access_token: exchanged.access_token,
                    token_type: if dpop_jkt.is_some() { "DPoP" } else { "Bearer" }.to_string(),
                    expires_in: oauth_broker_service::BROKER_ACCESS_TTL_SECS,
                    refresh_token: return_refresh_token.then_some(exchanged.refresh_token),
                    id_token: exchanged.id_token,
                    scope: Some(exchanged.granted_scope),
                    resource: response_resources(exchanged.resource_uris),
                    binding_id,
                    binding_updated: binding_updated.then_some(true),
                    issued_token_type: None,
                }));
            }

            Ok(Json(TokenResponse {
                access_token: exchanged.access_token,
                token_type: if dpop_jkt.is_some() { "DPoP" } else { "Bearer" }.to_string(),
                expires_in: state.config.jwt_access_ttl_secs,
                refresh_token: Some(exchanged.refresh_token),
                id_token: exchanged.id_token,
                scope: Some(exchanged.granted_scope),
                resource: response_resources(exchanged.resource_uris),
                binding_id: None,
                binding_updated: None,
                issued_token_type: None,
            }))
        }
        "refresh_token" => {
            let refresh = body.refresh_token.as_deref().ok_or_else(|| {
                AppError::BadRequest("Missing refresh_token parameter".to_string())
            })?;

            let tokens = crate::services::token_service::refresh_tokens(
                &state.db,
                &state.config,
                &state.jwt_keys,
                refresh,
                Some(&state.mcp_sessions),
                non_empty_resources(&body.resource),
            )
            .await?;

            Ok(Json(TokenResponse {
                access_token: tokens.access_token,
                token_type: "Bearer".to_string(),
                expires_in: tokens.access_expires_in,
                refresh_token: Some(tokens.refresh_token),
                id_token: None,
                scope: None,
                resource: response_resources(tokens.resource_uris),
                binding_id: None,
                binding_updated: None,
                issued_token_type: None,
            }))
        }
        // RFC 8693 Token Exchange
        "urn:ietf:params:oauth:grant-type:token-exchange" => {
            let basic_client_credentials = parse_basic_client_credentials(headers)?;
            if let (Some(form_client_id), Some((basic_client_id, _))) =
                (body.client_id.as_deref(), basic_client_credentials.as_ref())
                && form_client_id != basic_client_id
            {
                return Err(AppError::Unauthorized(
                    "Invalid client credentials".to_string(),
                ));
            }
            let client_id = body
                .client_id
                .as_deref()
                .or_else(|| {
                    basic_client_credentials
                        .as_ref()
                        .map(|(client_id, _)| client_id.as_str())
                })
                .ok_or_else(|| AppError::BadRequest("Missing client_id".to_string()))?;
            let client_secret_for_auth = body.client_secret.as_deref().or_else(|| {
                basic_client_credentials
                    .as_ref()
                    .map(|(_, client_secret)| client_secret.as_str())
            });
            let subject_token = body
                .subject_token
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing subject_token".to_string()))?;
            let subject_token_type = body
                .subject_token_type
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing subject_token_type".to_string()))?;

            // Route based on `provider` presence:
            // - provider present: social token exchange (provider-specific token type validation)
            // - provider absent + access_token type: delegated token exchange
            if let Some(provider) = body.provider.as_deref() {
                // TODO(telemetry): social branch blocked — see TELEMETRY.md §6.5
                // (auth.token_exchanged). `SocialTokenExchangeResponse` does not
                // carry `user_id`, so no distinct_id is available at this site.
                // Lift once §6.5 is resolved and the service response is extended.
                let result = social_token_exchange_service::exchange_social_token(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    &state.jwks_cache,
                    &state.http_client,
                    client_id,
                    client_secret_for_auth,
                    subject_token,
                    subject_token_type,
                    provider,
                )
                .await?;

                Ok(Json(TokenResponse {
                    access_token: result.access_token,
                    token_type: "Bearer".to_string(),
                    expires_in: result.expires_in,
                    refresh_token: Some(result.refresh_token),
                    id_token: result.id_token,
                    scope: Some(result.scope),
                    resource: None,
                    binding_id: None,
                    binding_updated: None,
                    issued_token_type: Some(
                        "urn:ietf:params:oauth:token-type:access_token".to_string(),
                    ),
                }))
            } else if subject_token_type == "urn:ietf:params:oauth:token-type:access_token" {
                // Existing: Delegated token exchange (NyxID access token -> delegated token)
                let client_secret = body
                    .client_secret
                    .as_deref()
                    .or(client_secret_for_auth)
                    .ok_or_else(|| AppError::BadRequest("Missing client_secret".to_string()))?;

                let result = token_exchange_service::exchange_token(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    client_id,
                    client_secret,
                    subject_token,
                    subject_token_type,
                    body.scope.as_deref(),
                )
                .await?;

                audit_service::log_async(
                    state.db.clone(),
                    Some(result.user_id.clone()),
                    "token_exchange".to_string(),
                    Some(serde_json::json!({
                        "client_id": client_id,
                        "scope": &result.scope,
                    })),
                    crate::handlers::admin_helpers::extract_ip(headers),
                    crate::handlers::admin_helpers::extract_user_agent(headers),
                    None,
                    None,
                );

                emit_event(
                    state.telemetry.as_deref(),
                    &result.user_id,
                    None,
                    tele,
                    TelemetryEvent::AuthTokenExchanged {
                        subject_token_type: subject_token_type.to_string(),
                        exchange_provider: None,
                    },
                );

                Ok(Json(TokenResponse {
                    access_token: result.access_token,
                    token_type: result.token_type,
                    expires_in: result.expires_in,
                    refresh_token: None,
                    id_token: None,
                    scope: Some(result.scope),
                    resource: None,
                    binding_id: None,
                    binding_updated: None,
                    issued_token_type: Some(result.issued_token_type),
                }))
            } else if subject_token_type == oauth_broker_service::BROKER_SUBJECT_TOKEN_TYPE {
                // RFC 8693 token exchange against an OauthBrokerBinding.
                let client = oauth_service::authenticate_client(
                    &state.db,
                    client_id,
                    client_secret_for_auth,
                )
                .await?;
                // Honor BOTH broker-mode triggers: the per-client admin flag
                // and the urn:nyxid:scope:broker_binding scope. Otherwise a
                // scope-opted-in client could issue bindings (commit #4 path
                // uses is_broker_client) but not exchange them.
                if !oauth_broker_service::is_broker_client_with_policy(
                    &client,
                    state.broker_require_admin_capability(),
                ) {
                    return Err(AppError::ExternalTokenInvalid("invalid_grant".to_string()));
                }

                let sender_constraint = sender_constraint_from_headers(state, headers)?;
                let dpop_jkt = sender_constraint
                    .as_ref()
                    .and_then(|cnf| cnf.jkt.as_deref());
                let mtls_x5t_s256 = sender_constraint
                    .as_ref()
                    .and_then(|cnf| cnf.x5t_s256.as_deref());

                let result = oauth_broker_service::exchange_via_binding(
                    &state.db,
                    state.encryption_keys.clone(),
                    &state.http_client,
                    &state.jwt_keys,
                    &state.config,
                    state.broker_require_sender_constraint(),
                    client_id,
                    subject_token,
                    body.scope.as_deref(),
                    dpop_jkt,
                    mtls_x5t_s256,
                )
                .await?;

                let binding_hash =
                    crate::models::oauth_broker_binding::hash_binding_id(subject_token);
                audit_service::log_async(
                    state.db.clone(),
                    None,
                    "oauth_broker_binding_token_refreshed".to_string(),
                    Some(serde_json::json!({
                        "client_id": client_id,
                        "binding_hash": oauth_broker_service::binding_hash_prefix(&binding_hash),
                        "scope": &result.granted_scope,
                        "via_chain_follow": result.via_chain_follow,
                        "sender_constraint": oauth_broker_service::sender_constraint_kind(
                            sender_constraint.as_ref(),
                        ),
                    })),
                    crate::handlers::admin_helpers::extract_ip(headers),
                    crate::handlers::admin_helpers::extract_user_agent(headers),
                    None,
                    None,
                );

                Ok(Json(TokenResponse {
                    access_token: result.access_token,
                    token_type: result.token_type,
                    expires_in: result.expires_in,
                    refresh_token: None,
                    id_token: None,
                    scope: Some(result.granted_scope),
                    resource: None,
                    binding_id: None,
                    binding_updated: None,
                    issued_token_type: Some(result.issued_token_type),
                }))
            } else {
                Err(AppError::BadRequest(format!(
                    "Unsupported subject_token_type: {subject_token_type}"
                )))
            }
        }

        // OAuth2 Client Credentials Grant (service accounts)
        "client_credentials" => {
            let client_id = body
                .client_id
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing client_id".to_string()))?;
            let client_secret = body
                .client_secret
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing client_secret".to_string()))?;

            let result = service_account_service::authenticate_client_credentials(
                &state.db,
                &state.config,
                &state.jwt_keys,
                client_id,
                client_secret,
                body.scope.as_deref(),
            )
            .await;

            match result {
                Ok(response) => {
                    audit_service::log_async(
                        state.db.clone(),
                        None,
                        "sa.token_issued".to_string(),
                        Some(serde_json::json!({
                            "client_id": client_id,
                            "scope": &response.scope,
                        })),
                        extract_ip(headers),
                        extract_user_agent(headers),
                        None,
                        None,
                    );

                    Ok(Json(TokenResponse {
                        access_token: response.access_token,
                        token_type: response.token_type,
                        expires_in: response.expires_in,
                        refresh_token: None,
                        id_token: None,
                        scope: Some(response.scope),
                        resource: None,
                        binding_id: None,
                        binding_updated: None,
                        issued_token_type: None,
                    }))
                }
                Err(e) => {
                    audit_service::log_async(
                        state.db.clone(),
                        None,
                        "sa.auth_failed".to_string(),
                        Some(serde_json::json!({ "client_id": client_id })),
                        extract_ip(headers),
                        extract_user_agent(headers),
                        None,
                        None,
                    );
                    Err(e)
                }
            }
        }

        other => Err(AppError::UnsupportedGrantType(other.to_string())),
    }
}

/// GET /oauth/userinfo
///
/// OpenID Connect UserInfo Endpoint. Returns claims about the authenticated user.
/// Includes roles/groups/permissions if the token's scope includes those scopes.
pub async fn userinfo(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UserinfoResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Check scopes from the access token claims
    let scopes: Vec<&str> = auth_user.scope.split_whitespace().collect();
    let include_roles = scopes.contains(&"roles");
    let include_groups = scopes.contains(&"groups");

    // RBAC resolution is principal-aware (users OR service_accounts) so it works
    // for both a human user and a service-account subject.
    let (roles, groups, permissions) = if include_roles || include_groups {
        let rbac =
            crate::services::rbac_helpers::resolve_user_rbac(&state.db, &user_id_str).await?;
        (
            if include_roles {
                Some(rbac.role_slugs)
            } else {
                None
            },
            if include_groups {
                Some(rbac.group_slugs)
            } else {
                None
            },
            if include_roles {
                Some(rbac.permissions)
            } else {
                None
            },
        )
    } else {
        (None, None, None)
    };

    // Service accounts have no users doc; build the subject from the SA record so
    // an SA access token gets a 200 (with SA-resolved roles) instead of a 404.
    if auth_user.auth_method == AuthMethod::ServiceAccount {
        let sa = state
            .db
            .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
            .find_one(doc! { "_id": &user_id_str })
            .await?
            .ok_or_else(|| AppError::NotFound("Service account not found".to_string()))?;
        return Ok(Json(UserinfoResponse {
            sub: sa.id,
            email: None,
            email_verified: None,
            name: Some(sa.name),
            picture: None,
            roles,
            groups,
            permissions,
        }));
    }

    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    Ok(Json(UserinfoResponse {
        sub: user.id.to_string(),
        email: Some(user.email),
        email_verified: Some(user.email_verified),
        name: user.display_name,
        picture: user.avatar_url,
        roles,
        groups,
        permissions,
    }))
}

/// POST /oauth/introspect
///
/// RFC 7662 Token Introspection. Authenticates the calling client before
/// returning token metadata. Returns `{"active": false}` for unauthenticated
/// or unauthorized callers.
pub async fn introspect(
    State(state): State<AppState>,
    Form(body): Form<IntrospectRequest>,
) -> Json<IntrospectResponse> {
    let inactive = IntrospectResponse {
        active: false,
        scope: None,
        resource: None,
        client_id: None,
        username: None,
        token_type: None,
        exp: None,
        iat: None,
        sub: None,
        iss: None,
        jti: None,
        roles: None,
        groups: None,
        permissions: None,
    };

    // Authenticate the calling client (RFC 7662 requirement)
    let caller_client_id = match body.client_id.as_deref() {
        Some(id) if !id.is_empty() => id,
        _ => return Json(inactive),
    };

    if oauth_service::authenticate_client(
        &state.db,
        caller_client_id,
        body.client_secret.as_deref(),
    )
    .await
    .is_err()
    {
        return Json(inactive);
    }

    // Broker-binding introspection: detect via the explicit token_type_hint
    // or the `bnd_` prefix as a defensive fallback. Same routing precedence
    // as /oauth/revoke's binding-revoke branch.
    let is_broker_binding = body
        .token_type_hint
        .as_deref()
        .map(|hint| hint == oauth_broker_service::BROKER_SUBJECT_TOKEN_TYPE)
        .unwrap_or(false)
        || body
            .token
            .starts_with(crate::models::oauth_broker_binding::BINDING_ID_PREFIX);

    if is_broker_binding {
        let binding = match oauth_broker_service::get_binding_for_client(
            &state.db,
            caller_client_id,
            &body.token,
        )
        .await
        {
            Ok(binding) if !binding.revoked => binding,
            _ => return Json(inactive),
        };

        return Json(IntrospectResponse {
            active: true,
            scope: Some(binding.scopes.join(" ")),
            resource: None,
            client_id: Some(binding.client_id),
            username: None,
            token_type: Some("broker_binding".to_string()),
            exp: None,
            iat: Some(binding.created_at.timestamp()),
            sub: Some(binding.user_id),
            iss: None,
            jti: None,
            roles: None,
            groups: None,
            permissions: None,
        });
    }

    // Try to verify the token
    let claims = match crate::crypto::jwt::verify_token(&state.jwt_keys, &state.config, &body.token)
    {
        Ok(c) => c,
        Err(_) => return Json(inactive),
    };

    // For refresh tokens, check if revoked in the database
    if claims.token_type == "refresh" {
        let stored = state
            .db
            .collection::<crate::models::refresh_token::RefreshToken>(
                crate::models::refresh_token::COLLECTION_NAME,
            )
            .find_one(doc! { "jti": &claims.jti })
            .await;

        match stored {
            Ok(Some(rt)) if rt.revoked => return Json(inactive),
            Err(_) => return Json(inactive),
            _ => {}
        }
    }

    // For service account tokens, check if revoked in the SA tokens collection
    if claims.sa == Some(true) {
        let sa_token = state
            .db
            .collection::<ServiceAccountToken>(SA_TOKENS)
            .find_one(doc! { "jti": &claims.jti })
            .await;
        match sa_token {
            Ok(Some(t)) if t.revoked => return Json(inactive),
            Err(_) => return Json(inactive),
            _ => {}
        }
    }

    // Fetch user email for username field
    let username = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &claims.sub })
        .await
        .ok()
        .flatten()
        .map(|u| u.email);

    // Always resolve RBAC from database rather than relying on JWT claims.
    // This ensures introspection returns correct roles/permissions even when
    // the access token was issued without them (e.g., after token refresh
    // with a scope that didn't include "roles").
    let rbac = match crate::services::rbac_helpers::resolve_user_rbac(&state.db, &claims.sub).await
    {
        Ok(rbac) => rbac,
        Err(_) => return Json(inactive),
    };

    Json(IntrospectResponse {
        active: true,
        scope: Some(claims.scope),
        resource: response_resources(claims.resources.unwrap_or_default()),
        client_id: None,
        username,
        token_type: Some(claims.token_type),
        exp: Some(claims.exp),
        iat: Some(claims.iat),
        sub: Some(claims.sub),
        iss: Some(claims.iss),
        jti: Some(claims.jti),
        roles: Some(rbac.role_slugs),
        groups: Some(rbac.group_slugs),
        permissions: Some(rbac.permissions),
    })
}

/// GET /oauth/bindings?external_subject_*=...
///
/// Reverse lookup of bindings by external_subject for a client. Auth via
/// client_credentials in Authorization: Basic or query params. Returns
/// only the caller's own active bindings.
pub async fn list_bindings_by_external_subject(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListBindingsByExternalSubjectQuery>,
) -> AppResult<Json<ListBindingsResponse>> {
    let empty = || {
        Json(ListBindingsResponse {
            bindings: Vec::new(),
        })
    };
    let basic = parse_basic_client_credentials(&headers).ok().flatten();
    let (client_id, client_secret) =
        match (basic, query.client_id.clone(), query.client_secret.clone()) {
            (Some((id, _)), Some(query_id), _) if query_id != id => return Ok(empty()),
            (Some((id, secret)), _, _) => (id, secret),
            (None, Some(id), Some(secret)) => (id, secret),
            _ => return Ok(empty()),
        };

    if oauth_service::authenticate_client(&state.db, &client_id, Some(&client_secret))
        .await
        .is_err()
    {
        return Ok(empty());
    }

    let external_subject = validate_external_subject_params(
        query.external_subject_platform.as_deref(),
        query.external_subject_tenant.as_deref(),
        query.external_subject_external_user_id.as_deref(),
    )?;
    let Some(external_subject) = external_subject else {
        return Err(AppError::BadRequest(
            "external_subject_platform and external_subject_external_user_id are required"
                .to_string(),
        ));
    };

    let bindings = oauth_broker_service::find_active_bindings_by_external_subject(
        &state.db,
        &client_id,
        &external_subject.platform,
        external_subject.tenant.as_deref(),
        &external_subject.external_user_id,
    )
    .await?;

    let summaries = bindings
        .into_iter()
        .map(|binding| BindingSummary {
            binding_hash: binding.id,
            client_id: binding.client_id,
            external_subject_ref: binding.external_subject,
            scopes: binding.scopes,
            created_at: binding.created_at.to_rfc3339(),
            last_used_at: binding.last_used_at.map(|t| t.to_rfc3339()),
            revoked: binding.revoked,
        })
        .collect();

    Ok(Json(ListBindingsResponse {
        bindings: summaries,
    }))
}

/// GET /oauth/bindings/{binding_id}
///
/// Returns metadata for an OAuth broker binding to its owning client.
/// Authenticated via client_credentials in either Authorization: Basic
/// or query params (?client_id=&client_secret=).
pub async fn get_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(raw_binding_id): axum::extract::Path<String>,
    Query(query): Query<GetBindingQuery>,
) -> AppResult<Json<GetBindingResponse>> {
    let not_found = || AppError::NotFound("binding not found".to_string());
    let basic = parse_basic_client_credentials(&headers).map_err(|_| not_found())?;
    let (client_id, client_secret) = match client_credentials_from_basic_or_params(
        basic,
        query.client_id,
        query.client_secret,
    ) {
        Some(credentials) => credentials,
        None => return Err(not_found()),
    };

    if oauth_service::authenticate_client(&state.db, &client_id, client_secret.as_deref())
        .await
        .is_err()
    {
        return Err(not_found());
    }

    let binding =
        oauth_broker_service::get_binding_for_client(&state.db, &client_id, &raw_binding_id)
            .await?;

    Ok(Json(GetBindingResponse {
        binding_id: raw_binding_id,
        client_id: binding.client_id,
        nyx_subject: binding.user_id,
        external_subject_ref: binding.external_subject,
        scopes: binding.scopes,
        created_at: binding.created_at.to_rfc3339(),
        last_used_at: binding.last_used_at.map(|t| t.to_rfc3339()),
        revoked: binding.revoked,
    }))
}

/// DELETE /oauth/bindings/{binding_id}
///
/// Client-initiated binding revocation aligned with the contract
/// proposed on issue #549. Authenticated via client_credentials in
/// Authorization: Basic or query params. Always returns 204 — missing,
/// already-revoked, and ownership-mismatched bindings are
/// indistinguishable from a successful revoke (no enumeration leak).
/// `/oauth/revoke` (RFC 7009) remains supported as the standards-track
/// alternative; this endpoint is the REST-style alias the issue spec
/// calls for.
pub async fn delete_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(raw_binding_id): axum::extract::Path<String>,
    Query(query): Query<GetBindingQuery>,
) -> StatusCode {
    let basic = parse_basic_client_credentials(&headers).ok().flatten();
    let (client_id, client_secret) = match client_credentials_from_basic_or_params(
        basic,
        query.client_id,
        query.client_secret,
    ) {
        Some(credentials) => credentials,
        None => return StatusCode::NO_CONTENT,
    };

    if oauth_service::authenticate_client(&state.db, &client_id, client_secret.as_deref())
        .await
        .is_err()
    {
        return StatusCode::NO_CONTENT;
    }

    let revoked = oauth_broker_service::revoke_binding_by_client(
        &state.db,
        state.encryption_keys.clone(),
        &state.http_client,
        &client_id,
        &raw_binding_id,
        "client_revoked",
    )
    .await
    .unwrap_or(false);

    if revoked {
        let binding_hash = crate::models::oauth_broker_binding::hash_binding_id(&raw_binding_id);
        audit_service::log_async(
            state.db.clone(),
            None,
            "oauth_broker_binding_revoked".to_string(),
            Some(serde_json::json!({
                "revoke_source": "client",
                "client_id": client_id,
                "binding_hash": oauth_broker_service::binding_hash_prefix(&binding_hash),
                "reason": "client_revoked",
            })),
            crate::handlers::admin_helpers::extract_ip(&headers),
            crate::handlers::admin_helpers::extract_user_agent(&headers),
            None,
            None,
        );
    }

    StatusCode::NO_CONTENT
}

/// POST /oauth/revoke
///
/// RFC 7009 Token Revocation. Authenticates the calling client before
/// revoking the token. Always returns 200 per the spec.
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(body): Form<RevokeRequest>,
) -> StatusCode {
    // Authenticate the calling client (RFC 7009 requirement).
    // Per the spec, always return 200 even if authentication fails.
    let basic = parse_basic_client_credentials(&headers).ok().flatten();
    let (caller_client_id, client_secret) = match client_credentials_from_basic_or_params(
        basic,
        body.client_id.clone(),
        body.client_secret.clone(),
    ) {
        Some((id, secret)) if !id.is_empty() => (id, secret),
        _ => return StatusCode::OK,
    };

    if oauth_service::authenticate_client(&state.db, &caller_client_id, client_secret.as_deref())
        .await
        .is_err()
    {
        return StatusCode::OK;
    }

    // Broker-binding revocation: detect via the explicit token_type_hint or
    // the `bnd_` prefix as a defensive fallback. RFC 7009 §2.1 makes the
    // hint optional, but standardising on the URN keeps the wire shape
    // aligned with the issued token type.
    let is_broker_binding = body
        .token_type_hint
        .as_deref()
        .map(|hint| hint == oauth_broker_service::BROKER_SUBJECT_TOKEN_TYPE)
        .unwrap_or(false)
        || body
            .token
            .starts_with(crate::models::oauth_broker_binding::BINDING_ID_PREFIX);

    if is_broker_binding {
        let revoked = oauth_broker_service::revoke_binding_by_client(
            &state.db,
            state.encryption_keys.clone(),
            &state.http_client,
            &caller_client_id,
            &body.token,
            "client_revoked",
        )
        .await
        .unwrap_or(false);

        if revoked {
            let binding_hash = crate::models::oauth_broker_binding::hash_binding_id(&body.token);
            audit_service::log_async(
                state.db.clone(),
                None,
                "oauth_broker_binding_revoked".to_string(),
                Some(serde_json::json!({
                    "revoke_source": "client",
                    "client_id": caller_client_id,
                    "binding_hash": oauth_broker_service::binding_hash_prefix(&binding_hash),
                    "reason": "client_revoked",
                })),
                crate::handlers::admin_helpers::extract_ip(&headers),
                crate::handlers::admin_helpers::extract_user_agent(&headers),
                None,
                None,
            );
        }

        // RFC 7009: always return 200 regardless of whether the token was
        // valid, owned by this client, or already revoked.
        return StatusCode::OK;
    }

    // Try to decode to get JTI for revocation
    let claims = match crate::crypto::jwt::verify_token(&state.jwt_keys, &state.config, &body.token)
    {
        Ok(c) => c,
        // Per RFC 7009, return 200 even if the token is invalid
        Err(_) => return StatusCode::OK,
    };

    if claims.token_type == "refresh" {
        // Revoke the refresh token in the database
        let _ = state
            .db
            .collection::<crate::models::refresh_token::RefreshToken>(
                crate::models::refresh_token::COLLECTION_NAME,
            )
            .update_one(
                doc! { "jti": &claims.jti, "revoked": false },
                doc! { "$set": { "revoked": true } },
            )
            .await;
        return StatusCode::OK;
    }

    // For service account tokens, revoke via the SA tokens collection
    if claims.sa == Some(true) {
        let _ = state
            .db
            .collection::<ServiceAccountToken>(SA_TOKENS)
            .update_one(
                doc! { "jti": &claims.jti, "revoked": false },
                doc! { "$set": { "revoked": true } },
            )
            .await;
        return StatusCode::OK;
    }

    // Access tokens are JWTs -- they cannot be directly revoked without a blacklist.
    // Per RFC 7009, the server SHOULD revoke the token if possible. Since access tokens
    // are short-lived and stateless, we simply return 200.

    StatusCode::OK
}

// --- Dynamic Client Registration (RFC 7591) ---

#[derive(Debug, Deserialize)]
pub struct RegisterClientRequest {
    pub client_name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    // RFC 7591 fields parsed but not yet acted on. Kept so serde accepts
    // conformant requests; remove if/when we start branching on them.
    #[allow(dead_code)]
    pub grant_types: Option<Vec<String>>,
    #[allow(dead_code)]
    pub response_types: Option<Vec<String>>,
    pub token_endpoint_auth_method: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterClientResponse {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub scope: String,
    pub client_id_issued_at: i64,
}

/// POST /oauth/register
///
/// RFC 7591 Dynamic Client Registration. MCP clients (Cursor, Claude Code, etc.)
/// call this endpoint to register themselves before starting the OAuth flow.
/// Only public clients (PKCE-based, no secret) are created via this endpoint.
// TODO(telemetry): dynamic registration has no user_id. RFC 7591 DCR is an
// unauthenticated endpoint, so there is no `AuthUser` from which to derive a
// distinct_id. `OauthClientRegistered` is emitted from the authenticated
// developer_apps path only.
pub async fn register_client(
    State(state): State<AppState>,
    Json(body): Json<RegisterClientRequest>,
) -> AppResult<(StatusCode, Json<RegisterClientResponse>)> {
    let client_name = body
        .client_name
        .unwrap_or_else(|| "Dynamic MCP Client".to_string());

    let redirect_uris = body.redirect_uris.unwrap_or_default();

    let auth_method = body.token_endpoint_auth_method.as_deref().unwrap_or("none");

    if auth_method != "none" {
        return Err(AppError::BadRequest(
            "Only token_endpoint_auth_method=none (public clients) is supported for dynamic registration".to_string(),
        ));
    }

    // Dynamic registration only creates public clients (PKCE-based, no secret).
    // Delegated RFC 8693 token exchange is controlled by `delegation_scopes`;
    // keeping it empty disables delegated token exchange for dynamic clients.
    //
    // DCR is used by MCP clients (Cursor, Claude Code, etc.) which need the
    // `proxy` scope to call `/mcp` (enforced in handlers/mcp_transport.rs).
    // Use the MCP scope set so the resulting access tokens pass that check.
    let allowed_scopes = match body.scope.as_deref().map(str::trim) {
        Some(scope) if !scope.is_empty() => oauth_client_service::validate_allowed_scopes(scope)?,
        _ => oauth_client_service::DEFAULT_MCP_ALLOWED_SCOPES.to_string(),
    };
    if state.broker_require_admin_capability()
        && allowed_scopes
            .split_whitespace()
            .any(|scope| scope == oauth_broker_service::BROKER_BINDING_SCOPE)
    {
        return Err(AppError::Forbidden(
            "Broker capability must be provisioned by a platform admin".to_string(),
        ));
    }

    let (client, _secret) = oauth_client_service::create_client(
        &state.db,
        &client_name,
        &redirect_uris,
        "public",
        "dynamic_registration",
        "",
        &allowed_scopes,
        false,
        None,
        None,
        &[],
    )
    .await?;

    tracing::info!(
        client_id = %client.id,
        client_name = %client.client_name,
        "Dynamic OAuth client registered"
    );

    Ok((
        StatusCode::CREATED,
        Json(RegisterClientResponse {
            client_id: client.id,
            client_name: client.client_name,
            redirect_uris: client.redirect_uris,
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: "none".to_string(),
            scope: client.allowed_scopes,
            client_id_issued_at: client.created_at.timestamp(),
        }),
    ))
}

#[cfg(test)]
mod wire_decoding_tests {
    //! Regression tests for #1115: the consent decision form and RFC 8707
    //! `resource` parameters arrive as repeated urlencoded keys
    //! (`allowed_service_ids=a&allowed_service_ids=b`). axum's built-in
    //! serde_urlencoded extractors reject repeated keys targeting `Vec`
    //! fields; these tests decode real wire bodies through the axum-extra
    //! extractors actually mounted on the handlers.
    use super::*;
    use axum::extract::{FromRequest, FromRequestParts};

    fn form_request(body: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn consent_decision_form_decodes_single_and_repeated_service_ids() {
        // Single selected service: the exact production failure in #1115.
        let single = "response_type=code&client_id=c1&redirect_uri=http%3A%2F%2Flocalhost%2Fcb\
             &decision=allow&allow_all_services=false\
             &allowed_service_ids=a64de078-f99f-4583-aeb0-040dc584d142";
        let Form(form) = Form::<ConsentDecisionForm>::from_request(form_request(single), &())
            .await
            .expect("single allowed_service_ids value must decode");
        assert_eq!(
            form.allowed_service_ids,
            vec!["a64de078-f99f-4583-aeb0-040dc584d142".to_string()]
        );
        assert!(!form.allow_all_services);

        let repeated = "response_type=code&client_id=c1&redirect_uri=http%3A%2F%2Flocalhost%2Fcb\
             &decision=allow&allow_all_services=false\
             &allowed_service_ids=svc-1&allowed_service_ids=svc-2\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fopenai\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fanthropic";
        let Form(form) = Form::<ConsentDecisionForm>::from_request(form_request(repeated), &())
            .await
            .expect("repeated allowed_service_ids and resource values must decode");
        assert_eq!(form.allowed_service_ids, vec!["svc-1", "svc-2"]);
        assert_eq!(form.resource.len(), 2);
    }

    #[tokio::test]
    async fn consent_decision_form_defaults_absent_vec_fields_to_empty() {
        let body = "response_type=code&client_id=c1&redirect_uri=http%3A%2F%2Flocalhost%2Fcb\
             &decision=allow&allow_all_services=true";
        let Form(form) = Form::<ConsentDecisionForm>::from_request(form_request(body), &())
            .await
            .expect("absent vec fields must decode to empty");
        assert!(form.allowed_service_ids.is_empty());
        assert!(form.resource.is_empty());
        assert!(form.allow_all_services);
    }

    #[tokio::test]
    async fn authorize_query_decodes_repeated_resource_params() {
        let uri = "/oauth/authorize?response_type=code&client_id=c1\
             &redirect_uri=http%3A%2F%2Flocalhost%2Fcb\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fopenai\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fanthropic";
        let request = axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap();
        let (mut parts, _) = request.into_parts();
        let Query(query) = Query::<AuthorizeQuery>::from_request_parts(&mut parts, &())
            .await
            .expect("repeated resource query params must decode");
        assert_eq!(query.resource.len(), 2);
        assert_eq!(query.client_id, "c1");
    }

    #[tokio::test]
    async fn token_request_decodes_repeated_resource_and_plain_grants() {
        let with_resource = "grant_type=refresh_token&refresh_token=rt-1\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fopenai\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fanthropic";
        let Form(body) = Form::<TokenRequest>::from_request(form_request(with_resource), &())
            .await
            .expect("repeated resource values must decode");
        assert_eq!(body.resource.len(), 2);

        // Scalar-only token request (standard OAuth client shape) is unchanged.
        let plain = "grant_type=authorization_code&code=abc&client_id=c1\
             &redirect_uri=http%3A%2F%2Flocalhost%2Fcb&code_verifier=ver";
        let Form(body) = Form::<TokenRequest>::from_request(form_request(plain), &())
            .await
            .expect("scalar-only token request must decode");
        assert_eq!(body.grant_type, "authorization_code");
        assert_eq!(body.code.as_deref(), Some("abc"));
        assert!(body.resource.is_empty());
    }

    #[tokio::test]
    async fn par_form_decodes_repeated_resource_params() {
        let body = "response_type=code&client_id=c1&client_secret=s1\
             &redirect_uri=http%3A%2F%2Flocalhost%2Fcb\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fopenai\
             &resource=https%3A%2F%2Fnyx.example%2Fapi%2Fv1%2Fproxy%2Fs%2Fanthropic";
        let Form(form) =
            Form::<PushedAuthorizationRequestForm>::from_request(form_request(body), &())
                .await
                .expect("repeated resource values must decode");
        assert_eq!(form.resource.len(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use chrono::{Duration, Utc};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use mongodb::bson::doc;
    use serde::Serialize;
    use uuid::Uuid;

    use crate::crypto::jwt;
    use crate::models::authorization_code::{AuthorizationCode, COLLECTION_NAME as AUTH_CODES};
    use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
    use crate::models::oauth_broker_binding::{
        COLLECTION_NAME as OAUTH_BROKER_BINDINGS, OauthBrokerBinding, hash_binding_id,
    };
    use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
    use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};
    use crate::models::ssh_auth_mode::SshAuthMode;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::oauth_broker_service::BROKER_BINDING_SCOPE;
    use crate::test_utils::{
        connect_test_database, test_app_state, test_app_state_with_config, test_membership,
        test_user, test_user_service,
    };

    #[test]
    fn consent_decision_form_defaults_service_access_to_deny() {
        let form: ConsentDecisionForm = serde_json::from_value(serde_json::json!({
            "response_type": "code",
            "client_id": "client-1",
            "redirect_uri": "http://localhost/callback",
            "scope": "openid",
            "consent_request": "signed",
            "decision": "allow",
        }))
        .expect("deserialize form");

        assert!(!form.allow_all_services);
        assert!(form.allowed_service_ids.is_empty());
    }

    #[derive(Serialize)]
    struct TestDpopClaims {
        htm: String,
        htu: String,
        iat: i64,
        jti: String,
    }

    fn sign_test_dpop_proof(
        encoding_key: &EncodingKey,
        jwk: &jsonwebtoken::jwk::Jwk,
        htu: &str,
    ) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("dpop+jwt".to_string());
        header.jwk = Some(jwk.clone());
        encode(
            &header,
            &TestDpopClaims {
                htm: "POST".to_string(),
                htu: htu.to_string(),
                iat: Utc::now().timestamp(),
                jti: Uuid::new_v4().to_string(),
            },
            encoding_key,
        )
        .expect("sign DPoP proof")
    }

    async fn insert_public_client(db: &mongodb::Database, client_id: &str, allowed_scopes: &str) {
        let now = Utc::now();
        let client = OauthClient {
            id: client_id.to_string(),
            client_name: "Public Broker Test Client".to_string(),
            client_secret_hash: String::new(),
            // Register both a loopback callback (served as the 200 success page
            // for MCP/CLI clients) and a hosted https callback (served as a bare
            // 302). Silent-issuance tests that assert the canonical 302 redirect
            // use the hosted URI; loopback-only tests keep using localhost.
            redirect_uris: vec![
                "http://localhost/callback".to_string(),
                "https://app.example/callback".to_string(),
            ],
            allowed_scopes: allowed_scopes.to_string(),
            grant_types: "authorization_code refresh_token".to_string(),
            client_type: "public".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            created_by: Some("test".to_string()),
            created_at: now,
            updated_at: now,
        };
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(client)
            .await
            .expect("insert public client");
    }

    async fn insert_binding_for_client(
        state: &AppState,
        client_id: &str,
        raw_binding_id: &str,
        user_id: &str,
        scopes: Vec<String>,
    ) {
        let user_uuid = Uuid::parse_str(user_id).expect("valid user id");
        let (refresh_jwt, refresh_jti) =
            jwt::generate_refresh_token(&state.jwt_keys, &state.config, &user_uuid)
                .expect("generate refresh jwt");
        let now = Utc::now();
        let refresh = RefreshToken {
            id: Uuid::new_v4().to_string(),
            jti: refresh_jti.clone(),
            client_id: client_id.to_string(),
            user_id: user_id.to_string(),
            session_id: None,
            scope: Some(scopes.join(" ")),
            expires_at: now + Duration::days(7),
            revoked: false,
            replaced_by: None,
            revoked_at: None,
            resource_uris: Vec::new(),
            allowed_service_ids: Vec::new(),
            allow_all_services: true,
            created_at: now,
        };
        state
            .db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(refresh)
            .await
            .expect("insert refresh token");

        let binding_hash = hash_binding_id(raw_binding_id);
        let refresh_token_encrypted = state
            .encryption_keys
            .encrypt_with_aad(refresh_jwt.as_bytes(), binding_hash.as_bytes())
            .await
            .expect("encrypt binding refresh token");
        let binding = OauthBrokerBinding {
            id: binding_hash,
            client_id: client_id.to_string(),
            user_id: user_id.to_string(),
            refresh_token_jti: refresh_jti,
            refresh_token_encrypted: Some(refresh_token_encrypted),
            scopes,
            external_subject: None,
            cnf: None,
            rotation_version: 0,
            revoked: false,
            last_used_at: None,
            revoked_at: None,
            revoke_reason: None,
            created_at: now,
        };
        state
            .db
            .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
            .insert_one(binding)
            .await
            .expect("insert broker binding");
    }

    async fn load_binding(db: &mongodb::Database, raw_binding_id: &str) -> OauthBrokerBinding {
        db.collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
            .find_one(doc! { "_id": hash_binding_id(raw_binding_id) })
            .await
            .expect("query binding")
            .expect("binding exists")
    }

    async fn insert_person_user(db: &mongodb::Database, user_id: &str) {
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(user_id, UserType::Person))
            .await
            .expect("insert user");
    }

    async fn insert_authorization_code(
        db: &mongodb::Database,
        code: &str,
        client_id: &str,
        user_id: &str,
        scope: &str,
    ) {
        let now = Utc::now();
        db.collection::<AuthorizationCode>(AUTH_CODES)
            .insert_one(AuthorizationCode {
                id: Uuid::new_v4().to_string(),
                code_hash: crate::crypto::token::hash_token(code),
                client_id: client_id.to_string(),
                user_id: user_id.to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
                scope: scope.to_string(),
                code_challenge: None,
                code_challenge_method: None,
                nonce: Some("nonce-1".to_string()),
                external_subject: None,
                binding_grant_id: None,
                resource_uris: Vec::new(),
                allowed_service_ids: Vec::new(),
                allow_all_services: true,
                expires_at: now + Duration::minutes(5),
                used: false,
                created_at: now,
            })
            .await
            .expect("insert authorization code");
    }

    async fn insert_legacy_consent(
        db: &mongodb::Database,
        user_id: &str,
        client_id: &str,
        scopes: &str,
    ) {
        db.collection::<Consent>(CONSENTS)
            .insert_one(Consent {
                id: Uuid::new_v4().to_string(),
                user_id: user_id.to_string(),
                client_id: client_id.to_string(),
                scopes: scopes.to_string(),
                allow_all_services: false,
                allowed_service_ids: None,
                granted_at: Utc::now(),
                expires_at: None,
            })
            .await
            .expect("insert legacy consent");
    }

    async fn insert_user_service(db: &mongodb::Database, user_id: &str, slug: &str) -> UserService {
        let now = Utc::now();
        let service = UserService {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            slug: slug.to_string(),
            endpoint_id: Uuid::new_v4().to_string(),
            api_key_id: None,
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            catalog_service_id: None,
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            ssh_auth_mode: SshAuthMode::ProxyOnly,
            admin_only: false,
            ssh_node_keys_stale: false,
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert user service");
        service
    }

    async fn insert_test_service(
        db: &mongodb::Database,
        owner_id: &str,
        service_id: &str,
        slug: &str,
    ) -> UserService {
        let service = test_user_service(
            service_id,
            owner_id,
            slug,
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert test service");
        service
    }

    #[tokio::test]
    async fn validate_allowed_service_ids_accepts_proxyable_org_service() {
        let Some(db) = connect_test_database("oauth_consent_org_service_allowed").await else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Member, None))
            .await
            .expect("insert membership");
        insert_test_service(&db, &org_id, &service_id, "org-openai").await;

        validate_allowed_service_ids(&db, &actor_id, &[service_id])
            .await
            .expect("proxyable org service should be grantable");
    }

    #[tokio::test]
    async fn validate_allowed_service_ids_rejects_unproxyable_org_services() {
        let Some(db) = connect_test_database("oauth_consent_org_service_denied").await else {
            return;
        };
        let member_id = Uuid::new_v4().to_string();
        let scoped_member_id = Uuid::new_v4().to_string();
        let viewer_id = Uuid::new_v4().to_string();
        let outsider_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        let admin_only_service_id = Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_many([
                test_user(&member_id, UserType::Person),
                test_user(&scoped_member_id, UserType::Person),
                test_user(&viewer_id, UserType::Person),
                test_user(&outsider_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_id, &member_id, OrgRole::Member, None),
                test_membership(
                    &org_id,
                    &scoped_member_id,
                    OrgRole::Member,
                    Some(vec![Uuid::new_v4().to_string()]),
                ),
                test_membership(&org_id, &viewer_id, OrgRole::Viewer, None),
            ])
            .await
            .expect("insert memberships");

        insert_test_service(&db, &org_id, &service_id, "org-openai").await;
        let mut admin_only =
            insert_test_service(&db, &org_id, &admin_only_service_id, "org-admin").await;
        admin_only.admin_only = true;
        db.collection::<UserService>(USER_SERVICES)
            .replace_one(doc! { "_id": &admin_only.id }, &admin_only)
            .await
            .expect("mark service admin_only");

        for (actor_id, denied_service_id) in [
            (&viewer_id, &service_id),
            (&scoped_member_id, &service_id),
            (&member_id, &admin_only_service_id),
            (&outsider_id, &service_id),
        ] {
            let err = validate_allowed_service_ids(
                &db,
                actor_id,
                std::slice::from_ref(denied_service_id),
            )
            .await
            .expect_err("unproxyable org service should be rejected");
            assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("unknown service")));
        }
    }

    fn resource_uri(slug: &str) -> String {
        format!("http://localhost:3001/api/v1/proxy/s/{slug}")
    }

    #[tokio::test]
    async fn authorize_inner_threads_stored_service_consent_into_code() {
        let Some(db) = connect_test_database("oauth_stored_service_consent_code").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "stored-service-consent-client";
        let allowed_service_ids = vec!["svc-allowed".to_string()];

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        consent_service::grant_consent_with_services(
            &db,
            &user_id,
            client_id,
            "openid",
            Some(allowed_service_ids.clone()),
        )
        .await
        .expect("grant restricted consent");

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: Vec::new(),
        };

        let _response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("authorize without prompting");

        let stored = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .find_one(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("query authorization code")
            .expect("authorization code exists");
        assert!(!stored.allow_all_services);
        assert_eq!(stored.allowed_service_ids, allowed_service_ids);
    }

    #[tokio::test]
    async fn authorize_inner_reprompts_when_resource_exceeds_service_consent() {
        let Some(db) = connect_test_database("oauth_resource_exceeds_consent").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "resource-exceeds-consent-client";
        let service_a = insert_user_service(&db, &user_id, "svc-a").await;
        let _service_b = insert_user_service(&db, &user_id, "svc-b").await;

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        consent_service::grant_consent_with_services(
            &db,
            &user_id,
            client_id,
            "openid",
            Some(vec![service_a.id.clone()]),
        )
        .await
        .expect("grant restricted consent");

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: vec![resource_uri("svc-b")],
        };

        let response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("uncovered resource redirects to consent page");

        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect location");
        assert!(location.contains("/oauth-consent?"));
        assert!(
            location
                .contains("resource=http%3A%2F%2Flocalhost%3A3001%2Fapi%2Fv1%2Fproxy%2Fs%2Fsvc-b")
        );

        let code_count = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .count_documents(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("count authorization codes");
        assert_eq!(code_count, 0);
    }

    #[tokio::test]
    async fn authorize_inner_silent_when_requested_resource_is_consented() {
        let Some(db) = connect_test_database("oauth_resource_within_consent").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "resource-within-consent-client";
        let service_a = insert_user_service(&db, &user_id, "svc-a").await;

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        consent_service::grant_consent_with_services(
            &db,
            &user_id,
            client_id,
            "openid",
            Some(vec![service_a.id.clone()]),
        )
        .await
        .expect("grant restricted consent");

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "https://app.example/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: vec![resource_uri("svc-a")],
        };

        let response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("covered resource issues code silently");

        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect location");
        assert!(location.starts_with("https://app.example/callback?code="));

        let stored = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .find_one(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("query authorization code")
            .expect("authorization code exists");
        assert!(!stored.allow_all_services);
        assert_eq!(stored.allowed_service_ids, vec![service_a.id]);
        assert_eq!(stored.resource_uris, vec![resource_uri("svc-a")]);
    }

    #[tokio::test]
    async fn authorize_inner_prompt_none_uncovered_resource_returns_consent_required() {
        let Some(db) = connect_test_database("oauth_resource_prompt_none").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "resource-prompt-none-client";
        let service_a = insert_user_service(&db, &user_id, "svc-a").await;
        let _service_b = insert_user_service(&db, &user_id, "svc-b").await;

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        consent_service::grant_consent_with_services(
            &db,
            &user_id,
            client_id,
            "openid",
            Some(vec![service_a.id]),
        )
        .await
        .expect("grant restricted consent");

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: Some("none".to_string()),
            request_uri: None,
            resource: vec![resource_uri("svc-b")],
        };

        let response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("prompt=none returns OAuth error redirect");

        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect location");
        assert!(location.contains("error=consent_required"));

        let code_count = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .count_documents(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("count authorization codes");
        assert_eq!(code_count, 0);
    }

    #[tokio::test]
    async fn authorize_inner_legacy_consent_requires_interactive_reconsent() {
        let Some(db) = connect_test_database("oauth_legacy_consent_reprompt").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "legacy-consent-client";

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        insert_legacy_consent(&db, &user_id, client_id, "openid").await;

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: Vec::new(),
        };

        let response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("legacy consent redirects to consent page");

        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect location");
        assert!(location.contains("/oauth-consent?"));

        let code_count = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .count_documents(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("count authorization codes");
        assert_eq!(code_count, 0);
    }

    #[tokio::test]
    async fn authorize_inner_prompt_none_legacy_consent_returns_consent_required() {
        let Some(db) = connect_test_database("oauth_legacy_consent_prompt_none").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "legacy-consent-prompt-none-client";

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        insert_legacy_consent(&db, &user_id, client_id, "openid").await;

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: Some("none".to_string()),
            request_uri: None,
            resource: Vec::new(),
        };

        let response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("prompt=none returns OAuth error redirect");

        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect location");
        assert!(location.contains("error=consent_required"));
    }

    #[tokio::test]
    async fn authorize_inner_explicit_all_services_consent_issues_code() {
        let Some(db) = connect_test_database("oauth_explicit_all_services_consent").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "explicit-all-services-client";

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        consent_service::grant_consent_with_services(&db, &user_id, client_id, "openid", None)
            .await
            .expect("grant explicit unrestricted consent");

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: Vec::new(),
        };

        let _response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("explicit unrestricted consent issues code");

        let stored = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .find_one(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("query authorization code")
            .expect("authorization code exists");
        assert!(stored.allow_all_services);
        assert!(stored.allowed_service_ids.is_empty());
    }

    #[tokio::test]
    async fn authorize_inner_allow_all_consent_silently_narrows_requested_resource() {
        let Some(db) = connect_test_database("oauth_all_services_resource_narrowing").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "all-services-resource-narrowing-client";
        let service_a = insert_user_service(&db, &user_id, "svc-a").await;

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;
        consent_service::grant_consent_with_services(&db, &user_id, client_id, "openid", None)
            .await
            .expect("grant explicit unrestricted consent");

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "https://app.example/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: vec![resource_uri("svc-a")],
        };

        let response = authorize_inner(
            &state,
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            &params,
            true,
            None,
        )
        .await
        .expect("allow-all consent issues narrowed resource code silently");

        assert_eq!(response.status(), StatusCode::FOUND);
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .expect("redirect location");
        assert!(location.starts_with("https://app.example/callback?code="));

        let stored = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .find_one(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("query authorization code")
            .expect("authorization code exists");
        assert!(!stored.allow_all_services);
        assert_eq!(stored.allowed_service_ids, vec![service_a.id]);
        assert_eq!(stored.resource_uris, vec![resource_uri("svc-a")]);
    }

    #[tokio::test]
    async fn authorize_decision_empty_selection_stores_explicit_zero_service_grant() {
        let Some(db) = connect_test_database("oauth_consent_empty_service_selection").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "empty-service-selection-client";

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, "openid").await;

        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: Vec::new(),
        };
        let consent_request = sign_consent_request(&state, &user_id, &params, "openid")
            .expect("sign consent request");

        let response = authorize_decision(
            State(state),
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            TelemetryContext::default(),
            Form(ConsentDecisionForm {
                response_type: "code".to_string(),
                client_id: client_id.to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
                scope: Some("openid".to_string()),
                state: Some("state-1".to_string()),
                code_challenge: Some("challenge".to_string()),
                code_challenge_method: Some("S256".to_string()),
                nonce: None,
                external_subject_platform: None,
                external_subject_tenant: None,
                external_subject_external_user_id: None,
                binding_grant_id: None,
                prompt: None,
                allow_all_services: false,
                allowed_service_ids: Vec::new(),
                consent_request: Some(consent_request),
                resource: Vec::new(),
                decision: "allow".to_string(),
            }),
        )
        .await
        .expect("approve consent");

        assert_eq!(response.status(), StatusCode::OK);

        let consent = db
            .collection::<Consent>(crate::models::consent::COLLECTION_NAME)
            .find_one(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("query consent")
            .expect("consent exists");
        assert!(!consent.allow_all_services);
        assert_eq!(consent.allowed_service_ids, Some(Vec::new()));

        let stored = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .find_one(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("query authorization code")
            .expect("authorization code exists");
        assert!(!stored.allow_all_services);
        assert!(stored.allowed_service_ids.is_empty());
    }

    #[tokio::test]
    async fn register_client_persists_requested_broker_scope() {
        let Some(db) = connect_test_database("oauth_dcr_broker_scope").await else {
            return;
        };
        let state = test_app_state(db.clone());

        let (status, Json(response)) = register_client(
            State(state),
            Json(RegisterClientRequest {
                client_name: Some("Aevatar".to_string()),
                redirect_uris: Some(vec!["http://localhost/callback".to_string()]),
                grant_types: None,
                response_types: None,
                token_endpoint_auth_method: Some("none".to_string()),
                scope: Some(format!("openid {BROKER_BINDING_SCOPE}")),
            }),
        )
        .await
        .expect("register client");

        assert_eq!(status, StatusCode::CREATED);
        assert!(
            response
                .scope
                .split_whitespace()
                .any(|s| s == BROKER_BINDING_SCOPE)
        );

        let client = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": &response.client_id })
            .await
            .expect("query client")
            .expect("client exists");
        assert_eq!(client.allowed_scopes, response.scope);
        assert!(oauth_broker_service::is_broker_client_with_policy(
            &client, false
        ));
    }

    #[tokio::test]
    async fn register_client_rejects_broker_scope_when_admin_capability_required() {
        let Some(db) = connect_test_database("oauth_dcr_broker_scope_strict").await else {
            return;
        };
        let mut config = crate::test_utils::test_app_config();
        config.broker_require_admin_capability = true;
        let state = test_app_state_with_config(db, config);

        let err = register_client(
            State(state),
            Json(RegisterClientRequest {
                client_name: Some("Aevatar".to_string()),
                redirect_uris: Some(vec!["http://localhost/callback".to_string()]),
                grant_types: None,
                response_types: None,
                token_endpoint_auth_method: Some("none".to_string()),
                scope: Some(format!("openid {BROKER_BINDING_SCOPE}")),
            }),
        )
        .await
        .expect_err("strict DCR rejects broker scope");

        assert!(matches!(err, AppError::Forbidden(message) if message.contains("platform admin")));
    }

    #[tokio::test]
    async fn register_client_accepts_offline_access_with_broker_scope() {
        let Some(db) = connect_test_database("oauth_dcr_broker_offline").await else {
            return;
        };
        let state = test_app_state(db.clone());

        let (status, Json(response)) = register_client(
            State(state),
            Json(RegisterClientRequest {
                client_name: Some("Aevatar".to_string()),
                redirect_uris: Some(vec!["http://localhost/callback".to_string()]),
                grant_types: None,
                response_types: None,
                token_endpoint_auth_method: Some("none".to_string()),
                scope: Some(format!(
                    "openid offline_access proxy {BROKER_BINDING_SCOPE}"
                )),
            }),
        )
        .await
        .expect("register client");

        assert_eq!(status, StatusCode::CREATED);
        let scopes: Vec<&str> = response.scope.split_whitespace().collect();
        assert!(scopes.contains(&"offline_access"));
        assert!(scopes.contains(&BROKER_BINDING_SCOPE));
        let client = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": &response.client_id })
            .await
            .expect("query client")
            .expect("client exists");
        assert_eq!(client.allowed_scopes, response.scope);
    }

    #[tokio::test]
    async fn broker_authorization_code_with_offline_access_returns_refresh_token_and_binding() {
        let Some(db) = connect_test_database("oauth_broker_offline_token").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let client_id = "public-broker-offline-token";
        let user_id = Uuid::new_v4().to_string();
        let scope = format!("openid profile offline_access proxy {BROKER_BINDING_SCOPE}");
        let code = "broker-offline-code";

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, &scope).await;
        insert_authorization_code(&db, code, client_id, &user_id, &scope).await;

        let Json(response) = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "authorization_code".to_string(),
                code: Some(code.to_string()),
                redirect_uri: Some("http://localhost/callback".to_string()),
                client_id: Some(client_id.to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect("exchange authorization code");

        assert_eq!(response.token_type, "Bearer");
        assert_eq!(
            response.expires_in,
            oauth_broker_service::BROKER_ACCESS_TTL_SECS
        );
        assert_eq!(response.scope.as_deref(), Some(scope.as_str()));
        let refresh_token = response.refresh_token.expect("refresh_token returned");
        let binding_id = response.binding_id.expect("binding_id returned");
        assert!(!refresh_token.is_empty());
        assert!(!binding_id.is_empty());

        let refresh_claims = jwt::verify_token(&state.jwt_keys, &state.config, &refresh_token)
            .expect("client refresh token verifies");
        let binding = load_binding(&db, &binding_id).await;
        assert_ne!(
            refresh_claims.jti, binding.refresh_token_jti,
            "client refresh token and broker binding must not share rotation state"
        );

        let refresh_count = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! { "client_id": client_id, "user_id": &user_id, "revoked": false })
            .await
            .expect("count refresh tokens");
        assert_eq!(refresh_count, 2);

        let Json(refreshed) = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "refresh_token".to_string(),
                code: None,
                redirect_uri: None,
                client_id: None,
                client_secret: None,
                code_verifier: None,
                refresh_token: Some(refresh_token),
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect("refresh returned token");
        assert!(!refreshed.access_token.is_empty());
        let rotated_refresh_token = refreshed
            .refresh_token
            .expect("rotated refresh_token returned");

        let Json(refreshed_again) = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "refresh_token".to_string(),
                code: None,
                redirect_uri: None,
                client_id: None,
                client_secret: None,
                code_verifier: None,
                refresh_token: Some(rotated_refresh_token),
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect("refresh rotated token");
        assert!(!refreshed_again.access_token.is_empty());
        assert!(refreshed_again.refresh_token.is_some());

        let Json(binding_exchange) = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "urn:ietf:params:oauth:grant-type:token-exchange".to_string(),
                code: None,
                redirect_uri: None,
                client_id: Some(client_id.to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: Some(binding_id),
                subject_token_type: Some(
                    oauth_broker_service::BROKER_SUBJECT_TOKEN_TYPE.to_string(),
                ),
                scope: Some("openid".to_string()),
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect("exchange binding after client refresh");
        assert_eq!(binding_exchange.token_type, "Bearer");
        assert!(!binding_exchange.access_token.is_empty());
        assert_eq!(binding_exchange.scope.as_deref(), Some("openid"));
    }

    #[tokio::test]
    async fn broker_authorization_code_strict_unpinned_reject_leaves_no_refresh_token() {
        let Some(db) = connect_test_database("oauth_broker_strict_unpinned_no_refresh").await
        else {
            return;
        };
        let mut config = crate::test_utils::test_app_config();
        config.broker_require_sender_constraint = true;
        let state = test_app_state_with_config(db.clone(), config);
        let client_id = "public-broker-strict-unpinned";
        let user_id = Uuid::new_v4().to_string();
        let scope = format!("openid profile offline_access proxy {BROKER_BINDING_SCOPE}");
        let code = "broker-strict-unpinned-code";

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, &scope).await;
        insert_authorization_code(&db, code, client_id, &user_id, &scope).await;

        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "authorization_code".to_string(),
                code: Some(code.to_string()),
                redirect_uri: Some("http://localhost/callback".to_string()),
                client_id: Some(client_id.to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("strict broker binding create requires sender proof");

        assert!(
            matches!(err, AppError::ExternalTokenInvalid(message) if message == "invalid_grant")
        );

        let refresh_count = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("count refresh tokens");
        assert_eq!(refresh_count, 0);

        let binding_count = db
            .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
            .count_documents(doc! { "client_id": client_id, "user_id": &user_id })
            .await
            .expect("count bindings");
        assert_eq!(binding_count, 0);
    }

    #[tokio::test]
    async fn broker_authorization_code_pins_dpop_sender_constraint_on_binding() {
        let Some(db) = connect_test_database("oauth_broker_code_pins_dpop").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let client_id = "public-broker-dpop-code";
        let user_id = Uuid::new_v4().to_string();
        let scope = format!("openid profile {BROKER_BINDING_SCOPE}");
        let code = "broker-dpop-code";

        insert_person_user(&db, &user_id).await;
        insert_public_client(&db, client_id, &scope).await;
        insert_authorization_code(&db, code, client_id, &user_id, &scope).await;

        let (encoding_key, jwk) = crate::crypto::dpop::test_dpop_keypair();
        let htu =
            crate::crypto::dpop::htu_from_base_and_path(&state.config.base_url, "/oauth/token")
                .expect("token htu");
        let proof = sign_test_dpop_proof(&encoding_key, &jwk, &htu);
        let expected_jkt = crate::crypto::dpop::jwk_thumbprint(&jwk);
        let mut headers = HeaderMap::new();
        headers.insert("dpop", proof.parse().expect("dpop header value"));

        let Json(response) = token_inner(
            &state,
            &TelemetryContext::default(),
            &headers,
            TokenRequest {
                grant_type: "authorization_code".to_string(),
                code: Some(code.to_string()),
                redirect_uri: Some("http://localhost/callback".to_string()),
                client_id: Some(client_id.to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect("exchange authorization code with DPoP proof");

        assert_eq!(response.token_type, "DPoP");
        let binding_id = response.binding_id.expect("binding_id returned");
        let binding = load_binding(&db, &binding_id).await;
        let cnf = binding.cnf.expect("binding cnf");
        assert_eq!(cnf.jkt.as_deref(), Some(expected_jkt.as_str()));
        assert!(cnf.x5t_s256.is_none());
    }

    #[tokio::test]
    async fn register_client_rejects_unknown_scope() {
        let Some(db) = connect_test_database("oauth_dcr_unknown_scope").await else {
            return;
        };
        let state = test_app_state(db);

        let result = register_client(
            State(state),
            Json(RegisterClientRequest {
                client_name: Some("Bad Scope".to_string()),
                redirect_uris: Some(vec!["http://localhost/callback".to_string()]),
                grant_types: None,
                response_types: None,
                token_endpoint_auth_method: Some("none".to_string()),
                scope: Some("openid unknown_scope".to_string()),
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[tokio::test]
    async fn register_client_without_scope_uses_default_mcp_scopes() {
        let Some(db) = connect_test_database("oauth_dcr_default_scope").await else {
            return;
        };
        let state = test_app_state(db);

        let (_status, Json(response)) = register_client(
            State(state),
            Json(RegisterClientRequest {
                client_name: Some("Default Scope".to_string()),
                redirect_uris: Some(vec!["http://localhost/callback".to_string()]),
                grant_types: None,
                response_types: None,
                token_endpoint_auth_method: Some("none".to_string()),
                scope: None,
            }),
        )
        .await
        .expect("register client");

        assert_eq!(
            response.scope,
            oauth_client_service::DEFAULT_MCP_ALLOWED_SCOPES
        );
    }

    #[tokio::test]
    async fn broker_token_exchange_accepts_public_client_without_secret() {
        let Some(db) = connect_test_database("oauth_broker_public_exchange").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let client_id = "public-broker-exchange";
        let raw_binding_id = crate::models::oauth_broker_binding::generate_binding_id();
        let user_id = Uuid::new_v4().to_string();
        insert_public_client(
            &db,
            client_id,
            &format!("openid profile {BROKER_BINDING_SCOPE}"),
        )
        .await;
        insert_binding_for_client(
            &state,
            client_id,
            &raw_binding_id,
            &user_id,
            vec!["openid".to_string(), "profile".to_string()],
        )
        .await;

        let Json(response) = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "urn:ietf:params:oauth:grant-type:token-exchange".to_string(),
                code: None,
                redirect_uri: None,
                client_id: Some(client_id.to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: Some(raw_binding_id),
                subject_token_type: Some(
                    oauth_broker_service::BROKER_SUBJECT_TOKEN_TYPE.to_string(),
                ),
                scope: Some("openid".to_string()),
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect("exchange binding");

        assert_eq!(response.token_type, "Bearer");
        assert!(!response.access_token.is_empty());
        assert_eq!(response.scope.as_deref(), Some("openid"));
    }

    #[tokio::test]
    async fn delete_binding_accepts_public_client_id_without_secret_and_preserves_ownership() {
        let Some(db) = connect_test_database("oauth_broker_public_delete").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let client_id = "public-broker-delete";
        let other_client_id = "public-broker-delete-other";
        let raw_binding_id = crate::models::oauth_broker_binding::generate_binding_id();
        let other_raw_binding_id = crate::models::oauth_broker_binding::generate_binding_id();
        let user_id = Uuid::new_v4().to_string();
        insert_public_client(
            &db,
            client_id,
            &format!("openid profile {BROKER_BINDING_SCOPE}"),
        )
        .await;
        insert_public_client(
            &db,
            other_client_id,
            &format!("openid profile {BROKER_BINDING_SCOPE}"),
        )
        .await;
        insert_binding_for_client(
            &state,
            client_id,
            &raw_binding_id,
            &user_id,
            vec!["openid".to_string()],
        )
        .await;
        insert_binding_for_client(
            &state,
            client_id,
            &other_raw_binding_id,
            &user_id,
            vec!["openid".to_string()],
        )
        .await;

        let status = delete_binding(
            State(state.clone()),
            HeaderMap::new(),
            Path(raw_binding_id.clone()),
            Query(GetBindingQuery {
                client_id: Some(client_id.to_string()),
                client_secret: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(load_binding(&db, &raw_binding_id).await.revoked);

        let wrong_client_status = delete_binding(
            State(state),
            HeaderMap::new(),
            Path(other_raw_binding_id.clone()),
            Query(GetBindingQuery {
                client_id: Some(other_client_id.to_string()),
                client_secret: None,
            }),
        )
        .await;
        assert_eq!(wrong_client_status, StatusCode::NO_CONTENT);
        assert!(!load_binding(&db, &other_raw_binding_id).await.revoked);
    }

    #[test]
    fn oauth_error_response_maps_unsupported_grant_type() {
        let err = AppError::UnsupportedGrantType("magic_grant".to_string());
        let response = oauth_error_response(err);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn oauth_error_response_maps_internal_error_without_leak() {
        let err = AppError::Internal("secret DB detail".to_string());
        let response = oauth_error_response(err);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn parse_basic_client_credentials_returns_none_for_missing_header() {
        let headers = HeaderMap::new();
        let result = parse_basic_client_credentials(&headers).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_basic_client_credentials_decodes_valid_basic() {
        let mut headers = HeaderMap::new();
        let encoded = base64::engine::general_purpose::STANDARD.encode("my_client:my_secret");
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Basic {encoded}").parse().unwrap(),
        );
        let (client_id, client_secret) = parse_basic_client_credentials(&headers).unwrap().unwrap();
        assert_eq!(client_id, "my_client");
        assert_eq!(client_secret, "my_secret");
    }

    #[test]
    fn parse_basic_client_credentials_rejects_invalid_base64() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Basic not!valid!base64!!!".parse().unwrap(),
        );
        let err = parse_basic_client_credentials(&headers);
        assert!(err.is_err());
    }

    #[test]
    fn client_credentials_from_basic_or_params_prefers_basic() {
        let basic = Some(("basic_id".to_string(), "basic_secret".to_string()));
        let result = client_credentials_from_basic_or_params(basic, None, None);
        assert_eq!(result.unwrap().0, "basic_id");
    }

    #[test]
    fn client_credentials_from_basic_or_params_rejects_conflicting_ids() {
        let basic = Some(("basic_id".to_string(), "basic_secret".to_string()));
        let result =
            client_credentials_from_basic_or_params(basic, Some("different_id".to_string()), None);
        assert!(result.is_none());
    }

    #[test]
    fn client_credentials_from_basic_or_params_falls_back_to_form() {
        let result = client_credentials_from_basic_or_params(
            None,
            Some("form_id".to_string()),
            Some("form_secret".to_string()),
        );
        let (id, secret) = result.unwrap();
        assert_eq!(id, "form_id");
        assert_eq!(secret.unwrap(), "form_secret");
    }

    #[test]
    fn client_credentials_from_basic_or_params_returns_none_for_empty() {
        let result = client_credentials_from_basic_or_params(None, None, None);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn token_inner_rejects_missing_code_for_authorization_code() {
        let Some(db) = connect_test_database("oauth_ext_missing_code").await else {
            return;
        };
        let state = test_app_state(db);
        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "authorization_code".to_string(),
                code: None,
                redirect_uri: Some("http://localhost/callback".to_string()),
                client_id: Some("test-client".to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("should reject missing code");
        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("code")));
    }

    #[tokio::test]
    async fn token_inner_rejects_missing_refresh_token() {
        let Some(db) = connect_test_database("oauth_ext_missing_refresh").await else {
            return;
        };
        let state = test_app_state(db);
        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "refresh_token".to_string(),
                code: None,
                redirect_uri: None,
                client_id: None,
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("should reject missing refresh_token");
        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("refresh_token")));
    }

    #[tokio::test]
    async fn token_inner_rejects_unsupported_grant_type() {
        let Some(db) = connect_test_database("oauth_ext_bad_grant").await else {
            return;
        };
        let state = test_app_state(db);
        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "magic_grant".to_string(),
                code: None,
                redirect_uri: None,
                client_id: None,
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("should reject unsupported grant_type");
        assert!(matches!(err, AppError::UnsupportedGrantType(_)));
    }

    #[tokio::test]
    async fn authorize_decision_rejects_resource_selection_outside_signed_request() {
        let Some(db) = connect_test_database("oauth_consent_resource_tamper").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let client_id = "oauth-consent-resource-tamper-client";
        let requested_resource = "http://localhost:3001/api/v1/proxy/s/openai".to_string();
        let tampered_resource = "http://localhost:3001/api/v1/proxy/s/anthropic".to_string();
        let now = Utc::now();

        insert_person_user(&db, &user_id).await;
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.to_string(),
                client_name: "Resource Tamper Test".to_string(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["http://localhost/callback".to_string()],
                allowed_scopes: "openid".to_string(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                default_service_catalog_slugs: Vec::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                created_by: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert oauth client");

        let trusted_params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scope: Some("openid".to_string()),
            state: Some("state-1".to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: vec![requested_resource],
        };
        let consent_request =
            sign_consent_request(&state, &user_id, &trusted_params, "openid").unwrap();

        let err = match authorize_decision(
            State(state),
            OptionalAuthUser(Some(crate::test_utils::test_auth_user(&user_id))),
            TelemetryContext::default(),
            Form(ConsentDecisionForm {
                response_type: "code".to_string(),
                client_id: client_id.to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
                scope: Some("openid".to_string()),
                state: Some("state-1".to_string()),
                code_challenge: Some("challenge".to_string()),
                code_challenge_method: Some("S256".to_string()),
                nonce: None,
                external_subject_platform: None,
                external_subject_tenant: None,
                external_subject_external_user_id: None,
                binding_grant_id: None,
                prompt: None,
                allow_all_services: true,
                allowed_service_ids: Vec::new(),
                consent_request: Some(consent_request),
                resource: vec![tampered_resource],
                decision: "allow".to_string(),
            }),
        )
        .await
        {
            Ok(_) => panic!("tampered resource selection must be rejected"),
            Err(err) => err,
        };

        assert!(
            matches!(err, AppError::InvalidTarget(msg) if msg.contains("original authorization request"))
        );
    }

    #[tokio::test]
    async fn token_inner_client_credentials_rejects_missing_client_id() {
        let Some(db) = connect_test_database("oauth_ext_cc_no_id").await else {
            return;
        };
        let state = test_app_state(db);
        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "client_credentials".to_string(),
                code: None,
                redirect_uri: None,
                client_id: None,
                client_secret: Some("some-secret".to_string()),
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("should reject missing client_id");
        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("client_id")));
    }

    #[tokio::test]
    async fn token_inner_client_credentials_rejects_missing_secret() {
        let Some(db) = connect_test_database("oauth_ext_cc_no_secret").await else {
            return;
        };
        let state = test_app_state(db);
        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "client_credentials".to_string(),
                code: None,
                redirect_uri: None,
                client_id: Some("some-client".to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: None,
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("should reject missing client_secret");
        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("client_secret")));
    }

    #[tokio::test]
    async fn token_exchange_rejects_missing_subject_token() {
        let Some(db) = connect_test_database("oauth_ext_te_no_subject").await else {
            return;
        };
        let state = test_app_state(db);
        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "urn:ietf:params:oauth:grant-type:token-exchange".to_string(),
                code: None,
                redirect_uri: None,
                client_id: Some("some-client".to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: None,
                subject_token_type: Some(
                    "urn:ietf:params:oauth:token-type:access_token".to_string(),
                ),
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("should reject missing subject_token");
        assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("subject_token")));
    }

    #[tokio::test]
    async fn token_exchange_rejects_unsupported_subject_token_type() {
        let Some(db) = connect_test_database("oauth_ext_te_bad_type").await else {
            return;
        };
        let state = test_app_state(db);
        let err = token_inner(
            &state,
            &TelemetryContext::default(),
            &HeaderMap::new(),
            TokenRequest {
                grant_type: "urn:ietf:params:oauth:grant-type:token-exchange".to_string(),
                code: None,
                redirect_uri: None,
                client_id: Some("some-client".to_string()),
                client_secret: None,
                code_verifier: None,
                refresh_token: None,
                subject_token: Some("some-token".to_string()),
                subject_token_type: Some("urn:unknown:type".to_string()),
                scope: None,
                provider: None,
                resource: Vec::new(),
            },
        )
        .await
        .expect_err("should reject unsupported subject_token_type");
        assert!(
            matches!(err, AppError::BadRequest(msg) if msg.contains("Unsupported subject_token_type"))
        );
    }

    #[test]
    fn needs_success_page_returns_true_for_loopback() {
        assert!(needs_success_page("http://127.0.0.1/callback"));
        assert!(needs_success_page("http://localhost/callback"));
        assert!(needs_success_page("http://[::1]/callback"));
    }

    #[test]
    fn needs_success_page_returns_true_for_custom_scheme() {
        assert!(needs_success_page("cursor://callback"));
        assert!(needs_success_page("vscode://callback"));
    }

    #[test]
    fn needs_success_page_returns_false_for_remote_url() {
        assert!(!needs_success_page("https://app.example.com/callback"));
    }

    #[test]
    fn accepts_json_returns_true_for_json_accept() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", "application/json".parse().unwrap());
        assert!(accepts_json(&headers));
    }

    #[test]
    fn accepts_json_returns_false_for_html_accept() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", "text/html".parse().unwrap());
        assert!(!accepts_json(&headers));
    }

    #[test]
    fn parse_prompt_empty_returns_empty_set() {
        assert!(parse_prompt(None).is_empty());
        assert!(parse_prompt(Some("")).is_empty());
    }

    #[test]
    fn parse_prompt_splits_space_separated_values() {
        let prompts = parse_prompt(Some("login consent"));
        assert!(prompts.contains("login"));
        assert!(prompts.contains("consent"));
        assert_eq!(prompts.len(), 2);
    }

    #[test]
    fn intersect_service_ids_preserves_requested_order_and_dedupes() {
        let requested = vec![
            "svc-2".to_string(),
            "svc-1".to_string(),
            "svc-2".to_string(),
            "svc-3".to_string(),
        ];
        let consented = vec!["svc-1".to_string(), "svc-2".to_string()];

        assert_eq!(
            intersect_service_ids(&requested, &consented),
            vec!["svc-2".to_string(), "svc-1".to_string()]
        );
    }

    #[test]
    fn intersect_service_ids_empty_when_disjoint() {
        let requested = vec!["svc-3".to_string()];
        let consented = vec!["svc-1".to_string(), "svc-2".to_string()];

        assert!(intersect_service_ids(&requested, &consented).is_empty());
    }

    #[tokio::test]
    async fn get_binding_accepts_public_client_id_without_secret() {
        let Some(db) = connect_test_database("oauth_broker_public_get").await else {
            return;
        };
        let state = test_app_state(db.clone());
        let client_id = "public-broker-get";
        let other_client_id = "public-broker-get-other";
        let raw_binding_id = crate::models::oauth_broker_binding::generate_binding_id();
        let user_id = Uuid::new_v4().to_string();
        insert_public_client(
            &db,
            client_id,
            &format!("openid profile {BROKER_BINDING_SCOPE}"),
        )
        .await;
        insert_public_client(
            &db,
            other_client_id,
            &format!("openid profile {BROKER_BINDING_SCOPE}"),
        )
        .await;
        insert_binding_for_client(
            &state,
            client_id,
            &raw_binding_id,
            &user_id,
            vec!["openid".to_string()],
        )
        .await;

        let Json(response) = get_binding(
            State(state.clone()),
            HeaderMap::new(),
            Path(raw_binding_id.clone()),
            Query(GetBindingQuery {
                client_id: Some(client_id.to_string()),
                client_secret: None,
            }),
        )
        .await
        .expect("get binding");
        assert_eq!(response.client_id, client_id);
        assert_eq!(response.nyx_subject, user_id);

        let wrong_owner = get_binding(
            State(state),
            HeaderMap::new(),
            Path(raw_binding_id),
            Query(GetBindingQuery {
                client_id: Some(other_client_id.to_string()),
                client_secret: None,
            }),
        )
        .await;
        assert!(matches!(wrong_owner, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn resolve_app_default_service_hints_matches_user_services_and_reports_unmatched() {
        let Some(db) = crate::test_utils::connect_test_database("oauth_default_hints").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let suffix = Uuid::new_v4().to_string();

        let mut matched_catalog = crate::models::downstream_service::test_helpers::dummy_service();
        matched_catalog.id = Uuid::new_v4().to_string();
        matched_catalog.slug = format!("hint-matched-{suffix}");
        matched_catalog.name = "Matched Service".to_string();
        let mut unmatched_catalog =
            crate::models::downstream_service::test_helpers::dummy_service();
        unmatched_catalog.id = Uuid::new_v4().to_string();
        unmatched_catalog.slug = format!("hint-unmatched-{suffix}");
        unmatched_catalog.name = "Unmatched Service".to_string();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_many(vec![&matched_catalog, &unmatched_catalog])
        .await
        .expect("insert catalog docs");

        let user_service_id = Uuid::new_v4().to_string();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(crate::test_utils::test_user_service(
                &user_service_id,
                &user_id,
                &format!("mine-{suffix}"),
                "endpoint-1",
                Some(&matched_catalog.id),
                None,
            ))
            .await
            .expect("insert user service");

        let mut client = OauthClient {
            id: Uuid::new_v4().to_string(),
            client_name: "Hint App".to_string(),
            client_secret_hash: "NONE".to_string(),
            redirect_uris: vec!["http://localhost/cb".to_string()],
            allowed_scopes: "openid".to_string(),
            grant_types: "authorization_code".to_string(),
            client_type: "public".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: vec![
                matched_catalog.slug.clone(),
                unmatched_catalog.slug.clone(),
            ],
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            created_by: Some("dev".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let hints = resolve_app_default_service_hints(&state, &client, &user_id)
            .await
            .expect("resolve hints");
        assert_eq!(hints.preselect_service_ids, vec![user_service_id.clone()]);
        assert_eq!(hints.unmatched_names, vec!["Unmatched Service".to_string()]);

        // Another user gets no matches -- both catalog names unmatched.
        let other = Uuid::new_v4().to_string();
        let other_hints = resolve_app_default_service_hints(&state, &client, &other)
            .await
            .expect("resolve hints for other user");
        assert!(other_hints.preselect_service_ids.is_empty());
        assert_eq!(other_hints.unmatched_names.len(), 2);

        // No declared defaults -> empty hints, no queries needed.
        client.default_service_catalog_slugs.clear();
        let empty = resolve_app_default_service_hints(&state, &client, &user_id)
            .await
            .expect("resolve empty hints");
        assert!(empty.preselect_service_ids.is_empty());
        assert!(empty.unmatched_names.is_empty());
    }

    #[test]
    fn build_consent_url_appends_default_service_hints_as_repeated_params() {
        let params = AuthorizeQuery {
            response_type: "code".to_string(),
            client_id: "c1".to_string(),
            redirect_uri: "http://localhost/cb".to_string(),
            scope: Some("openid".to_string()),
            state: None,
            code_challenge: None,
            code_challenge_method: None,
            nonce: None,
            external_subject_platform: None,
            external_subject_tenant: None,
            external_subject_external_user_id: None,
            binding_grant_id: None,
            prompt: None,
            request_uri: None,
            resource: vec![],
        };
        let hints = AppDefaultServiceHints {
            preselect_service_ids: vec!["svc-1".to_string(), "svc-2".to_string()],
            unmatched_names: vec!["Lark Bot".to_string()],
            ..AppDefaultServiceHints::default()
        };
        let url = build_consent_url(
            "https://app.example",
            &params,
            "Hint App",
            "openid",
            None,
            &hints,
        );
        assert!(url.contains("preselect_service_ids=svc-1"));
        assert!(url.contains("preselect_service_ids=svc-2"));
        assert!(url.contains("unmatched_defaults=Lark%20Bot"));

        let none = build_consent_url(
            "https://app.example",
            &params,
            "Hint App",
            "openid",
            None,
            &AppDefaultServiceHints::default(),
        );
        assert!(!none.contains("preselect_service_ids"));
        assert!(!none.contains("unmatched_defaults"));
    }
}
