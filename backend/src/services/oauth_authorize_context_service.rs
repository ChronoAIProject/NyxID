//! A five-minute login handoff holds validated authorize parameters in MongoDB.
//! The browser sees a signed context reference; it cannot replace the request.

use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, Header, Validation, decode, encode};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::models::app_connect_link::ValidatedAuthorizeParams;
use crate::models::oauth_authorize_context::{COLLECTION_NAME, OauthAuthorizeContext};
use crate::models::oauth_client::OauthClient;
use crate::{
    AppState,
    errors::{AppError, AppResult},
};

const AUDIENCE: &str = "nyxid/oauth-authorize-context";
const TOKEN_TYPE: &str = "oauth_authorize_context";

#[derive(Serialize, Deserialize)]
struct ContextClaims {
    sub: String,
    iss: String,
    aud: String,
    exp: i64,
    iat: i64,
    token_type: String,
}

pub async fn mint(state: &AppState, params: ValidatedAuthorizeParams) -> AppResult<String> {
    let now = Utc::now();
    let record = OauthAuthorizeContext {
        id: uuid::Uuid::new_v4().to_string(),
        authorize_params: params,
        created_at: now,
        expires_at: now + Duration::minutes(5),
    };
    let claims = ContextClaims {
        sub: record.id.clone(),
        iss: state.config.jwt_issuer.clone(),
        aud: AUDIENCE.into(),
        exp: record.expires_at.timestamp(),
        iat: now.timestamp(),
        token_type: TOKEN_TYPE.into(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(state.jwt_keys.kid.clone());
    let token = encode(&header, &claims, &state.jwt_keys.encoding)
        .map_err(|_| AppError::Internal("Could not sign authorize context".into()))?;
    state
        .db
        .collection::<OauthAuthorizeContext>(COLLECTION_NAME)
        .insert_one(record)
        .await?;
    Ok(token)
}

pub async fn load(
    state: &AppState,
    token: &str,
) -> AppResult<(OauthAuthorizeContext, OauthClient)> {
    let not_found = || AppError::NotFound("Authorization context not found".into());
    if token.len() > 4096 {
        return Err(not_found());
    }
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[AUDIENCE]);
    validation.set_issuer(&[&state.config.jwt_issuer]);
    validation.leeway = 0;
    let claims = decode::<ContextClaims>(token, &state.jwt_keys.decoding, &validation)
        .map_err(|_| not_found())?
        .claims;
    if claims.token_type != TOKEN_TYPE
        || claims.iat > Utc::now().timestamp()
        || claims.exp - claims.iat != 300
    {
        return Err(not_found());
    }
    let record = state
        .db
        .collection::<OauthAuthorizeContext>(COLLECTION_NAME)
        .find_one(doc! { "_id": &claims.sub, "expires_at": { "$gt": bson::DateTime::now() } })
        .await?
        .ok_or_else(not_found)?;
    let client =
        super::oauth_client_service::get_client(&state.db, &record.authorize_params.client_id)
            .await
            .map_err(|error| match error {
                AppError::NotFound(_) => not_found(),
                other => other,
            })?;
    if super::app_connect_authorize_service::gate_manifest(state, &client)
        .await?
        .is_none()
    {
        return Err(not_found());
    }
    super::oauth_service::validate_client(
        &state.db,
        &client.id,
        &record.authorize_params.redirect_uri,
    )
    .await
    .map_err(|error| match error {
        AppError::DatabaseError(_) | AppError::Internal(_) => error,
        _ => not_found(),
    })?;
    Ok((record, client))
}

/// A forced reauthentication can only resume with a session created after the
/// handoff. Visiting the resume URL with the old cookie cannot clear prompt=login.
pub async fn login_completed(
    db: &mongodb::Database,
    context: &OauthAuthorizeContext,
    user_id: &str,
    session_id: Option<&str>,
) -> AppResult<bool> {
    if !context
        .authorize_params
        .prompt
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .any(|p| p == "login")
    {
        return Ok(true);
    }
    let Some(session_id) = session_id else {
        return Ok(false);
    };
    Ok(db
        .collection::<crate::models::session::Session>(crate::models::session::COLLECTION_NAME)
        .find_one(
            doc! { "_id": session_id, "user_id": user_id, "revoked": false,
            "expires_at": { "$gt": bson::DateTime::now() },
            "created_at": { "$gte": bson::DateTime::from_chrono(context.created_at) } },
        )
        .await?
        .is_some())
}
