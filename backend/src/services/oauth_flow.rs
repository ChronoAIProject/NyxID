use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use std::sync::LazyLock;
use zeroize::Zeroizing;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::user_provider_token::{COLLECTION_NAME, UserProviderToken};
use crate::services::user_credentials_service;

/// A reqwest client that does NOT follow redirects, preventing `client_secret`
/// from being forwarded to redirect targets (SEC-H2).
static TOKEN_EXCHANGE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create token exchange client")
});

/// Get the no-redirect HTTP client for OAuth token exchange operations.
pub fn token_exchange_client() -> &'static reqwest::Client {
    &TOKEN_EXCHANGE_CLIENT
}

/// Generate a PKCE code verifier (43-128 characters, URL-safe).
pub fn generate_code_verifier() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

/// Generate a PKCE S256 code challenge from a verifier.
pub fn generate_code_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, hash)
}

/// Refresh an OAuth2 access token using the stored refresh token.
///
/// Uses a dedicated no-redirect HTTP client (SEC-H2) and truncates error
/// bodies before storing (SEC-M5).
pub async fn refresh_oauth_token(
    db: &mongodb::Database,
    encryption_key: &[u8],
    token: &UserProviderToken,
) -> AppResult<String> {
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": &token.provider_config_id })
        .await?
        .ok_or_else(|| AppError::Internal("Provider config not found for refresh".to_string()))?;

    let token_url = provider.token_url.as_ref().ok_or_else(|| {
        AppError::Internal("OAuth provider missing token_url for refresh".to_string())
    })?;

    let resolved = user_credentials_service::resolve_token_oauth_credentials(
        db,
        encryption_key,
        &provider,
        token.credential_user_id.as_deref(),
    )
    .await?;
    let client_id = resolved.client_id;
    let client_secret = resolved.client_secret;

    let decrypted_rt = Zeroizing::new(aes::decrypt(
        token
            .refresh_token_encrypted
            .as_ref()
            .ok_or_else(|| AppError::Internal("Token missing refresh_token".to_string()))?,
        encryption_key,
    )?);
    let refresh_token = String::from_utf8((*decrypted_rt).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode refresh_token: {e}")))?;

    let use_basic_auth = provider.token_endpoint_auth_method == "client_secret_basic";
    let mut params = vec![
        ("grant_type".to_string(), "refresh_token".to_string()),
        ("refresh_token".to_string(), refresh_token.clone()),
    ];
    if use_basic_auth {
        // Credentials go in Authorization header, not body
    } else {
        params.push(("client_id".to_string(), client_id.clone()));
        if let Some(ref secret) = client_secret {
            params.push(("client_secret".to_string(), secret.clone()));
        }
    }

    let mut request = token_exchange_client()
        .post(token_url)
        .form(&params);
    if use_basic_auth {
        request = request.basic_auth(&client_id, client_secret.as_deref());
    }
    let response = request
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Token refresh request failed: {e}")))?;

    if !response.status().is_success() {
        let now = Utc::now();
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        // SEC-M5: Truncate error body before storing to prevent leaking provider internals
        let truncated_body = &body[..body.len().min(200)];

        db.collection::<UserProviderToken>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &token.id },
                doc! { "$set": {
                    "status": "refresh_failed",
                    "error_message": format!("Refresh failed: {status} {truncated_body}"),
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;

        return Err(AppError::Internal(format!(
            "Token refresh failed with status {status}"
        )));
    }

    let token_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse refresh response: {e}")))?;

    let new_access_token = token_data["access_token"].as_str().ok_or_else(|| {
        AppError::Internal("Missing access_token in refresh response".to_string())
    })?;

    let new_refresh_token = token_data["refresh_token"].as_str();
    let expires_in = token_data["expires_in"].as_i64();
    let now = Utc::now();

    let access_enc = aes::encrypt(new_access_token.as_bytes(), encryption_key)?;

    let mut set_doc = doc! {
        "access_token_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: access_enc,
        },
        "status": "active",
        "error_message": bson::Bson::Null,
        "last_refreshed_at": bson::DateTime::from_chrono(now),
        "updated_at": bson::DateTime::from_chrono(now),
    };

    if let Some(exp) = expires_in {
        let new_expires = now + Duration::seconds(exp);
        set_doc.insert("expires_at", bson::DateTime::from_chrono(new_expires));
    }

    if let Some(rt) = new_refresh_token {
        let rt_enc = aes::encrypt(rt.as_bytes(), encryption_key)?;
        set_doc.insert(
            "refresh_token_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: rt_enc,
            },
        );
    }

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .update_one(doc! { "_id": &token.id }, doc! { "$set": set_doc })
        .await?;

    tracing::info!(
        user_id = %token.user_id,
        provider_id = %token.provider_config_id,
        "OAuth token refreshed"
    );

    Ok(new_access_token.to_string())
}
