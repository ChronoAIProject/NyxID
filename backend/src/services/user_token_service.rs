use std::collections::HashMap;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::oauth_state::{COLLECTION_NAME as OAUTH_STATES, OAuthState};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::user_provider_token::{COLLECTION_NAME, UserProviderToken};
use crate::services::oauth_flow;

/// Decrypted token ready for injection.
pub struct DecryptedProviderToken {
    pub token_type: String,
    pub access_token: Option<String>,
    pub api_key: Option<String>,
}

/// Summary for listing (no decrypted tokens).
#[derive(Debug, serde::Serialize)]
pub struct UserProviderTokenSummary {
    pub provider_config_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub token_type: String,
    pub status: String,
    pub label: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
}

/// Store an API key for a provider.
pub async fn store_api_key(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_id: &str,
    api_key: &str,
    label: Option<&str>,
) -> AppResult<UserProviderToken> {
    // Verify provider exists and is active
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    if provider.provider_type != "api_key" {
        return Err(AppError::BadRequest(
            "This provider requires OAuth connection, not an API key".to_string(),
        ));
    }

    if api_key.is_empty() {
        return Err(AppError::ValidationError(
            "API key must not be empty".to_string(),
        ));
    }

    // Check if user already has a token for this provider
    let existing = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
            "status": { "$ne": "revoked" },
        })
        .await?;

    let now = Utc::now();
    let encrypted = aes::encrypt(api_key.as_bytes(), encryption_key)?;

    if let Some(existing_token) = existing {
        // Update existing token
        db.collection::<UserProviderToken>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &existing_token.id },
                doc! { "$set": {
                    "api_key_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: encrypted,
                    },
                    "status": "active",
                    "label": label,
                    "error_message": bson::Bson::Null,
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;

        let updated = db
            .collection::<UserProviderToken>(COLLECTION_NAME)
            .find_one(doc! { "_id": &existing_token.id })
            .await?
            .ok_or_else(|| AppError::Internal("Token disappeared after update".to_string()))?;

        return Ok(updated);
    }

    let token = UserProviderToken {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        token_type: "api_key".to_string(),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        token_scopes: None,
        expires_at: None,
        api_key_encrypted: Some(encrypted),
        status: "active".to_string(),
        last_refreshed_at: None,
        last_used_at: None,
        error_message: None,
        label: label.map(String::from),
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .insert_one(&token)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "API key stored for provider"
    );

    Ok(token)
}

/// Initiate an OAuth2 connection flow. Returns the authorization URL.
///
/// When `on_behalf_of` is `Some(sa_id)`, the flow stores tokens under the SA's
/// ID instead of the initiating user. `redirect_path` overrides the default
/// frontend callback path for the post-OAuth redirect.
pub async fn initiate_oauth_connect(
    db: &mongodb::Database,
    encryption_key: &[u8],
    base_url: &str,
    user_id: &str,
    provider_id: &str,
    on_behalf_of: Option<&str>,
    redirect_path: Option<&str>,
) -> AppResult<String> {
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    if provider.provider_type != "oauth2" {
        return Err(AppError::BadRequest(
            "This provider uses API keys, not OAuth".to_string(),
        ));
    }

    let authorization_url = provider.authorization_url.as_ref().ok_or_else(|| {
        AppError::Internal("OAuth provider missing authorization_url".to_string())
    })?;

    let client_id_bytes = provider
        .client_id_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("OAuth provider missing client_id".to_string()))?;
    let decrypted_cid = Zeroizing::new(aes::decrypt(client_id_bytes, encryption_key)?);
    let client_id = String::from_utf8((*decrypted_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode client_id: {e}")))?;

    // Create state for CSRF protection
    let state_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + Duration::minutes(10);

    // Generate PKCE code verifier if supported
    let code_verifier = if provider.supports_pkce {
        Some(oauth_flow::generate_code_verifier())
    } else {
        None
    };

    // SEC-M2: Encrypt code_verifier before storing
    let encrypted_verifier = code_verifier
        .as_ref()
        .map(|v| {
            let encrypted = aes::encrypt(v.as_bytes(), encryption_key)?;
            Ok::<_, AppError>(hex::encode(encrypted))
        })
        .transpose()?;

    let oauth_state = OAuthState {
        id: state_id.clone(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        code_verifier: encrypted_verifier,
        device_code_encrypted: None,
        user_code_encrypted: None,
        poll_interval: None,
        target_user_id: on_behalf_of.map(String::from),
        redirect_path: redirect_path.map(String::from),
        expires_at,
        created_at: now,
    };

    db.collection::<OAuthState>(OAUTH_STATES)
        .insert_one(&oauth_state)
        .await?;

    // Use the generic callback URL (matches the route registered for the callback)
    let callback_url = format!(
        "{}/api/v1/providers/callback",
        base_url.trim_end_matches('/')
    );

    let mut auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&state={}",
        authorization_url,
        urlencoding::encode(&client_id),
        urlencoding::encode(&callback_url),
        urlencoding::encode(&state_id),
    );

    if let Some(ref scopes) = provider.default_scopes {
        let scope_str = scopes.join(" ");
        auth_url.push_str(&format!("&scope={}", urlencoding::encode(&scope_str)));
    }

    if let Some(ref verifier) = code_verifier {
        let challenge = oauth_flow::generate_code_challenge(verifier);
        auth_url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            urlencoding::encode(&challenge)
        ));
    }

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        on_behalf_of = ?on_behalf_of,
        "OAuth connect flow initiated"
    );

    Ok(auth_url)
}

/// Result from requesting a device code (RFC 8628 step 1).
pub struct DeviceCodeInitiateResult {
    pub user_code: String,
    pub verification_uri: String,
    pub state: String,
    pub expires_in: i64,
    pub interval: i32,
}

/// Result from polling device code status (RFC 8628 step 3).
pub struct DeviceCodePollResult {
    pub status: String,
    pub interval: Option<i32>,
}

/// Step 1: Request a device code from the provider.
///
/// Calls the provider's device_code_url to get a device_auth_id + user_code,
/// stores the encrypted identifiers in an oauth_state, and returns the
/// user_code and verification_uri for the frontend to display.
///
/// When `on_behalf_of` is `Some(sa_id)`, the resulting tokens will be stored
/// under the SA's ID instead of the initiating user.
pub async fn request_device_code(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_id: &str,
    on_behalf_of: Option<&str>,
) -> AppResult<DeviceCodeInitiateResult> {
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    if provider.provider_type != "device_code" {
        return Err(AppError::BadRequest(
            "This provider does not use the device code flow".to_string(),
        ));
    }

    let device_code_url = provider.device_code_url.as_ref().ok_or_else(|| {
        AppError::Internal("Device code provider missing device_code_url".to_string())
    })?;

    let client_id_bytes = provider
        .client_id_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("Device code provider missing client_id".to_string()))?;
    let decrypted_cid = Zeroizing::new(aes::decrypt(client_id_bytes, encryption_key)?);
    let client_id = String::from_utf8((*decrypted_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode client_id: {e}")))?;

    // POST JSON with client_id to device code endpoint
    let body = serde_json::json!({ "client_id": &client_id });

    let response = oauth_flow::token_exchange_client()
        .post(device_code_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Device code request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let resp_body = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        tracing::error!(
            provider_id = %provider_id,
            status = %status,
            "Device code request returned error"
        );
        return Err(AppError::Internal(format!(
            "Device code request failed with status {status}: {}",
            &resp_body[..resp_body.len().min(200)]
        )));
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse device code response: {e}")))?;

    // OpenAI returns `device_auth_id`; standard RFC 8628 returns `device_code`
    let device_auth_id = data["device_auth_id"]
        .as_str()
        .or_else(|| data["device_code"].as_str())
        .ok_or_else(|| {
            AppError::Internal("Missing device_auth_id/device_code in response".to_string())
        })?;

    let user_code = data["user_code"]
        .as_str()
        .or_else(|| data["usercode"].as_str())
        .ok_or_else(|| AppError::Internal("Missing user_code in response".to_string()))?;

    // Verification URI: try response first, then provider config
    let verification_uri = data["verification_uri"]
        .as_str()
        .or_else(|| data["verification_url"].as_str())
        .map(String::from)
        .or_else(|| provider.device_verification_url.clone())
        .ok_or_else(|| {
            AppError::Internal("No verification URI in response or provider config".to_string())
        })?;

    // OpenAI returns interval as a string; handle both string and number
    let interval = data["interval"]
        .as_i64()
        .or_else(|| data["interval"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(5) as i32;

    // OpenAI returns expires_at (ISO timestamp); fall back to expires_in (seconds)
    let expires_in = if let Some(expires_at_str) = data["expires_at"].as_str() {
        chrono::DateTime::parse_from_rfc3339(expires_at_str)
            .map(|dt| (dt.timestamp() - Utc::now().timestamp()).max(60))
            .unwrap_or(900)
    } else {
        data["expires_in"].as_i64().unwrap_or(900)
    };

    // Encrypt device_auth_id and user_code before storing
    let device_code_encrypted =
        hex::encode(aes::encrypt(device_auth_id.as_bytes(), encryption_key)?);
    let user_code_encrypted = hex::encode(aes::encrypt(user_code.as_bytes(), encryption_key)?);

    // Create state document
    let state_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + Duration::seconds(expires_in);

    let oauth_state = OAuthState {
        id: state_id.clone(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        code_verifier: None,
        device_code_encrypted: Some(device_code_encrypted),
        user_code_encrypted: Some(user_code_encrypted),
        poll_interval: Some(interval),
        target_user_id: on_behalf_of.map(String::from),
        redirect_path: None,
        expires_at,
        created_at: now,
    };

    db.collection::<OAuthState>(OAUTH_STATES)
        .insert_one(&oauth_state)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        on_behalf_of = ?on_behalf_of,
        "Device code flow initiated"
    );

    Ok(DeviceCodeInitiateResult {
        user_code: user_code.to_string(),
        verification_uri,
        state: state_id,
        expires_in,
        interval,
    })
}

/// Step 3: Poll the provider's device token endpoint.
///
/// OpenAI-style: sends device_auth_id + user_code as JSON, checks HTTP status.
/// On 403/404 = still pending, on 2xx = success with authorization_code + PKCE,
/// then exchanges authorization_code at token_url for actual tokens.
pub async fn poll_device_code(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_id: &str,
    state: &str,
) -> AppResult<DeviceCodePollResult> {
    let now = Utc::now();

    // Look up state without deleting (we need it for multiple polls)
    let oauth_state = db
        .collection::<OAuthState>(OAUTH_STATES)
        .find_one(doc! { "_id": state })
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired device code state".to_string()))?;

    if oauth_state.expires_at < now {
        db.collection::<OAuthState>(OAUTH_STATES)
            .delete_one(doc! { "_id": state })
            .await?;
        return Ok(DeviceCodePollResult {
            status: "expired".to_string(),
            interval: None,
        });
    }

    if oauth_state.provider_config_id != provider_id {
        return Err(AppError::BadRequest(
            "Device code state provider mismatch".to_string(),
        ));
    }

    if oauth_state.user_id != user_id {
        return Err(AppError::BadRequest(
            "Device code state user mismatch".to_string(),
        ));
    }

    // When admin-on-behalf flow, store tokens under the target SA's ID
    let effective_user_id = oauth_state.target_user_id.as_deref().unwrap_or(user_id);

    // Decrypt device_auth_id
    let device_code_hex = oauth_state
        .device_code_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("OAuth state missing device_auth_id".to_string()))?;
    let dc_bytes = hex::decode(device_code_hex).map_err(|e| {
        AppError::Internal(format!("Failed to decode encrypted device_auth_id: {e}"))
    })?;
    let decrypted_dc = Zeroizing::new(aes::decrypt(&dc_bytes, encryption_key)?);
    let device_auth_id = String::from_utf8((*decrypted_dc).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode device_auth_id: {e}")))?;

    // Decrypt user_code
    let user_code_hex = oauth_state
        .user_code_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("OAuth state missing user_code".to_string()))?;
    let uc_bytes = hex::decode(user_code_hex)
        .map_err(|e| AppError::Internal(format!("Failed to decode encrypted user_code: {e}")))?;
    let decrypted_uc = Zeroizing::new(aes::decrypt(&uc_bytes, encryption_key)?);
    let user_code = String::from_utf8((*decrypted_uc).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode user_code: {e}")))?;

    // Load provider config
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))?;

    let device_token_url = provider.device_token_url.as_ref().ok_or_else(|| {
        AppError::Internal("Device code provider missing device_token_url".to_string())
    })?;

    // OpenAI-style poll: send device_auth_id + user_code as JSON
    let poll_body = serde_json::json!({
        "device_auth_id": &device_auth_id,
        "user_code": &user_code,
    });

    let response = oauth_flow::token_exchange_client()
        .post(device_token_url)
        .json(&poll_body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Device code poll failed: {e}")))?;

    let status_code = response.status();

    // OpenAI: 403/404 = authorization pending
    if status_code == reqwest::StatusCode::FORBIDDEN
        || status_code == reqwest::StatusCode::NOT_FOUND
    {
        return Ok(DeviceCodePollResult {
            status: "pending".to_string(),
            interval: oauth_state.poll_interval,
        });
    }

    if !status_code.is_success() {
        // Try to parse RFC 8628 error response as fallback
        if let Ok(resp_data) = response.json::<serde_json::Value>().await
            && let Some(error) = resp_data["error"].as_str()
        {
            match error {
                "authorization_pending" => {
                    return Ok(DeviceCodePollResult {
                        status: "pending".to_string(),
                        interval: oauth_state.poll_interval,
                    });
                }
                "slow_down" => {
                    let new_interval = oauth_state.poll_interval.unwrap_or(5) + 5;
                    db.collection::<OAuthState>(OAUTH_STATES)
                        .update_one(
                            doc! { "_id": state },
                            doc! { "$set": { "poll_interval": new_interval } },
                        )
                        .await?;
                    return Ok(DeviceCodePollResult {
                        status: "slow_down".to_string(),
                        interval: Some(new_interval),
                    });
                }
                "expired_token" => {
                    db.collection::<OAuthState>(OAUTH_STATES)
                        .delete_one(doc! { "_id": state })
                        .await?;
                    return Ok(DeviceCodePollResult {
                        status: "expired".to_string(),
                        interval: None,
                    });
                }
                "access_denied" => {
                    db.collection::<OAuthState>(OAUTH_STATES)
                        .delete_one(doc! { "_id": state })
                        .await?;
                    return Ok(DeviceCodePollResult {
                        status: "denied".to_string(),
                        interval: None,
                    });
                }
                _ => {}
            }
        }
        return Err(AppError::Internal(format!(
            "Device code poll returned unexpected status: {status_code}"
        )));
    }

    // Success (2xx): parse response
    let resp_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse poll response: {e}")))?;

    // OpenAI returns authorization_code + PKCE for a second exchange step
    if let Some(authorization_code) = resp_data["authorization_code"].as_str() {
        let code_verifier = resp_data["code_verifier"].as_str().ok_or_else(|| {
            AppError::Internal("Missing code_verifier in device poll response".to_string())
        })?;

        let token_url = provider.token_url.as_ref().ok_or_else(|| {
            AppError::Internal("Provider missing token_url for code exchange".to_string())
        })?;

        let client_id = decrypt_client_id(&provider, encryption_key)?;

        // Exchange authorization_code at token_url with PKCE
        // Codex CLI uses form-urlencoded (NOT JSON) and redirect_uri = {issuer}/deviceauth/callback
        let issuer = device_token_url
            .find("/api/accounts/")
            .map(|idx| &device_token_url[..idx])
            .unwrap_or("https://auth.openai.com");
        let redirect_uri = format!("{issuer}/deviceauth/callback");

        let token_params = [
            ("grant_type", "authorization_code"),
            ("code", authorization_code),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id.as_str()),
            ("code_verifier", code_verifier),
        ];

        let token_response = oauth_flow::token_exchange_client()
            .post(token_url)
            .form(&token_params)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Device code token exchange failed: {e}")))?;

        if !token_response.status().is_success() {
            let err_status = token_response.status();
            let err_body = token_response.text().await.unwrap_or_default();
            tracing::error!(
                status = %err_status,
                body = %&err_body[..err_body.len().min(200)],
                "Device code token exchange returned error"
            );
            return Err(AppError::Internal(format!(
                "Device code token exchange failed with status {err_status}"
            )));
        }

        let token_data: serde_json::Value = token_response.json().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse token exchange response: {e}"))
        })?;

        return store_device_code_tokens(
            db,
            encryption_key,
            effective_user_id,
            provider_id,
            state,
            &token_data,
            now,
        )
        .await;
    }

    // Standard flow: access_token directly in poll response
    store_device_code_tokens(
        db,
        encryption_key,
        effective_user_id,
        provider_id,
        state,
        &resp_data,
        now,
    )
    .await
}

/// Decrypt the client_id from a provider config.
fn decrypt_client_id(provider: &ProviderConfig, encryption_key: &[u8]) -> AppResult<String> {
    let cid_encrypted = provider
        .client_id_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("Provider missing client_id".to_string()))?;
    let decrypted = Zeroizing::new(aes::decrypt(cid_encrypted, encryption_key)?);
    String::from_utf8((*decrypted).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode client_id: {e}")))
}

/// Store tokens from a device code flow response (either direct or after code exchange).
async fn store_device_code_tokens(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_id: &str,
    state: &str,
    token_data: &serde_json::Value,
    now: chrono::DateTime<Utc>,
) -> AppResult<DeviceCodePollResult> {
    let access_token = token_data["access_token"]
        .as_str()
        .ok_or_else(|| AppError::Internal("Missing access_token in token response".to_string()))?;

    let refresh_token = token_data["refresh_token"].as_str();
    let expires_in = token_data["expires_in"].as_i64();
    let scope = token_data["scope"].as_str();

    let access_enc = aes::encrypt(access_token.as_bytes(), encryption_key)?;
    let refresh_enc = match refresh_token {
        Some(rt) => Some(aes::encrypt(rt.as_bytes(), encryption_key)?),
        None => None,
    };

    let token_expires_at = expires_in.map(|secs| now + Duration::seconds(secs));

    // Delete the oauth_state (flow complete)
    db.collection::<OAuthState>(OAUTH_STATES)
        .delete_one(doc! { "_id": state })
        .await?;

    // Upsert: remove existing token for this user+provider, insert new
    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .delete_many(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
        })
        .await?;

    let token = UserProviderToken {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        token_type: "oauth2".to_string(),
        access_token_encrypted: Some(access_enc),
        refresh_token_encrypted: refresh_enc,
        token_scopes: scope.map(String::from),
        expires_at: token_expires_at,
        api_key_encrypted: None,
        status: "active".to_string(),
        last_refreshed_at: None,
        last_used_at: None,
        error_message: None,
        label: None,
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .insert_one(&token)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "Device code OAuth token stored"
    );

    Ok(DeviceCodePollResult {
        status: "complete".to_string(),
        interval: None,
    })
}

/// Peek at an OAuth state without consuming it (for the generic callback handler).
pub async fn peek_oauth_state(db: &mongodb::Database, state_id: &str) -> AppResult<OAuthState> {
    db.collection::<OAuthState>(OAUTH_STATES)
        .find_one(doc! { "_id": state_id })
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired OAuth state".to_string()))
}

/// Handle the OAuth2 callback after user authorizes.
///
/// Uses a dedicated no-redirect HTTP client (SEC-H2) for the token exchange.
pub async fn handle_oauth_callback(
    db: &mongodb::Database,
    encryption_key: &[u8],
    base_url: &str,
    provider_id: &str,
    code: &str,
    state: &str,
) -> AppResult<UserProviderToken> {
    // Validate state (atomic claim -- delete to prevent replay)
    let now = Utc::now();
    let oauth_state = db
        .collection::<OAuthState>(OAUTH_STATES)
        .find_one_and_delete(doc! { "_id": state })
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired OAuth state".to_string()))?;

    if oauth_state.expires_at < now {
        return Err(AppError::BadRequest("OAuth state has expired".to_string()));
    }

    if oauth_state.provider_config_id != provider_id {
        return Err(AppError::BadRequest(
            "OAuth state provider mismatch".to_string(),
        ));
    }

    // When admin-on-behalf flow, store tokens under the target SA's ID
    let effective_user_id = oauth_state
        .target_user_id
        .as_deref()
        .unwrap_or(&oauth_state.user_id);
    let user_id = effective_user_id;

    // Load provider config
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))?;

    let token_url = provider
        .token_url
        .as_ref()
        .ok_or_else(|| AppError::Internal("OAuth provider missing token_url".to_string()))?;

    let decrypted_cid = Zeroizing::new(aes::decrypt(
        provider
            .client_id_encrypted
            .as_ref()
            .ok_or_else(|| AppError::Internal("OAuth provider missing client_id".to_string()))?,
        encryption_key,
    )?);
    let client_id = String::from_utf8((*decrypted_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode client_id: {e}")))?;

    let decrypted_csec = Zeroizing::new(aes::decrypt(
        provider.client_secret_encrypted.as_ref().ok_or_else(|| {
            AppError::Internal("OAuth provider missing client_secret".to_string())
        })?,
        encryption_key,
    )?);
    let client_secret = String::from_utf8((*decrypted_csec).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode client_secret: {e}")))?;

    // Use the generic callback URL (must match what was sent in initiate)
    let callback_url = format!(
        "{}/api/v1/providers/callback",
        base_url.trim_end_matches('/')
    );

    // Exchange code for tokens
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", callback_url),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ];

    // SEC-M2: Decrypt code_verifier from stored state
    if let Some(ref encrypted_verifier) = oauth_state.code_verifier {
        let verifier_bytes = hex::decode(encrypted_verifier)
            .map_err(|e| AppError::Internal(format!("Failed to decode encrypted verifier: {e}")))?;
        let decrypted = Zeroizing::new(aes::decrypt(&verifier_bytes, encryption_key)?);
        let verifier = String::from_utf8((*decrypted).clone())
            .map_err(|e| AppError::Internal(format!("Failed to decode verifier: {e}")))?;
        params.push(("code_verifier", verifier));
    }

    // SEC-H2: Use no-redirect client for token exchange
    let token_response = oauth_flow::token_exchange_client()
        .post(token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("OAuth token exchange failed: {e}")))?;

    if !token_response.status().is_success() {
        let status = token_response.status();
        let body = token_response
            .text()
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        tracing::error!(
            provider_id = %provider_id,
            status = %status,
            body = %body,
            "OAuth token exchange returned error"
        );
        return Err(AppError::Internal(format!(
            "OAuth token exchange failed with status {status}"
        )));
    }

    let token_data: serde_json::Value = token_response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse token response: {e}")))?;

    let access_token = token_data["access_token"]
        .as_str()
        .ok_or_else(|| AppError::Internal("Missing access_token in response".to_string()))?;

    let refresh_token = token_data["refresh_token"].as_str();
    let expires_in = token_data["expires_in"].as_i64();
    let scope = token_data["scope"].as_str();

    let access_enc = aes::encrypt(access_token.as_bytes(), encryption_key)?;
    let refresh_enc = match refresh_token {
        Some(rt) => Some(aes::encrypt(rt.as_bytes(), encryption_key)?),
        None => None,
    };

    let token_expires_at = expires_in.map(|secs| now + Duration::seconds(secs));

    // Upsert: remove existing token for this user+provider, insert new
    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .delete_many(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
        })
        .await?;

    let token = UserProviderToken {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider_config_id: provider_id.to_string(),
        token_type: "oauth2".to_string(),
        access_token_encrypted: Some(access_enc),
        refresh_token_encrypted: refresh_enc,
        token_scopes: scope.map(String::from),
        expires_at: token_expires_at,
        api_key_encrypted: None,
        status: "active".to_string(),
        last_refreshed_at: None,
        last_used_at: None,
        error_message: None,
        label: None,
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .insert_one(&token)
        .await?;

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "OAuth token stored for provider"
    );

    Ok(token)
}

/// Get a user's decrypted token for a provider, with lazy refresh for OAuth tokens.
pub async fn get_active_token(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_id: &str,
) -> AppResult<DecryptedProviderToken> {
    let token = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_id,
            "status": { "$in": ["active", "expired"] },
        })
        .await?
        .ok_or_else(|| AppError::NotFound("No active token found for this provider".to_string()))?;

    // Update last_used_at
    let now = Utc::now();
    db.collection::<UserProviderToken>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": &token.id },
            doc! { "$set": { "last_used_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;

    match token.token_type.as_str() {
        "api_key" => {
            let encrypted = token.api_key_encrypted.ok_or_else(|| {
                AppError::Internal("API key token missing encrypted key".to_string())
            })?;
            let decrypted_bytes = Zeroizing::new(aes::decrypt(&encrypted, encryption_key)?);
            let decrypted = String::from_utf8((*decrypted_bytes).clone())
                .map_err(|e| AppError::Internal(format!("Failed to decode API key: {e}")))?;

            Ok(DecryptedProviderToken {
                token_type: "api_key".to_string(),
                access_token: None,
                api_key: Some(decrypted),
            })
        }
        "oauth2" => {
            // Check if token needs refresh (5-minute buffer)
            let needs_refresh = token
                .expires_at
                .is_some_and(|exp| exp <= now + Duration::minutes(5));

            if needs_refresh && token.refresh_token_encrypted.is_some() {
                match oauth_flow::refresh_oauth_token(db, encryption_key, &token).await {
                    Ok(new_access_token) => {
                        return Ok(DecryptedProviderToken {
                            token_type: "oauth2".to_string(),
                            access_token: Some(new_access_token),
                            api_key: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            user_id = %user_id,
                            provider_id = %provider_id,
                            error = %e,
                            "Token refresh failed, attempting to use existing token"
                        );
                        // Fall through to return existing token
                    }
                }
            }

            let encrypted = token.access_token_encrypted.ok_or_else(|| {
                AppError::Internal("OAuth token missing encrypted access_token".to_string())
            })?;
            let decrypted_bytes = Zeroizing::new(aes::decrypt(&encrypted, encryption_key)?);
            let decrypted = String::from_utf8((*decrypted_bytes).clone())
                .map_err(|e| AppError::Internal(format!("Failed to decode access token: {e}")))?;

            Ok(DecryptedProviderToken {
                token_type: "oauth2".to_string(),
                access_token: Some(decrypted),
                api_key: None,
            })
        }
        other => Err(AppError::Internal(format!("Unknown token type: {other}"))),
    }
}

/// Revoke and delete a user's stored token for a provider.
pub async fn disconnect_provider(
    db: &mongodb::Database,
    user_id: &str,
    provider_id: &str,
) -> AppResult<()> {
    let now = Utc::now();

    let result = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .update_one(
            doc! {
                "user_id": user_id,
                "provider_config_id": provider_id,
                "status": { "$ne": "revoked" },
            },
            doc! { "$set": {
                "status": "revoked",
                "api_key_encrypted": bson::Bson::Null,
                "access_token_encrypted": bson::Bson::Null,
                "refresh_token_encrypted": bson::Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "No active token found for this provider".to_string(),
        ));
    }

    tracing::info!(
        user_id = %user_id,
        provider_id = %provider_id,
        "Provider disconnected"
    );

    Ok(())
}

/// List all providers the user has connected to, with status.
///
/// Uses a single batch query for provider lookups (CR-4/5/6: fix N+1).
pub async fn list_user_tokens(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<UserProviderTokenSummary>> {
    let tokens: Vec<UserProviderToken> = db
        .collection::<UserProviderToken>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "status": { "$ne": "revoked" } })
        .await?
        .try_collect()
        .await?;

    if tokens.is_empty() {
        return Ok(vec![]);
    }

    // Batch fetch all providers in a single query
    let provider_ids: Vec<&str> = tokens
        .iter()
        .map(|t| t.provider_config_id.as_str())
        .collect();
    let providers: Vec<ProviderConfig> = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find(doc! { "_id": { "$in": &provider_ids } })
        .await?
        .try_collect()
        .await?;
    let provider_map: HashMap<&str, &ProviderConfig> =
        providers.iter().map(|p| (p.id.as_str(), p)).collect();

    let summaries = tokens
        .iter()
        .map(|token| {
            let (provider_name, provider_slug) =
                match provider_map.get(token.provider_config_id.as_str()) {
                    Some(p) => (p.name.clone(), p.slug.clone()),
                    None => ("Unknown".to_string(), "unknown".to_string()),
                };

            UserProviderTokenSummary {
                provider_config_id: token.provider_config_id.clone(),
                provider_name,
                provider_slug,
                token_type: token.token_type.clone(),
                status: token.status.clone(),
                label: token.label.clone(),
                expires_at: token.expires_at.map(|dt| dt.to_rfc3339()),
                last_used_at: token.last_used_at.map(|dt| dt.to_rfc3339()),
                connected_at: token.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(summaries)
}
