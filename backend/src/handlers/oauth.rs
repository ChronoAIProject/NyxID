use axum::{
    extract::{Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::mw::auth::{AuthUser, OptionalAuthUser};
use crate::services::{audit_service, oauth_service};
use crate::AppState;

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
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UserinfoResponse {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
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
    Query(params): Query<AuthorizeQuery>,
) -> Result<Response, AppError> {
    let is_browser_mode = !accepts_json(&headers);
    let result = authorize_inner(&state, opt_auth, &params, is_browser_mode).await;

    match result {
        Ok(response) => Ok(response),
        Err(err) if is_browser_mode => {
            // In browser mode, redirect to frontend error page for better UX.
            // Per RFC 6749 s4.1.2.1, we must NOT redirect to the client's URI
            // when client_id/redirect_uri validation fails, but redirecting to
            // our own frontend error page is safe.
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

async fn authorize_inner(
    state: &AppState,
    opt_auth: OptionalAuthUser,
    params: &AuthorizeQuery,
    is_browser_mode: bool,
) -> Result<Response, AppError> {
    // --- Common validation (both modes) ---

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

    // Require explicit code_challenge_method (RFC 7636 s4.3 default is plain,
    // but we only support S256 -- reject ambiguity by requiring the parameter).
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

    // Validate client_id and redirect_uri BEFORE any redirects.
    // If these are invalid we must NOT redirect (could be an attacker's URI).
    let client =
        oauth_service::validate_client(&state.db, &params.client_id, &params.redirect_uri)
            .await?;

    let scope = params.scope.as_deref().unwrap_or("openid profile email");
    let validated_scope = oauth_service::validate_scopes(scope, &client.allowed_scopes)?;

    if is_browser_mode {
        match opt_auth.0 {
            None => {
                // Redirect to frontend login with return_to pointing back here
                let return_to = build_authorize_url(&state.config.base_url, params);
                let login_url = format!(
                    "{}/login?return_to={}",
                    state.config.frontend_url,
                    urlencoding::encode(&return_to),
                );
                Ok(redirect_302(&login_url))
            }
            Some(auth_user) => {
                // TODO: When third-party clients are supported, display a consent
                // screen here. Currently only first-party clients exist, so
                // auto-granting is acceptable. Implement per-user consent storage
                // keyed by (user_id, client_id, scope) when adding third-party support.
                let code =
                    issue_authorization_code(state, &auth_user, params, &validated_scope)
                        .await?;
                let redirect_url = build_callback_url(params, &code);
                Ok(redirect_302(&redirect_url))
            }
        }
    } else {
        // API mode: require authentication (existing behavior)
        let auth_user = opt_auth.0.ok_or_else(|| {
            AppError::Unauthorized("Authentication required".to_string())
        })?;

        let code =
            issue_authorization_code(state, &auth_user, params, &validated_scope).await?;
        let redirect_url = build_callback_url(params, &code);
        Ok(Json(AuthorizeResponse { redirect_url }).into_response())
    }
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

    url
}

/// Build the callback redirect URL with code and optional state.
fn build_callback_url(params: &AuthorizeQuery, code: &str) -> String {
    let mut url = format!(
        "{}?code={}",
        params.redirect_uri,
        urlencoding::encode(code),
    );
    if let Some(ref state_param) = params.state {
        url.push_str(&format!("&state={}", urlencoding::encode(state_param)));
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
/// OAuth 2.0 Token Endpoint. Exchanges an authorization code for tokens.
/// Validates client_secret for confidential clients.
pub async fn token(
    State(state): State<AppState>,
    Form(body): Form<TokenRequest>,
) -> AppResult<Json<TokenResponse>> {
    match body.grant_type.as_str() {
        "authorization_code" => {
            let code = body
                .code
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing code parameter".to_string()))?;

            let redirect_uri = body
                .redirect_uri
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing redirect_uri parameter".to_string()))?;

            let client_id_str = body
                .client_id
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing client_id parameter".to_string()))?;

            let (access_token, refresh_token, id_token) =
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
                scope: Some("openid profile email".to_string()),
            }))
        }
        "refresh_token" => {
            let refresh = body
                .refresh_token
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing refresh_token parameter".to_string()))?;

            let tokens = crate::services::token_service::refresh_tokens(
                &state.db,
                &state.config,
                &state.jwt_keys,
                refresh,
            )
            .await?;

            Ok(Json(TokenResponse {
                access_token: tokens.access_token,
                token_type: "Bearer".to_string(),
                expires_in: tokens.access_expires_in,
                refresh_token: Some(tokens.refresh_token),
                id_token: None,
                scope: None,
            }))
        }
        other => Err(AppError::BadRequest(format!(
            "Unsupported grant_type: {other}"
        ))),
    }
}

/// GET /oauth/userinfo
///
/// OpenID Connect UserInfo Endpoint. Returns claims about the authenticated user.
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

    Ok(Json(UserinfoResponse {
        sub: user.id.to_string(),
        email: Some(user.email),
        email_verified: Some(user.email_verified),
        name: user.display_name,
        picture: user.avatar_url,
    }))
}
