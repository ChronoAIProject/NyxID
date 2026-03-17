use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use url::Url;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::oauth_client::OauthClient;
use crate::mw::auth::AuthUser;
use crate::services::oauth_client_service;

// ── Request / Response DTOs ──

#[derive(Debug, Deserialize)]
pub struct CreateDeveloperOAuthClientRequest {
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub client_type: Option<String>,
    /// Space-separated delegation scopes (empty = token exchange disabled).
    pub delegation_scopes: Option<String>,
    /// OIDC scopes this client is allowed to request (e.g. `["openid", "profile", "email", "roles"]`).
    /// Defaults to `["openid", "profile", "email"]` when omitted; `[]` canonicalizes to `["openid"]`.
    pub allowed_scopes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDeveloperOAuthClientRequest {
    pub name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    /// Space-separated delegation scopes (empty = token exchange disabled).
    pub delegation_scopes: Option<String>,
    /// OIDC scopes this client is allowed to request. `[]` canonicalizes to `["openid"]`.
    pub allowed_scopes: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct DeveloperOAuthClientResponse {
    pub id: String,
    pub client_name: String,
    pub client_type: String,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: String,
    pub delegation_scopes: String,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct DeveloperOAuthClientListResponse {
    pub clients: Vec<DeveloperOAuthClientResponse>,
}

#[derive(Debug, Serialize)]
pub struct RotateDeveloperClientSecretResponse {
    pub id: String,
    pub client_secret: String,
}

// ── Shared helpers ──

fn to_response(c: OauthClient, secret: Option<String>) -> DeveloperOAuthClientResponse {
    DeveloperOAuthClientResponse {
        id: c.id,
        client_name: c.client_name,
        client_type: c.client_type,
        redirect_uris: c.redirect_uris,
        allowed_scopes: c.allowed_scopes,
        delegation_scopes: c.delegation_scopes,
        is_active: c.is_active,
        client_secret: secret,
        created_at: c.created_at.to_rfc3339(),
    }
}

fn validate_redirect_uris(redirect_uris: &[String]) -> AppResult<Vec<String>> {
    if redirect_uris.is_empty() {
        return Err(AppError::ValidationError(
            "At least one redirect_uri is required".to_string(),
        ));
    }

    let mut unique = HashSet::new();
    let mut validated = Vec::new();

    for raw_uri in redirect_uris {
        let uri = raw_uri.trim();
        if uri.is_empty() {
            return Err(AppError::ValidationError(
                "redirect_uri cannot be empty".to_string(),
            ));
        }

        let parsed = Url::parse(uri).map_err(|_| {
            AppError::ValidationError(format!("Invalid redirect_uri format: {uri}"))
        })?;

        if matches!(parsed.scheme(), "javascript" | "data" | "file") {
            return Err(AppError::ValidationError(format!(
                "Unsupported redirect_uri scheme: {uri}"
            )));
        }

        if parsed.fragment().is_some() {
            return Err(AppError::ValidationError(format!(
                "redirect_uri must not contain fragment: {uri}"
            )));
        }

        let normalized = parsed.to_string();
        if unique.insert(normalized.clone()) {
            validated.push(normalized);
        }
    }

    Ok(validated)
}

// ── Handlers ──

/// POST /api/v1/developer/oauth-clients
pub async fn create_my_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateDeveloperOAuthClientRequest>,
) -> AppResult<Json<DeveloperOAuthClientResponse>> {
    if body.name.trim().is_empty() {
        return Err(AppError::ValidationError(
            "Client name is required".to_string(),
        ));
    }

    let validated_uris = validate_redirect_uris(&body.redirect_uris)?;

    let client_type = body.client_type.as_deref().unwrap_or("public");
    if !matches!(client_type, "confidential" | "public") {
        return Err(AppError::ValidationError(
            "client_type must be 'confidential' or 'public'".to_string(),
        ));
    }

    let delegation_scopes = body.delegation_scopes.as_deref().unwrap_or("");
    let user_id = auth_user.user_id.to_string();

    let allowed_scopes = body
        .allowed_scopes
        .as_deref()
        .map(oauth_client_service::validate_allowed_scopes_list)
        .transpose()?
        .unwrap_or_else(|| oauth_client_service::DEFAULT_ALLOWED_SCOPES.to_string());

    let (client, raw_secret) = oauth_client_service::create_client(
        &state.db,
        &body.name,
        &validated_uris,
        client_type,
        &user_id,
        delegation_scopes,
        &allowed_scopes,
    )
    .await?;

    Ok(Json(to_response(client, raw_secret)))
}

/// GET /api/v1/developer/oauth-clients
pub async fn list_my_oauth_clients(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<DeveloperOAuthClientListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let clients = oauth_client_service::list_clients_by_creator(&state.db, &user_id).await?;

    let items = clients.into_iter().map(|c| to_response(c, None)).collect();

    Ok(Json(DeveloperOAuthClientListResponse { clients: items }))
}

/// GET /api/v1/developer/oauth-clients/:client_id
pub async fn get_my_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<DeveloperOAuthClientResponse>> {
    let user_id = auth_user.user_id.to_string();
    let c = oauth_client_service::get_client_for_creator(&state.db, &client_id, &user_id).await?;
    Ok(Json(to_response(c, None)))
}

/// PATCH /api/v1/developer/oauth-clients/:client_id
pub async fn update_my_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
    Json(body): Json<UpdateDeveloperOAuthClientRequest>,
) -> AppResult<Json<DeveloperOAuthClientResponse>> {
    if let Some(name) = body.name.as_ref()
        && name.trim().is_empty()
    {
        return Err(AppError::ValidationError(
            "Client name cannot be empty".to_string(),
        ));
    }

    let validated_uris = body
        .redirect_uris
        .as_ref()
        .map(|uris| validate_redirect_uris(uris))
        .transpose()?;

    let user_id = auth_user.user_id.to_string();

    let validated_allowed_scopes = body
        .allowed_scopes
        .as_deref()
        .map(oauth_client_service::validate_allowed_scopes_list)
        .transpose()?;

    let updated = oauth_client_service::update_client_for_creator(
        &state.db,
        &client_id,
        &user_id,
        body.name.as_deref().map(str::trim),
        validated_uris.as_deref(),
        body.delegation_scopes.as_deref(),
        validated_allowed_scopes.as_deref(),
    )
    .await?;

    Ok(Json(to_response(updated, None)))
}

/// POST /api/v1/developer/oauth-clients/:client_id/rotate-secret
pub async fn rotate_my_oauth_client_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<RotateDeveloperClientSecretResponse>> {
    let user_id = auth_user.user_id.to_string();
    let (updated, new_secret) =
        oauth_client_service::rotate_client_secret_for_creator(&state.db, &client_id, &user_id)
            .await?;

    Ok(Json(RotateDeveloperClientSecretResponse {
        id: updated.id,
        client_secret: new_secret,
    }))
}

/// DELETE /api/v1/developer/oauth-clients/:client_id
pub async fn delete_my_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = auth_user.user_id.to_string();
    oauth_client_service::delete_client_for_creator(&state.db, &client_id, &user_id).await?;
    Ok(Json(
        serde_json::json!({ "message": "OAuth client deactivated" }),
    ))
}
