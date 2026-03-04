use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::Deserialize;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};

/// Supported social login providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocialProvider {
    GitHub,
    Google,
    Apple,
}

impl SocialProvider {
    /// Parse from URL path segment. Returns None for unsupported providers.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Self::GitHub),
            "google" => Some(Self::Google),
            "apple" => Some(Self::Apple),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Google => "google",
            Self::Apple => "apple",
        }
    }
}

/// Normalized user profile from a social provider.
pub struct SocialProfile {
    pub provider: SocialProvider,
    pub provider_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

/// Build the OAuth authorization URL for the given provider.
pub fn build_authorization_url(
    provider: SocialProvider,
    state: &str,
    nonce: Option<&str>,
    config: &AppConfig,
) -> AppResult<String> {
    let base_url = config.base_url.trim_end_matches('/');

    match provider {
        SocialProvider::GitHub => {
            let client_id = config.github_client_id.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("GitHub provider not configured".to_string())
            })?;
            // Verify secret is also configured
            config.github_client_secret.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("GitHub provider not configured".to_string())
            })?;

            let raw_redirect = format!("{base_url}/api/v1/auth/social/github/callback");
            let redirect_uri = urlencoding::encode(&raw_redirect);

            Ok(format!(
                "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope=user:email&state={}",
                client_id, redirect_uri, state,
            ))
        }
        SocialProvider::Google => {
            let client_id = config.google_client_id.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("Google provider not configured".to_string())
            })?;
            config.google_client_secret.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("Google provider not configured".to_string())
            })?;

            let raw_redirect = format!("{base_url}/api/v1/auth/social/google/callback");
            let redirect_uri = urlencoding::encode(&raw_redirect);
            let scope = urlencoding::encode("openid email profile");

            Ok(format!(
                "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&scope={}&state={}&response_type=code&access_type=online",
                client_id, redirect_uri, scope, state,
            ))
        }
        SocialProvider::Apple => {
            let client_id = config.apple_client_id.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("Apple provider not configured".to_string())
            })?;
            if !config.apple_configured() {
                return Err(AppError::SocialAuthFailed(
                    "Apple provider not fully configured".to_string(),
                ));
            }
            let nonce = nonce.filter(|n| !n.is_empty()).ok_or_else(|| {
                AppError::SocialAuthFailed("Apple authorization requires nonce".to_string())
            })?;

            let raw_redirect = format!("{base_url}/api/v1/auth/social/apple/callback");
            let redirect_uri = urlencoding::encode(&raw_redirect);
            let scope = urlencoding::encode("name email");
            let nonce = urlencoding::encode(nonce);

            Ok(format!(
                "https://appleid.apple.com/auth/authorize?client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&response_type=code&response_mode=form_post",
                client_id, redirect_uri, scope, state, nonce,
            ))
        }
    }
}

// --- Token exchange response types ---

#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct AppleTokenResponse {
    #[allow(dead_code)]
    access_token: Option<String>,
    id_token: Option<String>,
    error: Option<String>,
}

/// Exchange an authorization code for an access token.
pub async fn exchange_code(
    provider: SocialProvider,
    code: &str,
    config: &AppConfig,
    http_client: &reqwest::Client,
) -> AppResult<String> {
    let base_url = config.base_url.trim_end_matches('/');

    match provider {
        SocialProvider::GitHub => {
            let client_id = config.github_client_id.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("GitHub provider not configured".to_string())
            })?;
            let client_secret = config.github_client_secret.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("GitHub provider not configured".to_string())
            })?;
            let redirect_uri = format!("{base_url}/api/v1/auth/social/github/callback");

            let resp = http_client
                .post("https://github.com/login/oauth/access_token")
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", client_id),
                    ("client_secret", client_secret),
                    ("code", code),
                    ("redirect_uri", &redirect_uri),
                ])
                .send()
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "GitHub token exchange HTTP error");
                    AppError::SocialAuthFailed("Failed to exchange code with GitHub".to_string())
                })?;

            let body: GitHubTokenResponse = resp.json().await.map_err(|e| {
                tracing::error!(error = %e, "GitHub token response parse error");
                AppError::SocialAuthFailed("Failed to exchange code with GitHub".to_string())
            })?;

            if let Some(err) = body.error {
                tracing::debug!(
                    provider = "github",
                    error = %err,
                    description = ?body.error_description,
                    "Provider token exchange error"
                );
                return Err(AppError::SocialAuthFailed(
                    "Failed to exchange code with GitHub".to_string(),
                ));
            }

            body.access_token.ok_or_else(|| {
                AppError::SocialAuthFailed("No access token in GitHub response".to_string())
            })
        }
        SocialProvider::Google => {
            let client_id = config.google_client_id.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("Google provider not configured".to_string())
            })?;
            let client_secret = config.google_client_secret.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("Google provider not configured".to_string())
            })?;
            let redirect_uri = format!("{base_url}/api/v1/auth/social/google/callback");

            let resp = http_client
                .post("https://oauth2.googleapis.com/token")
                .form(&[
                    ("client_id", client_id),
                    ("client_secret", client_secret),
                    ("code", code),
                    ("redirect_uri", &redirect_uri),
                    ("grant_type", "authorization_code"),
                ])
                .send()
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "Google token exchange HTTP error");
                    AppError::SocialAuthFailed("Failed to exchange code with Google".to_string())
                })?;

            let body: GoogleTokenResponse = resp.json().await.map_err(|e| {
                tracing::error!(error = %e, "Google token response parse error");
                AppError::SocialAuthFailed("Failed to exchange code with Google".to_string())
            })?;

            if let Some(err) = body.error {
                tracing::debug!(
                    provider = "google",
                    error = %err,
                    description = ?body.error_description,
                    "Provider token exchange error"
                );
                return Err(AppError::SocialAuthFailed(
                    "Failed to exchange code with Google".to_string(),
                ));
            }

            body.access_token.ok_or_else(|| {
                AppError::SocialAuthFailed("No access token in Google response".to_string())
            })
        }
        SocialProvider::Apple => {
            let client_id = config.apple_client_id.as_deref().ok_or_else(|| {
                AppError::SocialAuthFailed("Apple provider not configured".to_string())
            })?;

            // Generate ephemeral client_secret JWT
            let client_secret =
                crate::crypto::apple_client_secret::generate_apple_client_secret(config)?;

            let redirect_uri = format!("{base_url}/api/v1/auth/social/apple/callback");

            let resp = http_client
                .post("https://appleid.apple.com/auth/token")
                .form(&[
                    ("client_id", client_id),
                    ("client_secret", client_secret.as_str()),
                    ("code", code),
                    ("redirect_uri", &redirect_uri),
                    ("grant_type", "authorization_code"),
                ])
                .send()
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "Apple token exchange HTTP error");
                    AppError::SocialAuthFailed("Failed to exchange code with Apple".to_string())
                })?;

            let body: AppleTokenResponse = resp.json().await.map_err(|e| {
                tracing::error!(error = %e, "Apple token response parse error");
                AppError::SocialAuthFailed("Failed to exchange code with Apple".to_string())
            })?;

            if let Some(err) = body.error {
                tracing::debug!(provider = "apple", error = %err, "Apple token exchange error");
                return Err(AppError::SocialAuthFailed(
                    "Failed to exchange code with Apple".to_string(),
                ));
            }

            // Apple returns an id_token -- the caller verifies it via JWKS
            // and extracts profile from claims.
            body.id_token.ok_or_else(|| {
                AppError::SocialAuthFailed("No id_token in Apple response".to_string())
            })
        }
    }
}

// --- User profile response types ---

#[derive(Deserialize)]
struct GitHubUser {
    id: u64,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

#[derive(Deserialize)]
struct GoogleUserInfo {
    sub: String,
    email: String,
    email_verified: Option<bool>,
    name: Option<String>,
    picture: Option<String>,
}

/// Fetch the user profile from the social provider using the access token.
pub async fn fetch_user_profile(
    provider: SocialProvider,
    access_token: &str,
    http_client: &reqwest::Client,
) -> AppResult<SocialProfile> {
    match provider {
        SocialProvider::GitHub => fetch_github_profile(access_token, http_client).await,
        SocialProvider::Google => fetch_google_profile(access_token, http_client).await,
        SocialProvider::Apple => Err(AppError::SocialAuthFailed(
            "Apple does not support userinfo endpoint. Use ID token flow.".to_string(),
        )),
    }
}

/// Build a SocialProfile from a verified Apple ID token.
///
/// Apple does not have a userinfo endpoint -- profile comes from the ID token itself.
/// Note: display_name is NOT available from the ID token. Apple only sends the name
/// in the initial POST body (handled in the callback handler).
pub fn profile_from_apple_id_token(
    claims: &crate::crypto::jwks::AppleIdTokenClaims,
) -> AppResult<SocialProfile> {
    let email = claims.email.clone().ok_or(AppError::SocialAuthNoEmail)?;

    // Accept the email even if email_verified is not explicitly true.
    // Apple private relay emails are always verified by Apple.
    // However, reject if explicitly unverified.
    if claims.is_email_verified() == Some(false) {
        return Err(AppError::SocialAuthNoEmail);
    }

    Ok(SocialProfile {
        provider: SocialProvider::Apple,
        provider_id: claims.sub.clone(),
        email,
        display_name: None, // Apple only sends name in first auth POST body
        avatar_url: None,   // Apple never provides avatars
    })
}

async fn fetch_github_profile(
    access_token: &str,
    http_client: &reqwest::Client,
) -> AppResult<SocialProfile> {
    let user: GitHubUser = http_client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "NyxID")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "GitHub user API HTTP error");
            AppError::SocialAuthFailed("Failed to fetch profile from GitHub".to_string())
        })?
        .json()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "GitHub user API parse error");
            AppError::SocialAuthFailed("Invalid profile response from GitHub".to_string())
        })?;

    // Always use /user/emails to get a verified email. The /user endpoint
    // email may not carry explicit verification status. The /user/emails
    // endpoint returns verification flags so we can guarantee a verified
    // address.
    let email = fetch_github_primary_email(access_token, http_client).await?;

    Ok(SocialProfile {
        provider: SocialProvider::GitHub,
        provider_id: user.id.to_string(),
        email,
        display_name: user.name.or(Some(user.login)),
        avatar_url: user.avatar_url,
    })
}

async fn fetch_github_primary_email(
    access_token: &str,
    http_client: &reqwest::Client,
) -> AppResult<String> {
    let emails: Vec<GitHubEmail> = http_client
        .get("https://api.github.com/user/emails")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "NyxID")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "GitHub emails API HTTP error");
            AppError::SocialAuthFailed("Failed to fetch emails from GitHub".to_string())
        })?
        .json()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "GitHub emails API parse error");
            AppError::SocialAuthFailed("Invalid email response from GitHub".to_string())
        })?;

    // Prefer primary + verified, then any verified
    if let Some(primary) = emails.iter().find(|e| e.primary && e.verified) {
        return Ok(primary.email.clone());
    }
    if let Some(verified) = emails.iter().find(|e| e.verified) {
        return Ok(verified.email.clone());
    }

    Err(AppError::SocialAuthNoEmail)
}

async fn fetch_google_profile(
    access_token: &str,
    http_client: &reqwest::Client,
) -> AppResult<SocialProfile> {
    let info: GoogleUserInfo = http_client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Google userinfo API HTTP error");
            AppError::SocialAuthFailed("Failed to fetch profile from Google".to_string())
        })?
        .json()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Google userinfo API parse error");
            AppError::SocialAuthFailed("Invalid profile response from Google".to_string())
        })?;

    if info.email_verified == Some(false) {
        return Err(AppError::SocialAuthNoEmail);
    }

    Ok(SocialProfile {
        provider: SocialProvider::Google,
        provider_id: info.sub,
        email: info.email,
        display_name: info.name,
        avatar_url: info.picture,
    })
}

/// Find an existing user by social identity or email, or create a new one.
///
/// NOTE: The returned `User` struct reflects the state *before* the update.
/// Only `user.id` should be relied upon from the return value for downstream
/// operations (e.g. session creation). Profile fields may be stale.
pub async fn find_or_create_user(
    db: &mongodb::Database,
    profile: &SocialProfile,
) -> AppResult<User> {
    let users = db.collection::<User>(USERS);

    // Case 1: Returning social user (same provider + provider_id)
    let existing_social = users
        .find_one(doc! {
            "social_provider": profile.provider.as_str(),
            "social_provider_id": &profile.provider_id,
        })
        .await?;

    if let Some(user) = existing_social {
        if !user.is_active {
            return Err(AppError::SocialAuthDeactivated);
        }

        let now = Utc::now();
        let mut update = doc! {
            "last_login_at": bson::DateTime::from_chrono(now),
            "updated_at": bson::DateTime::from_chrono(now),
        };
        if let Some(ref name) = profile.display_name {
            update.insert("display_name", name);
        }
        if let Some(ref avatar) = profile.avatar_url {
            update.insert("avatar_url", avatar);
        }
        users
            .update_one(doc! { "_id": &user.id }, doc! { "$set": update })
            .await?;
        return Ok(user);
    }

    // Case 2: Existing email user (account linking)
    //
    // Trust the provider's email verification: this is an accepted industry
    // pattern used by Auth0, Supabase Auth, and Firebase Auth. The provider
    // has already verified the email address as part of its own OAuth flow.
    let email_lower = profile.email.to_lowercase();
    let existing_email = users.find_one(doc! { "email": &email_lower }).await?;

    if let Some(user) = existing_email {
        if !user.is_active {
            return Err(AppError::SocialAuthDeactivated);
        }

        if user.social_provider.is_some() {
            return Err(AppError::SocialAuthConflict);
        }

        // Link social identity to existing email/password user.
        // Use a conditional filter to prevent TOCTOU race: only update if
        // social_provider is still null (no concurrent linking occurred).
        let now = Utc::now();
        let mut update = doc! {
            "social_provider": profile.provider.as_str(),
            "social_provider_id": &profile.provider_id,
            "last_login_at": bson::DateTime::from_chrono(now),
            "updated_at": bson::DateTime::from_chrono(now),
        };
        if user.avatar_url.is_none() {
            if let Some(ref avatar) = profile.avatar_url {
                update.insert("avatar_url", avatar);
            }
        }
        if !user.email_verified {
            update.insert("email_verified", true);
        }
        let result = users
            .update_one(
                doc! { "_id": &user.id, "social_provider": null },
                doc! { "$set": update },
            )
            .await?;
        if result.modified_count == 0 {
            return Err(AppError::SocialAuthConflict);
        }
        return Ok(user);
    }

    // Case 3: New social user
    let now = Utc::now();
    let user_id = Uuid::new_v4().to_string();

    let new_user = User {
        id: user_id.clone(),
        email: email_lower,
        password_hash: None,
        display_name: profile.display_name.clone(),
        avatar_url: profile.avatar_url.clone(),
        email_verified: true,
        email_verification_token: None,
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin: false,
        role_ids: vec![],
        group_ids: vec![],
        mfa_enabled: false,
        social_provider: Some(profile.provider.as_str().to_string()),
        social_provider_id: Some(profile.provider_id.clone()),
        created_at: now,
        updated_at: now,
        last_login_at: Some(now),
    };

    users.insert_one(&new_user).await?;

    tracing::info!(user_id = %user_id, provider = %profile.provider.as_str(), "Social user created");

    Ok(new_user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_from_str_valid() {
        assert_eq!(
            SocialProvider::parse("github"),
            Some(SocialProvider::GitHub)
        );
        assert_eq!(
            SocialProvider::parse("google"),
            Some(SocialProvider::Google)
        );
        assert_eq!(SocialProvider::parse("apple"), Some(SocialProvider::Apple));
    }

    #[test]
    fn provider_from_str_invalid() {
        assert_eq!(SocialProvider::parse("facebook"), None);
        assert_eq!(SocialProvider::parse(""), None);
        assert_eq!(SocialProvider::parse("GitHub"), None);
    }

    #[test]
    fn provider_as_str() {
        assert_eq!(SocialProvider::GitHub.as_str(), "github");
        assert_eq!(SocialProvider::Google.as_str(), "google");
        assert_eq!(SocialProvider::Apple.as_str(), "apple");
    }

    #[test]
    fn provider_roundtrip() {
        for name in &["github", "google", "apple"] {
            let provider = SocialProvider::parse(name).unwrap();
            assert_eq!(provider.as_str(), *name);
        }
    }

    fn make_test_config(
        github_id: Option<&str>,
        github_secret: Option<&str>,
        google_id: Option<&str>,
        google_secret: Option<&str>,
    ) -> AppConfig {
        AppConfig {
            port: 3001,
            base_url: "http://localhost:3001".to_string(),
            frontend_url: "http://localhost:3000".to_string(),
            database_url: "mongodb://localhost:27017/nyxid".to_string(),
            database_max_connections: 10,
            environment: "development".to_string(),
            jwt_private_key_path: "keys/private.pem".to_string(),
            jwt_public_key_path: "keys/public.pem".to_string(),
            jwt_issuer: "http://localhost:3001".to_string(),
            jwt_access_ttl_secs: 900,
            jwt_refresh_ttl_secs: 604800,
            google_client_id: google_id.map(String::from),
            google_client_secret: google_secret.map(String::from),
            github_client_id: github_id.map(String::from),
            github_client_secret: github_secret.map(String::from),
            apple_client_id: None,
            apple_team_id: None,
            apple_key_id: None,
            apple_private_key_path: None,
            smtp_host: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_from_address: None,
            encryption_key: "ab".repeat(32),
            rate_limit_per_second: 10,
            rate_limit_burst: 30,
            sa_token_ttl_secs: 3600,
            cookie_domain: None,
            telegram_bot_token: None,
            telegram_webhook_secret: None,
            telegram_webhook_url: None,
            telegram_bot_username: None,
            approval_expiry_interval_secs: 5,
            fcm_service_account_path: None,
            fcm_project_id: None,
            apns_key_path: None,
            apns_key_id: None,
            apns_team_id: None,
            apns_topic: None,
            apns_sandbox: true,
        }
    }

    #[test]
    fn build_github_url() {
        let config = make_test_config(Some("gh_id"), Some("gh_secret"), None, None);
        let url =
            build_authorization_url(SocialProvider::GitHub, "test_state", None, &config).unwrap();
        assert!(url.starts_with("https://github.com/login/oauth/authorize"));
        assert!(url.contains("client_id=gh_id"));
        assert!(url.contains("state=test_state"));
        assert!(url.contains("scope=user:email"));
        assert!(url.contains("callback"));
    }

    #[test]
    fn build_google_url() {
        let config = make_test_config(None, None, Some("goog_id"), Some("goog_secret"));
        let url =
            build_authorization_url(SocialProvider::Google, "test_state", None, &config).unwrap();
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth"));
        assert!(url.contains("client_id=goog_id"));
        assert!(url.contains("state=test_state"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("callback"));
    }

    #[test]
    fn build_url_errors_when_not_configured() {
        let config = make_test_config(None, None, None, None);
        let result = build_authorization_url(SocialProvider::GitHub, "state", None, &config);
        assert!(result.is_err());
        let result = build_authorization_url(SocialProvider::Google, "state", None, &config);
        assert!(result.is_err());
    }

    #[test]
    fn build_url_errors_when_secret_missing() {
        // Has client_id but not secret
        let config = make_test_config(Some("gh_id"), None, None, None);
        let result = build_authorization_url(SocialProvider::GitHub, "state", None, &config);
        assert!(result.is_err());
    }

    #[test]
    fn build_apple_url_requires_nonce() {
        let mut config = make_test_config(None, None, None, None);
        config.apple_client_id = Some("com.example.nyxid".to_string());
        config.apple_team_id = Some("TEAM123".to_string());
        config.apple_key_id = Some("KEY123".to_string());
        config.apple_private_key_path = Some("keys/apple.p8".to_string());

        let err = build_authorization_url(SocialProvider::Apple, "state", None, &config)
            .expect_err("apple url should require nonce");
        assert!(matches!(err, AppError::SocialAuthFailed(_)));
    }

    #[test]
    fn build_apple_url_includes_form_post_and_nonce() {
        let mut config = make_test_config(None, None, None, None);
        config.apple_client_id = Some("com.example.nyxid".to_string());
        config.apple_team_id = Some("TEAM123".to_string());
        config.apple_key_id = Some("KEY123".to_string());
        config.apple_private_key_path = Some("keys/apple.p8".to_string());

        let url = build_authorization_url(
            SocialProvider::Apple,
            "test_state",
            Some("test_nonce"),
            &config,
        )
        .unwrap();
        assert!(url.starts_with("https://appleid.apple.com/auth/authorize"));
        assert!(url.contains("response_mode=form_post"));
        assert!(url.contains("nonce=test_nonce"));
        assert!(url.contains("state=test_state"));
        assert!(url.contains("scope=name%20email"));
    }
}
