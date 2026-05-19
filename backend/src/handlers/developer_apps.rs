use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use url::Url;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::mw::auth::AuthUser;
use crate::services::{oauth_client_service, org_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event, hash_short_id};
use mongodb::bson::doc;

/// Resolve which user_id owns this developer OAuth client and whether the
/// actor may modify it. The OauthClient's `created_by` field is the
/// owner -- if it points at an org user, org admins can manage it; org
/// members and viewers cannot.
///
/// `OrgMembership.allowed_service_ids` is *not* applied here. That scope
/// lives in `UserService.id` space and gates which proxyable services
/// an admin may manage; an OAuth client is a developer app identity,
/// not a service. Org admins manage every org-owned OAuth client as a
/// unit.
async fn resolve_developer_app_write_owner(
    state: &AppState,
    actor: &str,
    client_id: &str,
) -> AppResult<String> {
    let client = state
        .db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id })
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))?;

    let owner = client
        .created_by
        .as_deref()
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, owner).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this OAuth client".to_string(),
        ));
    }
    Ok(owner.to_string())
}

/// Read variant: any active member of the owning org (or the direct
/// creator) may view the client. See `resolve_developer_app_write_owner`
/// for why the membership scope is not applied at the resource level.
async fn resolve_developer_app_read_owner(
    state: &AppState,
    actor: &str,
    client_id: &str,
) -> AppResult<String> {
    let client = state
        .db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id })
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))?;

    let owner = client
        .created_by
        .as_deref()
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, owner).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("OAuth client not found".to_string()));
    }
    Ok(owner.to_string())
}

// ── Request / Response DTOs ──

#[derive(Debug, Deserialize)]
pub struct CreateDeveloperOAuthClientRequest {
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub client_type: Option<String>,
    /// Space-separated delegation scopes (empty = token exchange disabled).
    pub delegation_scopes: Option<String>,
    pub broker_capability_enabled: Option<bool>,
    pub revocation_webhook_url: Option<String>,
    pub revocation_webhook_secret: Option<String>,
    /// OIDC scopes this client is allowed to request (e.g. `["openid", "profile", "email", "roles"]`).
    /// Defaults to `["openid", "profile", "email"]` when omitted; `[]` canonicalizes to `["openid"]`.
    pub allowed_scopes: Option<Vec<String>>,
    /// When set, create this OAuth client under the given org. The
    /// `created_by` field is set to the org's user_id, making the client
    /// manageable by every admin of that org. The caller must be an admin
    /// of the target org.
    pub target_org_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDeveloperOAuthClientRequest {
    pub name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    /// Space-separated delegation scopes (empty = token exchange disabled).
    pub delegation_scopes: Option<String>,
    pub broker_capability_enabled: Option<bool>,
    pub revocation_webhook_url: Option<String>,
    pub revocation_webhook_secret: Option<String>,
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
    pub broker_capability_enabled: bool,
    pub revocation_webhook_url: Option<String>,
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
        broker_capability_enabled: c.broker_capability_enabled,
        revocation_webhook_url: c.revocation_webhook_url,
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

fn normalize_optional_nonempty(input: Option<&str>) -> Option<&str> {
    input.map(str::trim).filter(|value| !value.is_empty())
}

// ── Handlers ──

/// POST /api/v1/developer/oauth-clients
pub async fn create_my_oauth_client(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
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
    let actor = auth_user.user_id.to_string();
    let user_id = if let Some(target_org_id) = body.target_org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create OAuth clients under it"
                    .to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor
    };

    let allowed_scopes = body
        .allowed_scopes
        .as_deref()
        .map(oauth_client_service::validate_allowed_scopes_list)
        .transpose()?
        .unwrap_or_else(|| oauth_client_service::DEFAULT_ALLOWED_SCOPES.to_string());
    let revocation_webhook_url =
        normalize_optional_nonempty(body.revocation_webhook_url.as_deref());
    let revocation_webhook_secret_encrypted =
        match normalize_optional_nonempty(body.revocation_webhook_secret.as_deref()) {
            Some(secret) => Some(state.encryption_keys.encrypt(secret.as_bytes()).await?),
            None => None,
        };

    let (client, raw_secret) = oauth_client_service::create_client(
        &state.db,
        &body.name,
        &validated_uris,
        client_type,
        &user_id,
        delegation_scopes,
        &allowed_scopes,
        body.broker_capability_enabled.unwrap_or(false),
        revocation_webhook_url,
        revocation_webhook_secret_encrypted,
    )
    .await?;

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::OauthClientRegistered,
    );

    Ok(Json(to_response(client, raw_secret)))
}

#[derive(Debug, Deserialize)]
pub struct ListDeveloperAppsQuery {
    /// When set, list OAuth clients owned by the given org instead of the
    /// caller's personal scope. The caller must be an admin of that org.
    pub org_id: Option<String>,
}

/// GET /api/v1/developer/oauth-clients
pub async fn list_my_oauth_clients(
    State(state): State<AppState>,
    auth_user: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ListDeveloperAppsQuery>,
) -> AppResult<Json<DeveloperOAuthClientListResponse>> {
    let actor = auth_user.user_id.to_string();
    let user_id = if let Some(target_org_id) = query.org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its OAuth clients".to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor
    };
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
    let actor = auth_user.user_id.to_string();
    let user_id = resolve_developer_app_read_owner(&state, &actor, &client_id).await?;
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

    let actor = auth_user.user_id.to_string();
    let user_id = resolve_developer_app_write_owner(&state, &actor, &client_id).await?;

    let validated_allowed_scopes = body
        .allowed_scopes
        .as_deref()
        .map(oauth_client_service::validate_allowed_scopes_list)
        .transpose()?;
    let revocation_webhook_url =
        normalize_optional_nonempty(body.revocation_webhook_url.as_deref());
    let revocation_webhook_secret_encrypted =
        match normalize_optional_nonempty(body.revocation_webhook_secret.as_deref()) {
            Some(secret) => Some(state.encryption_keys.encrypt(secret.as_bytes()).await?),
            None => None,
        };

    let updated = oauth_client_service::update_client_for_creator(
        &state.db,
        &client_id,
        &user_id,
        body.name.as_deref().map(str::trim),
        validated_uris.as_deref(),
        body.delegation_scopes.as_deref(),
        validated_allowed_scopes.as_deref(),
        body.broker_capability_enabled,
        revocation_webhook_url,
        revocation_webhook_secret_encrypted,
    )
    .await?;

    Ok(Json(to_response(updated, None)))
}

/// POST /api/v1/developer/oauth-clients/:client_id/rotate-secret
pub async fn rotate_my_oauth_client_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(client_id): Path<String>,
) -> AppResult<Json<RotateDeveloperClientSecretResponse>> {
    let actor = auth_user.user_id.to_string();
    let user_id = resolve_developer_app_write_owner(&state, &actor, &client_id).await?;
    let (updated, new_secret) =
        oauth_client_service::rotate_client_secret_for_creator(&state.db, &client_id, &user_id)
            .await?;

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::OauthClientSecretRotated {
            // Hash: raw UUID would be scrubbed to `[UUID_REDACTED]`.
            client_id: hash_short_id(&updated.id),
        },
    );

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
    let actor = auth_user.user_id.to_string();
    let user_id = resolve_developer_app_write_owner(&state, &actor, &client_id).await?;
    oauth_client_service::delete_client_for_creator(&state.db, &client_id, &user_id).await?;
    Ok(Json(
        serde_json::json!({ "message": "OAuth client deactivated" }),
    ))
}
