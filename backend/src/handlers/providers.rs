use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, provider_service};
use crate::AppState;

use super::services_helpers::{require_admin, validate_base_url};

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct CreateProviderRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub provider_type: String,
    // OAuth2 fields
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub supports_pkce: Option<bool>,
    // Device code flow fields
    pub device_code_url: Option<String>,
    pub device_token_url: Option<String>,
    pub device_verification_url: Option<String>,
    pub hosted_callback_url: Option<String>,
    // API key fields
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    // Display
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
}

impl std::fmt::Debug for CreateProviderRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateProviderRequest")
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("provider_type", &self.provider_type)
            .field("client_id", &self.client_id.as_ref().map(|_| "[REDACTED]"))
            .field("client_secret", &self.client_secret.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Deserialize)]
pub struct UpdateProviderRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub is_active: Option<bool>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub supports_pkce: Option<bool>,
    pub device_code_url: Option<String>,
    pub device_token_url: Option<String>,
    pub device_verification_url: Option<String>,
    pub hosted_callback_url: Option<String>,
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
}

impl std::fmt::Debug for UpdateProviderRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateProviderRequest")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("is_active", &self.is_active)
            .field("client_id", &self.client_id.as_ref().map(|_| "[REDACTED]"))
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Validate slug format: lowercase alphanumeric and hyphens only, no
/// leading/trailing/consecutive hyphens.
fn validate_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty() || slug.len() > 100 {
        return Err(AppError::ValidationError(
            "slug must be between 1 and 100 characters".to_string(),
        ));
    }
    let valid = slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--");
    if !valid {
        return Err(AppError::ValidationError(
            "slug must contain only lowercase letters, digits, and hyphens (no leading/trailing/consecutive hyphens)".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ProviderResponse {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub provider_type: String,
    pub has_oauth_config: bool,
    pub default_scopes: Option<Vec<String>>,
    pub supports_pkce: bool,
    pub device_code_url: Option<String>,
    pub device_token_url: Option<String>,
    pub device_verification_url: Option<String>,
    pub hosted_callback_url: Option<String>,
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderResponse>,
}

#[derive(Debug, Serialize)]
pub struct DeleteProviderResponse {
    pub message: String,
}

fn provider_to_response(p: crate::models::provider_config::ProviderConfig) -> ProviderResponse {
    ProviderResponse {
        id: p.id,
        slug: p.slug,
        name: p.name,
        description: p.description,
        provider_type: p.provider_type,
        has_oauth_config: p.client_id_encrypted.is_some(),
        default_scopes: p.default_scopes,
        supports_pkce: p.supports_pkce,
        device_code_url: p.device_code_url,
        device_token_url: p.device_token_url,
        device_verification_url: p.device_verification_url,
        hosted_callback_url: p.hosted_callback_url,
        api_key_instructions: p.api_key_instructions,
        api_key_url: p.api_key_url,
        icon_url: p.icon_url,
        documentation_url: p.documentation_url,
        is_active: p.is_active,
        created_at: p.created_at.to_rfc3339(),
        updated_at: p.updated_at.to_rfc3339(),
    }
}

// --- Handlers ---

/// GET /api/v1/providers
pub async fn list_providers(
    State(state): State<AppState>,
    _auth_user: AuthUser,
) -> AppResult<Json<ProviderListResponse>> {
    let providers = provider_service::list_providers(&state.db).await?;

    let items: Vec<ProviderResponse> = providers
        .into_iter()
        .map(provider_to_response)
        .collect();

    Ok(Json(ProviderListResponse { providers: items }))
}

/// POST /api/v1/providers
pub async fn create_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateProviderRequest>,
) -> AppResult<Json<ProviderResponse>> {
    require_admin(&state, &auth_user).await?;

    if body.name.is_empty() || body.slug.is_empty() {
        return Err(AppError::ValidationError(
            "name and slug are required".to_string(),
        ));
    }

    if body.name.len() > 200 {
        return Err(AppError::ValidationError(
            "name exceeds maximum length of 200 characters".to_string(),
        ));
    }

    // Validate slug format
    validate_slug(&body.slug)?;

    // Validate provider_type
    let valid_types = ["oauth2", "api_key", "device_code"];
    if !valid_types.contains(&body.provider_type.as_str()) {
        return Err(AppError::ValidationError(format!(
            "provider_type must be one of: {}",
            valid_types.join(", ")
        )));
    }

    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let user_id_str = auth_user.user_id.to_string();

    let oauth_config = if body.provider_type == "oauth2" {
        let client_id = body.client_id.as_ref().ok_or_else(|| {
            AppError::ValidationError("client_id is required for OAuth2 providers".to_string())
        })?;
        let client_secret = body.client_secret.as_ref().ok_or_else(|| {
            AppError::ValidationError(
                "client_secret is required for OAuth2 providers".to_string(),
            )
        })?;
        let authorization_url = body.authorization_url.as_ref().ok_or_else(|| {
            AppError::ValidationError(
                "authorization_url is required for OAuth2 providers".to_string(),
            )
        })?;
        let token_url = body.token_url.as_ref().ok_or_else(|| {
            AppError::ValidationError("token_url is required for OAuth2 providers".to_string())
        })?;

        // SSRF validation on OAuth provider URLs
        validate_base_url(authorization_url)?;
        validate_base_url(token_url)?;

        Some(provider_service::OAuthProviderInput {
            authorization_url: authorization_url.clone(),
            token_url: token_url.clone(),
            revocation_url: body.revocation_url.clone(),
            default_scopes: body.default_scopes.clone(),
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            supports_pkce: body.supports_pkce.unwrap_or(false),
        })
    } else {
        None
    };

    let device_code_config = if body.provider_type == "device_code" {
        let client_id = body.client_id.as_ref().ok_or_else(|| {
            AppError::ValidationError(
                "client_id is required for device_code providers".to_string(),
            )
        })?;
        let authorization_url = body.authorization_url.as_ref().ok_or_else(|| {
            AppError::ValidationError(
                "authorization_url is required for device_code providers".to_string(),
            )
        })?;
        let token_url = body.token_url.as_ref().ok_or_else(|| {
            AppError::ValidationError(
                "token_url is required for device_code providers".to_string(),
            )
        })?;
        let device_code_url = body.device_code_url.as_ref().ok_or_else(|| {
            AppError::ValidationError(
                "device_code_url is required for device_code providers".to_string(),
            )
        })?;
        let device_token_url = body.device_token_url.as_ref().ok_or_else(|| {
            AppError::ValidationError(
                "device_token_url is required for device_code providers".to_string(),
            )
        })?;

        // SSRF validation on all URLs
        validate_base_url(authorization_url)?;
        validate_base_url(token_url)?;
        validate_base_url(device_code_url)?;
        validate_base_url(device_token_url)?;
        if let Some(ref url) = body.device_verification_url {
            validate_base_url(url)?;
        }
        if let Some(ref url) = body.hosted_callback_url {
            validate_base_url(url)?;
        }

        Some(provider_service::DeviceCodeProviderInput {
            authorization_url: authorization_url.clone(),
            token_url: token_url.clone(),
            device_code_url: device_code_url.clone(),
            device_token_url: device_token_url.clone(),
            device_verification_url: body.device_verification_url.clone(),
            hosted_callback_url: body.hosted_callback_url.clone(),
            default_scopes: body.default_scopes.clone(),
            client_id: client_id.clone(),
            client_secret: body.client_secret.clone(),
            supports_pkce: body.supports_pkce.unwrap_or(true),
        })
    } else {
        None
    };

    let api_key_config = if body.provider_type == "api_key" {
        Some(provider_service::ApiKeyProviderInput {
            api_key_instructions: body.api_key_instructions.clone(),
            api_key_url: body.api_key_url.clone(),
        })
    } else {
        None
    };

    let provider = provider_service::create_provider(
        &state.db,
        &encryption_key,
        &body.name,
        &body.slug,
        &body.provider_type,
        oauth_config,
        api_key_config,
        device_code_config,
        body.description.as_deref(),
        body.icon_url.as_deref(),
        body.documentation_url.as_deref(),
        &user_id_str,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_created".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider.id,
            "slug": &provider.slug,
        })),
        None,
        None,
    );

    Ok(Json(provider_to_response(provider)))
}

/// GET /api/v1/providers/{provider_id}
pub async fn get_provider(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<ProviderResponse>> {
    let provider = provider_service::get_provider(&state.db, &provider_id).await?;
    Ok(Json(provider_to_response(provider)))
}

/// PUT /api/v1/providers/{provider_id}
pub async fn update_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<UpdateProviderRequest>,
) -> AppResult<Json<ProviderResponse>> {
    require_admin(&state, &auth_user).await?;

    // SSRF validation on URLs if provided
    if let Some(ref url) = body.authorization_url {
        validate_base_url(url)?;
    }
    if let Some(ref url) = body.token_url {
        validate_base_url(url)?;
    }
    if let Some(ref url) = body.device_code_url {
        validate_base_url(url)?;
    }
    if let Some(ref url) = body.device_token_url {
        validate_base_url(url)?;
    }
    if let Some(ref url) = body.device_verification_url {
        validate_base_url(url)?;
    }
    if let Some(ref url) = body.hosted_callback_url {
        validate_base_url(url)?;
    }

    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let updates = provider_service::ProviderUpdateInput {
        name: body.name,
        description: body.description,
        is_active: body.is_active,
        authorization_url: body.authorization_url,
        token_url: body.token_url,
        revocation_url: body.revocation_url,
        default_scopes: body.default_scopes,
        client_id: body.client_id,
        client_secret: body.client_secret,
        supports_pkce: body.supports_pkce,
        device_code_url: body.device_code_url,
        device_token_url: body.device_token_url,
        device_verification_url: body.device_verification_url,
        hosted_callback_url: body.hosted_callback_url,
        api_key_instructions: body.api_key_instructions,
        api_key_url: body.api_key_url,
        icon_url: body.icon_url,
        documentation_url: body.documentation_url,
    };

    let updated = provider_service::update_provider(
        &state.db,
        &encryption_key,
        &provider_id,
        updates,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "provider_updated".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
    );

    Ok(Json(provider_to_response(updated)))
}

/// DELETE /api/v1/providers/{provider_id}
pub async fn delete_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<DeleteProviderResponse>> {
    require_admin(&state, &auth_user).await?;

    provider_service::delete_provider(&state.db, &provider_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "provider_deleted".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
    );

    Ok(Json(DeleteProviderResponse {
        message: "Provider deactivated and user tokens revoked".to_string(),
    }))
}
