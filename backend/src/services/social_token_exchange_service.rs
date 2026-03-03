use base64::Engine as _;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwks::JwksCache;
use crate::crypto::jwt::{self, JwtKeys};
use crate::errors::{AppError, AppResult};
use crate::services::{
    audit_service, oauth_service, social_auth_service,
    social_auth_service::{SocialProfile, SocialProvider},
    token_service,
};

/// Response from a successful social token exchange.
pub struct SocialTokenExchangeResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub expires_in: i64,
    pub scope: String,
}

/// Exchange an external provider token (Google ID token or GitHub access token)
/// for a full NyxID token set.
///
/// Wraps the core logic to ensure both success and failure are audit-logged.
#[allow(clippy::too_many_arguments)]
pub async fn exchange_social_token(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    jwks_cache: &JwksCache,
    http_client: &reqwest::Client,
    client_id: &str,
    client_secret: Option<&str>,
    subject_token: &str,
    subject_token_type: &str,
    provider: &str,
) -> AppResult<SocialTokenExchangeResponse> {
    let result = exchange_social_token_inner(
        db,
        config,
        jwt_keys,
        jwks_cache,
        http_client,
        client_id,
        client_secret,
        subject_token,
        subject_token_type,
        provider,
    )
    .await;

    match &result {
        Ok(resp) => {
            audit_service::log_async(
                db.clone(),
                Some(resp.user_id_for_audit.clone()),
                "social_token_exchange".to_string(),
                Some(serde_json::json!({
                    "provider": provider,
                    "subject_token_type": subject_token_type,
                    "client_id": client_id,
                    "result": "success",
                })),
                None,
                None,
            );
        }
        Err(err) => {
            audit_service::log_async(
                db.clone(),
                None,
                "social_token_exchange".to_string(),
                Some(serde_json::json!({
                    "provider": provider,
                    "subject_token_type": subject_token_type,
                    "client_id": client_id,
                    "result": "failure",
                    "error": err.to_string(),
                })),
                None,
                None,
            );
        }
    }

    // Strip the internal-only user_id before returning
    result.map(|r| SocialTokenExchangeResponse {
        access_token: r.access_token,
        refresh_token: r.refresh_token,
        id_token: r.id_token,
        expires_in: r.expires_in,
        scope: r.scope,
    })
}

/// Internal response that carries user_id for audit logging purposes.
struct SocialTokenExchangeInner {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    expires_in: i64,
    scope: String,
    user_id_for_audit: String,
}

/// Core exchange logic. Separated to allow the outer function to audit-log
/// both success and failure.
///
/// Flow:
/// 1. Authenticate the NyxID OAuth client
/// 2. Verify the external provider token
/// 3. Find or create the user via social auth service
/// 4. Issue full NyxID tokens (access + refresh + ID token)
#[allow(clippy::too_many_arguments)]
async fn exchange_social_token_inner(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    jwks_cache: &JwksCache,
    http_client: &reqwest::Client,
    client_id: &str,
    client_secret: Option<&str>,
    subject_token: &str,
    subject_token_type: &str,
    provider: &str,
) -> AppResult<SocialTokenExchangeInner> {
    // Step 1: Authenticate the requesting OAuth client
    let _client = oauth_service::authenticate_client(db, client_id, client_secret).await?;

    // Step 2: Parse provider
    let social_provider = SocialProvider::parse(provider).ok_or_else(|| {
        AppError::ExternalProviderNotConfigured("Unsupported or unconfigured provider".to_string())
    })?;

    validate_subject_token_type(social_provider, subject_token_type)?;

    // Step 3: Verify external token and build SocialProfile
    let profile = match social_provider {
        SocialProvider::Google => verify_google_token(jwks_cache, config, subject_token).await?,
        SocialProvider::GitHub => {
            verify_github_token_bound_to_app(config, http_client, subject_token).await?;
            social_auth_service::fetch_user_profile(
                SocialProvider::GitHub,
                subject_token,
                http_client,
            )
            .await?
        }
    };

    // Step 4: Find or create user
    let user = social_auth_service::find_or_create_user(db, &profile).await?;

    // Step 5: Issue full NyxID token set
    let tokens = token_service::create_session_and_issue_tokens(
        db, config, jwt_keys, &user.id, None, // no IP from token exchange
        None, // no user agent
    )
    .await?;

    // Step 6: Generate ID token
    let user_uuid = Uuid::parse_str(&user.id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let id_token = jwt::generate_id_token(
        jwt_keys,
        config,
        &user_uuid,
        Some(&profile.email),
        Some(true),
        profile.display_name.as_deref(),
        profile.avatar_url.as_deref(),
        client_id,
        None, // no nonce for token exchange
        Some(&tokens.access_token),
        None, // no auth context
    )?;

    Ok(SocialTokenExchangeInner {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: Some(id_token),
        expires_in: tokens.access_expires_in,
        scope: "openid profile email".to_string(),
        user_id_for_audit: user.id,
    })
}

/// Verify a Google ID token via JWKS and build a SocialProfile.
async fn verify_google_token(
    jwks_cache: &JwksCache,
    config: &AppConfig,
    token: &str,
) -> AppResult<SocialProfile> {
    let google_client_id = config.google_client_id.as_deref().ok_or_else(|| {
        AppError::ExternalProviderNotConfigured(
            "Google provider not configured (missing GOOGLE_CLIENT_ID)".to_string(),
        )
    })?;

    let claims = jwks_cache
        .verify_google_id_token(token, google_client_id)
        .await?;

    // Require verified email
    if claims.email_verified != Some(true) {
        return Err(AppError::ExternalTokenInvalid(
            "Email not verified by Google".to_string(),
        ));
    }

    let email = claims
        .email
        .ok_or_else(|| AppError::ExternalTokenInvalid("No email in Google ID token".to_string()))?;

    Ok(SocialProfile {
        provider: SocialProvider::Google,
        provider_id: claims.sub,
        email,
        display_name: claims.name,
        avatar_url: claims.picture,
    })
}

const SUBJECT_TOKEN_TYPE_ID_TOKEN: &str = "urn:ietf:params:oauth:token-type:id_token";
const SUBJECT_TOKEN_TYPE_ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";

fn validate_subject_token_type(
    provider: SocialProvider,
    subject_token_type: &str,
) -> AppResult<()> {
    match (provider, subject_token_type) {
        (SocialProvider::Google, SUBJECT_TOKEN_TYPE_ID_TOKEN)
        | (SocialProvider::GitHub, SUBJECT_TOKEN_TYPE_ACCESS_TOKEN) => Ok(()),
        (SocialProvider::Google, _) => Err(AppError::BadRequest(
            "Google social exchange requires subject_token_type=urn:ietf:params:oauth:token-type:id_token".to_string(),
        )),
        (SocialProvider::GitHub, _) => Err(AppError::BadRequest(
            "GitHub social exchange requires subject_token_type=urn:ietf:params:oauth:token-type:access_token".to_string(),
        )),
    }
}

/// Verify that the provided GitHub access token belongs to NyxID's configured
/// GitHub OAuth app.
///
/// GitHub's token check endpoint validates tokens for a specific OAuth app,
/// preventing tokens issued to unrelated third-party apps from being accepted.
async fn verify_github_token_bound_to_app(
    config: &AppConfig,
    http_client: &reqwest::Client,
    access_token: &str,
) -> AppResult<()> {
    let github_client_id = config.github_client_id.as_deref().ok_or_else(|| {
        AppError::ExternalProviderNotConfigured(
            "GitHub provider not configured (missing GITHUB_CLIENT_ID)".to_string(),
        )
    })?;
    let github_client_secret = config.github_client_secret.as_deref().ok_or_else(|| {
        AppError::ExternalProviderNotConfigured(
            "GitHub provider not configured (missing GITHUB_CLIENT_SECRET)".to_string(),
        )
    })?;

    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("{github_client_id}:{github_client_secret}"));
    let check_url = format!("https://api.github.com/applications/{github_client_id}/token");

    let response = http_client
        .post(&check_url)
        .header("Authorization", format!("Basic {basic}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "NyxID")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&serde_json::json!({ "access_token": access_token }))
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "GitHub token check HTTP error");
            AppError::SocialAuthFailed("Failed to verify token with GitHub".to_string())
        })?;

    match response.status() {
        reqwest::StatusCode::OK => {
            let body: GitHubTokenCheckResponse = response.json().await.map_err(|e| {
                tracing::error!(error = %e, "GitHub token check parse error");
                AppError::SocialAuthFailed(
                    "Invalid token verification response from GitHub".to_string(),
                )
            })?;

            if body
                .app
                .and_then(|app| app.client_id)
                .is_some_and(|cid| cid != github_client_id)
            {
                return Err(AppError::ExternalTokenInvalid(
                    "GitHub access token is not issued for this application".to_string(),
                ));
            }

            Ok(())
        }
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            Err(AppError::SocialAuthFailed(
                "GitHub token verification credentials are invalid".to_string(),
            ))
        }
        reqwest::StatusCode::NOT_FOUND => Err(AppError::ExternalTokenInvalid(
            "GitHub access token is invalid for this application".to_string(),
        )),
        status if status.is_client_error() => Err(AppError::ExternalTokenInvalid(
            "GitHub access token is invalid for this application".to_string(),
        )),
        status => Err(AppError::SocialAuthFailed(format!(
            "GitHub token verification failed with status {status}"
        ))),
    }
}

#[derive(serde::Deserialize)]
struct GitHubTokenCheckResponse {
    app: Option<GitHubTokenCheckApp>,
}

#[derive(serde::Deserialize)]
struct GitHubTokenCheckApp {
    client_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_parsing_valid() {
        assert!(SocialProvider::parse("google").is_some());
        assert!(SocialProvider::parse("github").is_some());
    }

    #[test]
    fn provider_parsing_invalid() {
        assert!(SocialProvider::parse("apple").is_none());
        assert!(SocialProvider::parse("").is_none());
        assert!(SocialProvider::parse("Google").is_none());
    }

    #[test]
    fn subject_token_type_google() {
        assert!(
            validate_subject_token_type(
                SocialProvider::Google,
                "urn:ietf:params:oauth:token-type:id_token"
            )
            .is_ok()
        );
        assert!(
            validate_subject_token_type(
                SocialProvider::Google,
                "urn:ietf:params:oauth:token-type:access_token"
            )
            .is_err()
        );
    }

    #[test]
    fn subject_token_type_github() {
        assert!(
            validate_subject_token_type(
                SocialProvider::GitHub,
                "urn:ietf:params:oauth:token-type:access_token"
            )
            .is_ok()
        );
        assert!(
            validate_subject_token_type(
                SocialProvider::GitHub,
                "urn:ietf:params:oauth:token-type:id_token"
            )
            .is_err()
        );
    }
}
