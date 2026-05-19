use axum::{
    Json,
    extract::{Form, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent};
use crate::models::authorization_code::{ExternalSubjectRef, validate_external_subject_params};
use crate::models::service_account_token::{COLLECTION_NAME as SA_TOKENS, ServiceAccountToken};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{AuthUser, OptionalAuthUser};
use crate::services::{
    audit_service, consent_service, oauth_broker_service, oauth_client_service, oauth_service,
    par_service, service_account_service, social_token_exchange_service, token_exchange_service,
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
    /// OIDC prompt parameter: "none", "login", "consent", or space-separated combo.
    pub prompt: Option<String>,
    pub request_uri: Option<String>,
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
    pub prompt: Option<String>,
    pub decision: String,
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
    pub binding_id: Option<String>,
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
    pub external_subject_platform: Option<String>,
    pub external_subject_tenant: Option<String>,
    pub external_subject_external_user_id: Option<String>,
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
    let params = AuthorizeQuery {
        response_type: form.response_type,
        client_id: form.client_id,
        redirect_uri: form.redirect_uri,
        scope: form.scope,
        state: form.state,
        code_challenge: form.code_challenge,
        code_challenge_method: form.code_challenge_method,
        nonce: form.nonce,
        external_subject_platform: form.external_subject_platform,
        external_subject_tenant: form.external_subject_tenant,
        external_subject_external_user_id: form.external_subject_external_user_id,
        prompt: form.prompt,
        request_uri: None,
    };

    let external_subject = validate_external_subject_params(
        params.external_subject_platform.as_deref(),
        params.external_subject_tenant.as_deref(),
        params.external_subject_external_user_id.as_deref(),
    )?;

    let auth_user = match opt_auth.0 {
        Some(user) => user,
        None => {
            let return_to = build_authorize_url(&state.config.frontend_url, &params);
            let login_url = format!(
                "{}/login?return_to={}",
                state.config.frontend_url,
                urlencoding::encode(&return_to),
            );
            return Ok(redirect_302(&login_url));
        }
    };

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

    let user_id_str = auth_user.user_id.to_string();
    consent_service::grant_consent(&state.db, &user_id_str, &params.client_id, &validated_scope)
        .await?;

    let code = issue_authorization_code(
        &state,
        &auth_user,
        &params,
        &validated_scope,
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

                let has_consent = consent_service::check_consent(
                    &state.db,
                    &user_id_str,
                    &params.client_id,
                    &validated_scope,
                )
                .await?
                .is_some();

                let needs_consent = !has_consent || force_consent;

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

                    let consent_url = build_consent_url(
                        &state.config.frontend_url,
                        params,
                        &client.client_name,
                        &validated_scope,
                    );
                    return Ok(redirect_302(&consent_url));
                }

                let code = issue_authorization_code(
                    state,
                    &auth_user,
                    params,
                    &validated_scope,
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

        let has_consent = consent_service::check_consent(
            &state.db,
            &user_id_str,
            &params.client_id,
            &validated_scope,
        )
        .await?
        .is_some();

        if !has_consent || force_consent {
            let consent_url = build_consent_url(
                &state.config.frontend_url,
                params,
                &client.client_name,
                &validated_scope,
            );
            return Err(AppError::ConsentRequired { consent_url });
        }

        let code = issue_authorization_code(
            state,
            &auth_user,
            params,
            &validated_scope,
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
        || params.prompt.is_some()
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
        prompt: record.prompt,
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
    if let Some(ref prompt) = params.prompt {
        url.push_str(&format!("&prompt={}", urlencoding::encode(prompt)));
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

fn build_consent_url(
    frontend_url: &str,
    params: &AuthorizeQuery,
    client_name: &str,
    validated_scope: &str,
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
    if let Some(ref prompt) = params.prompt {
        url.push_str(&format!("&prompt={}", urlencoding::encode(prompt)));
    }

    url
}

/// Create an authorization code for the given user and OAuth parameters.
async fn issue_authorization_code(
    state: &AppState,
    auth_user: &crate::mw::auth::AuthUser,
    params: &AuthorizeQuery,
    validated_scope: &str,
    external_subject: Option<&ExternalSubjectRef>,
) -> AppResult<String> {
    let user_id_str = auth_user.user_id.to_string();
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
    )
    .await?;

    let mut event_data = serde_json::json!({
        "client_id": params.client_id,
        "scope": validated_scope,
    });
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
        external_subject,
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

            let exchanged = oauth_service::exchange_authorization_code(
                &state.db,
                &state.config,
                &state.jwt_keys,
                code,
                client_id_str,
                redirect_uri,
                body.code_verifier.as_deref(),
                body.client_secret.as_deref(),
                Some(oauth_broker_service::BROKER_ACCESS_TTL_SECS),
            )
            .await?;

            if exchanged.broker_capability_enabled {
                let granted_scopes: Vec<String> = exchanged
                    .granted_scope
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                let (binding_id, _binding_hash) = oauth_broker_service::create_binding(
                    &state.db,
                    &state.encryption_keys,
                    client_id_str,
                    &exchanged.user_id,
                    &exchanged.refresh_token,
                    &exchanged.refresh_token_jti,
                    &granted_scopes,
                    exchanged.external_subject.as_ref(),
                )
                .await?;

                let binding_hash =
                    crate::models::oauth_broker_binding::hash_binding_id(&binding_id);
                audit_service::log_async(
                    state.db.clone(),
                    Some(exchanged.user_id.clone()),
                    "oauth_broker_binding_issued".to_string(),
                    Some(serde_json::json!({
                        "client_id": client_id_str,
                        "binding_hash": oauth_broker_service::binding_hash_prefix(&binding_hash),
                        "scope": &exchanged.granted_scope,
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
                    token_type: "Bearer".to_string(),
                    expires_in: oauth_broker_service::BROKER_ACCESS_TTL_SECS,
                    refresh_token: None,
                    id_token: exchanged.id_token,
                    scope: Some(exchanged.granted_scope),
                    binding_id: Some(binding_id),
                    issued_token_type: None,
                }));
            }

            Ok(Json(TokenResponse {
                access_token: exchanged.access_token,
                token_type: "Bearer".to_string(),
                expires_in: state.config.jwt_access_ttl_secs,
                refresh_token: Some(exchanged.refresh_token),
                id_token: exchanged.id_token,
                scope: Some(exchanged.granted_scope),
                binding_id: None,
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
            )
            .await?;

            Ok(Json(TokenResponse {
                access_token: tokens.access_token,
                token_type: "Bearer".to_string(),
                expires_in: tokens.access_expires_in,
                refresh_token: Some(tokens.refresh_token),
                id_token: None,
                scope: None,
                binding_id: None,
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
                    binding_id: None,
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
                    binding_id: None,
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
                if !oauth_broker_service::is_broker_client(&client) {
                    return Err(AppError::ExternalTokenInvalid("invalid_grant".to_string()));
                }

                let dpop_jkt = match headers.get("dpop") {
                    Some(value) => {
                        let proof = value.to_str().map_err(|_| {
                            AppError::Unauthorized("invalid DPoP proof".to_string())
                        })?;
                        let htu = crate::crypto::dpop::htu_from_base_and_path(
                            &state.config.base_url,
                            "/oauth/token",
                        )?;
                        Some(crate::crypto::dpop::validate_proof(
                            proof,
                            "POST",
                            &htu,
                            &state.dpop_jti_cache,
                        )?)
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
                                AppError::Unauthorized(
                                    "invalid mTLS client certificate header".to_string(),
                                )
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

                let result = oauth_broker_service::exchange_via_binding(
                    &state.db,
                    state.encryption_keys.clone(),
                    &state.http_client,
                    &state.jwt_keys,
                    &state.config,
                    client_id,
                    subject_token,
                    body.scope.as_deref(),
                    dpop_jkt.as_deref(),
                    mtls_x5t_s256.as_deref(),
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
                        "dpop_jkt": dpop_jkt
                            .as_deref()
                            .map(|jkt| jkt.chars().take(16).collect::<String>()),
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
                    binding_id: None,
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
                        binding_id: None,
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
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    // Check scopes from the access token claims
    let scopes: Vec<&str> = auth_user.scope.split_whitespace().collect();
    let include_roles = scopes.contains(&"roles");
    let include_groups = scopes.contains(&"groups");

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
    axum::extract::Query(query): axum::extract::Query<ListBindingsByExternalSubjectQuery>,
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
    axum::extract::Query(query): axum::extract::Query<GetBindingQuery>,
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
    axum::extract::Query(query): axum::extract::Query<GetBindingQuery>,
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
mod tests {
    use super::*;
    use axum::extract::Path;
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;
    use uuid::Uuid;

    use crate::crypto::jwt;
    use crate::models::oauth_broker_binding::{
        COLLECTION_NAME as OAUTH_BROKER_BINDINGS, OauthBrokerBinding, hash_binding_id,
    };
    use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
    use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};
    use crate::services::oauth_broker_service::BROKER_BINDING_SCOPE;
    use crate::test_utils::{connect_test_database, test_app_state};

    async fn insert_public_client(db: &mongodb::Database, client_id: &str, allowed_scopes: &str) {
        let now = Utc::now();
        let client = OauthClient {
            id: client_id.to_string(),
            client_name: "Public Broker Test Client".to_string(),
            client_secret_hash: String::new(),
            redirect_uris: vec!["http://localhost/callback".to_string()],
            allowed_scopes: allowed_scopes.to_string(),
            grant_types: "authorization_code refresh_token".to_string(),
            client_type: "public".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
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
            expires_at: now + Duration::days(7),
            revoked: false,
            replaced_by: None,
            revoked_at: None,
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
        assert!(oauth_broker_service::is_broker_client(&client));
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
}
