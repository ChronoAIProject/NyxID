use axum::{
    Json,
    extract::{Form, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent};
use crate::models::service_account_token::{COLLECTION_NAME as SA_TOKENS, ServiceAccountToken};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{AuthUser, OptionalAuthUser};
use crate::services::{
    audit_service, consent_service, oauth_client_service, oauth_service, service_account_service,
    social_token_exchange_service, token_exchange_service,
};

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub nonce: Option<String>,
    /// OIDC prompt parameter: "none", "login", "consent", or space-separated combo.
    pub prompt: Option<String>,
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
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

    let is_authenticated = opt_auth.0.is_some();
    tracing::info!(
        client_id = %params.client_id,
        is_browser_mode,
        is_authenticated,
        redirect_uri = %params.redirect_uri,
        "OAuth authorize endpoint hit"
    );

    let result = authorize_inner(&state, opt_auth, &params, is_browser_mode).await;

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
        prompt: form.prompt,
    };

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

    let code = issue_authorization_code(&state, &auth_user, &params, &validated_scope).await?;
    let redirect_url = build_callback_url(&params, &code);

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

                let code =
                    issue_authorization_code(state, &auth_user, params, &validated_scope).await?;
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

        let code = issue_authorization_code(state, &auth_user, params, &validated_scope).await?;
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

/// Reconstruct the full authorize URL so it can be used as a `return_to` target
/// after the user logs in on the frontend.
fn build_authorize_url(base_url: &str, params: &AuthorizeQuery) -> String {
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
    )
    .await?;

    // Audit log the authorization code issuance
    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "oauth_code_issued".to_string(),
        Some(serde_json::json!({
            "client_id": params.client_id,
            "scope": validated_scope,
        })),
        None,
        None,
    );

    Ok(code)
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
    headers: HeaderMap,
    Form(body): Form<TokenRequest>,
) -> Response {
    match token_inner(&state, &headers, body).await {
        Ok(json) => json.into_response(),
        Err(err) => oauth_error_response(err),
    }
}

async fn token_inner(
    state: &AppState,
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

            let (access_token, refresh_token, id_token, granted_scope) =
                oauth_service::exchange_authorization_code(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    code,
                    client_id_str,
                    redirect_uri,
                    body.code_verifier.as_deref(),
                    body.client_secret.as_deref(),
                )
                .await?;

            Ok(Json(TokenResponse {
                access_token,
                token_type: "Bearer".to_string(),
                expires_in: state.config.jwt_access_ttl_secs,
                refresh_token: Some(refresh_token),
                id_token,
                scope: Some(granted_scope),
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
                issued_token_type: None,
            }))
        }
        // RFC 8693 Token Exchange
        "urn:ietf:params:oauth:grant-type:token-exchange" => {
            let client_id = body
                .client_id
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing client_id".to_string()))?;
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
                let result = social_token_exchange_service::exchange_social_token(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    &state.jwks_cache,
                    &state.http_client,
                    client_id,
                    body.client_secret.as_deref(),
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
                    issued_token_type: Some(
                        "urn:ietf:params:oauth:token-type:access_token".to_string(),
                    ),
                }))
            } else if subject_token_type == "urn:ietf:params:oauth:token-type:access_token" {
                // Existing: Delegated token exchange (NyxID access token -> delegated token)
                let client_secret = body
                    .client_secret
                    .as_deref()
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
                    None,
                    None,
                );

                Ok(Json(TokenResponse {
                    access_token: result.access_token,
                    token_type: result.token_type,
                    expires_in: result.expires_in,
                    refresh_token: None,
                    id_token: None,
                    scope: Some(result.scope),
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
                    );

                    Ok(Json(TokenResponse {
                        access_token: response.access_token,
                        token_type: response.token_type,
                        expires_in: response.expires_in,
                        refresh_token: None,
                        id_token: None,
                        scope: Some(response.scope),
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
        roles: claims.roles,
        groups: claims.groups,
        permissions: claims.permissions,
    })
}

/// POST /oauth/revoke
///
/// RFC 7009 Token Revocation. Authenticates the calling client before
/// revoking the token. Always returns 200 per the spec.
pub async fn revoke(State(state): State<AppState>, Form(body): Form<RevokeRequest>) -> StatusCode {
    // Authenticate the calling client (RFC 7009 requirement).
    // Per the spec, always return 200 even if authentication fails.
    let caller_client_id = match body.client_id.as_deref() {
        Some(id) if !id.is_empty() => id,
        _ => return StatusCode::OK,
    };

    if oauth_service::authenticate_client(
        &state.db,
        caller_client_id,
        body.client_secret.as_deref(),
    )
    .await
    .is_err()
    {
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
#[allow(dead_code)]
pub struct RegisterClientRequest {
    pub client_name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    pub grant_types: Option<Vec<String>>,
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
    let (client, _secret) = oauth_client_service::create_client(
        &state.db,
        &client_name,
        &redirect_uris,
        "public",
        "dynamic_registration",
        "",
        oauth_client_service::DEFAULT_ALLOWED_SCOPES,
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
            grant_types: vec!["authorization_code".to_string()],
            response_types: vec!["code".to_string()],
            token_endpoint_auth_method: "none".to_string(),
            scope: client.allowed_scopes,
            client_id_issued_at: client.created_at.timestamp(),
        }),
    ))
}
