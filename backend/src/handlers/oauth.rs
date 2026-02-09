use axum::{
    extract::{Query, State},
    Json,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::mw::auth::AuthUser;
use crate::services::oauth_service;
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
/// OAuth 2.0 Authorization Endpoint. Validates the client and parameters,
/// then (after user consent) issues an authorization code.
///
/// Requires PKCE (code_challenge) for all requests. Only S256 method is supported.
pub async fn authorize(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<AuthorizeQuery>,
) -> AppResult<Json<AuthorizeResponse>> {
    if params.response_type != "code" {
        return Err(AppError::BadRequest(
            "Only response_type=code is supported".to_string(),
        ));
    }

    // Require PKCE for all clients
    if params.code_challenge.is_none() {
        return Err(AppError::BadRequest(
            "code_challenge is required (PKCE)".to_string(),
        ));
    }

    // Only allow S256 method
    if let Some(ref method) = params.code_challenge_method {
        if method != "S256" {
            return Err(AppError::BadRequest(
                "Only S256 code_challenge_method is supported".to_string(),
            ));
        }
    }

    let client = oauth_service::validate_client(&state.db, &params.client_id, &params.redirect_uri).await?;

    let scope = params.scope.as_deref().unwrap_or("openid profile email");
    let validated_scope = oauth_service::validate_scopes(scope, &client.allowed_scopes)?;

    let user_id_str = auth_user.user_id.to_string();
    let code = oauth_service::create_authorization_code(
        &state.db,
        &params.client_id,
        &user_id_str,
        &params.redirect_uri,
        &validated_scope,
        params.code_challenge.as_deref(),
        params.code_challenge_method.as_deref().or(Some("S256")),
        params.nonce.as_deref(),
    )
    .await?;

    // Build redirect URL with properly URL-encoded parameters
    let mut redirect_url = format!(
        "{}?code={}",
        params.redirect_uri,
        urlencoding::encode(&code),
    );

    if let Some(state_param) = &params.state {
        redirect_url.push_str(&format!("&state={}", urlencoding::encode(state_param)));
    }

    Ok(Json(AuthorizeResponse { redirect_url }))
}

/// POST /oauth/token
///
/// OAuth 2.0 Token Endpoint. Exchanges an authorization code for tokens.
/// Validates client_secret for confidential clients.
pub async fn token(
    State(state): State<AppState>,
    Json(body): Json<TokenRequest>,
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
