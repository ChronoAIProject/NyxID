use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, provider_service, user_credentials_service};

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct SetUserCredentialsRequest {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub label: Option<String>,
}

impl std::fmt::Debug for SetUserCredentialsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetUserCredentialsRequest")
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("label", &self.label)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct UserCredentialsResponse {
    pub provider_config_id: String,
    pub has_credentials: bool,
    pub label: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteCredentialsResponse {
    pub message: String,
}

#[derive(Debug)]
struct ValidatedUserCredentialsInput {
    client_id: String,
    client_secret: Option<String>,
}

fn validate_set_user_credentials_request(
    provider: &crate::models::provider_config::ProviderConfig,
    body: &SetUserCredentialsRequest,
) -> AppResult<ValidatedUserCredentialsInput> {
    if body.client_id.trim().is_empty() || body.client_id.len() > 500 {
        return Err(AppError::ValidationError(
            "client_id must be between 1 and 500 characters".to_string(),
        ));
    }
    if body
        .client_secret
        .as_ref()
        .is_some_and(|value| value.len() > 2000)
    {
        return Err(AppError::ValidationError(
            "client_secret must be at most 2000 characters".to_string(),
        ));
    }

    let client_id = if provider.provider_type == "telegram_widget" {
        body.client_id.trim().to_string()
    } else {
        body.client_id.clone()
    };

    let client_secret = if provider.provider_type == "telegram_widget" {
        body.client_secret
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from)
    } else {
        body.client_secret
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(String::from)
    };

    if provider.provider_type == "telegram_widget" && client_secret.is_none() {
        return Err(AppError::ValidationError(
            "Bot token is required for Telegram widget providers".to_string(),
        ));
    }

    Ok(ValidatedUserCredentialsInput {
        client_id,
        client_secret,
    })
}

// --- Handlers ---

/// GET /api/v1/providers/{provider_id}/credentials
pub async fn get_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<UserCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let metadata = user_credentials_service::get_user_credentials_metadata(
        &state.db,
        &user_id_str,
        &provider_id,
    )
    .await?;

    match metadata {
        Some(m) => Ok(Json(UserCredentialsResponse {
            provider_config_id: m.provider_config_id,
            has_credentials: true,
            label: m.label,
            created_at: Some(m.created_at.to_rfc3339()),
            updated_at: Some(m.updated_at.to_rfc3339()),
        })),
        None => Ok(Json(UserCredentialsResponse {
            provider_config_id: provider_id,
            has_credentials: false,
            label: None,
            created_at: None,
            updated_at: None,
        })),
    }
}

/// PUT /api/v1/providers/{provider_id}/credentials
pub async fn set_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<SetUserCredentialsRequest>,
) -> AppResult<Json<UserCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Validate provider exists and is active
    let provider = provider_service::get_provider(&state.db, &provider_id).await?;
    if !provider.is_active {
        return Err(AppError::BadRequest("Provider is not active".to_string()));
    }

    // Validate credential_mode allows user credentials
    if !user_credentials_service::supports_user_credentials(&provider) {
        return Err(AppError::BadRequest(
            "This provider does not accept user-provided credentials".to_string(),
        ));
    }

    let validated = validate_set_user_credentials_request(&provider, &body)?;

    let cred = user_credentials_service::upsert_user_credentials(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        &validated.client_id,
        validated.client_secret.as_deref(),
        body.label.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "user_credentials_set".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
        })),
        None,
        None,
    );

    Ok(Json(UserCredentialsResponse {
        provider_config_id: cred.provider_config_id,
        has_credentials: true,
        label: cred.label,
        created_at: Some(cred.created_at.to_rfc3339()),
        updated_at: Some(cred.updated_at.to_rfc3339()),
    }))
}

/// DELETE /api/v1/providers/{provider_id}/credentials
pub async fn delete_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<DeleteCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    user_credentials_service::delete_user_credentials(&state.db, &user_id_str, &provider_id)
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "user_credentials_deleted".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
        })),
        None,
        None,
    );

    Ok(Json(DeleteCredentialsResponse {
        message: "Credentials deleted successfully".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::models::provider_config::ProviderConfig;

    fn make_provider(provider_type: &str) -> ProviderConfig {
        ProviderConfig {
            id: "provider-1".to_string(),
            slug: "provider-1".to_string(),
            name: "Provider 1".to_string(),
            description: None,
            provider_type: provider_type.to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            created_by: "tester".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn telegram_widget_user_credentials_require_a_bot_token() {
        let provider = make_provider("telegram_widget");
        let request = SetUserCredentialsRequest {
            client_id: "nyxid_bot".to_string(),
            client_secret: None,
            label: None,
        };

        let error = validate_set_user_credentials_request(&provider, &request)
            .expect_err("telegram widget credentials without a bot token should fail");

        assert!(matches!(error, AppError::ValidationError(_)));
        assert_eq!(
            error.to_string(),
            "Validation error: Bot token is required for Telegram widget providers"
        );
    }

    #[test]
    fn oauth_user_credentials_can_omit_a_client_secret() {
        let provider = make_provider("oauth2");
        let request = SetUserCredentialsRequest {
            client_id: "client-id".to_string(),
            client_secret: None,
            label: None,
        };

        let validated = validate_set_user_credentials_request(&provider, &request)
            .expect("oauth credentials without a client secret should remain valid");

        assert_eq!(validated.client_id, "client-id");
        assert!(validated.client_secret.is_none());
    }

    #[test]
    fn telegram_widget_user_credentials_trim_bot_values() {
        let provider = make_provider("telegram_widget");
        let request = SetUserCredentialsRequest {
            client_id: "  nyxid_bot  ".to_string(),
            client_secret: Some("  123456:ABC-DEF  ".to_string()),
            label: None,
        };

        let validated = validate_set_user_credentials_request(&provider, &request)
            .expect("telegram widget credentials should normalize");

        assert_eq!(validated.client_id, "nyxid_bot");
        assert_eq!(validated.client_secret.as_deref(), Some("123456:ABC-DEF"));
    }
}
