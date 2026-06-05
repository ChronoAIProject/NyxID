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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::State;

    fn tele() -> TelemetryContext {
        TelemetryContext::default()
    }

    #[tokio::test]
    async fn create_and_list_oauth_client() {
        let Some(db) = connect_test_database("h_dev_apps_create_list").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_my_oauth_client(
            State(state.clone()),
            auth.clone(),
            tele(),
            Json(CreateDeveloperOAuthClientRequest {
                name: "Test App".to_string(),
                redirect_uris: vec!["https://example.com/callback".to_string()],
                client_type: Some("confidential".to_string()),
                delegation_scopes: None,
                broker_capability_enabled: None,
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
                target_org_id: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(created.client_name, "Test App");
        assert_eq!(created.client_type, "confidential");
        assert!(created.client_secret.is_some());
        assert!(created.is_active);

        let Json(list) = list_my_oauth_clients(
            State(state),
            auth,
            axum::extract::Query(ListDeveloperAppsQuery { org_id: None }),
        )
        .await
        .unwrap();

        assert_eq!(list.clients.len(), 1);
        assert_eq!(list.clients[0].id, created.id);
    }

    #[tokio::test]
    async fn get_oauth_client() {
        let Some(db) = connect_test_database("h_dev_apps_get").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_my_oauth_client(
            State(state.clone()),
            auth.clone(),
            tele(),
            Json(CreateDeveloperOAuthClientRequest {
                name: "Get App".to_string(),
                redirect_uris: vec!["https://example.com/cb".to_string()],
                client_type: None,
                delegation_scopes: None,
                broker_capability_enabled: None,
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
                target_org_id: None,
            }),
        )
        .await
        .unwrap();

        let Json(fetched) = get_my_oauth_client(State(state), auth, Path(created.id.clone()))
            .await
            .unwrap();

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.client_name, "Get App");
        assert!(fetched.client_secret.is_none());
    }

    #[tokio::test]
    async fn update_oauth_client() {
        let Some(db) = connect_test_database("h_dev_apps_update").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_my_oauth_client(
            State(state.clone()),
            auth.clone(),
            tele(),
            Json(CreateDeveloperOAuthClientRequest {
                name: "Before Update".to_string(),
                redirect_uris: vec!["https://example.com/cb".to_string()],
                client_type: Some("confidential".to_string()),
                delegation_scopes: None,
                broker_capability_enabled: None,
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
                target_org_id: None,
            }),
        )
        .await
        .unwrap();

        let Json(updated) = update_my_oauth_client(
            State(state),
            auth,
            Path(created.id.clone()),
            Json(UpdateDeveloperOAuthClientRequest {
                name: Some("After Update".to_string()),
                redirect_uris: None,
                delegation_scopes: None,
                broker_capability_enabled: Some(true),
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(updated.client_name, "After Update");
        assert!(updated.broker_capability_enabled);
    }

    #[tokio::test]
    async fn rotate_oauth_client_secret() {
        let Some(db) = connect_test_database("h_dev_apps_rotate").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_my_oauth_client(
            State(state.clone()),
            auth.clone(),
            tele(),
            Json(CreateDeveloperOAuthClientRequest {
                name: "Rotate App".to_string(),
                redirect_uris: vec!["https://example.com/cb".to_string()],
                client_type: Some("confidential".to_string()),
                delegation_scopes: None,
                broker_capability_enabled: None,
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
                target_org_id: None,
            }),
        )
        .await
        .unwrap();

        let original_secret = created.client_secret.unwrap();

        let Json(rotated) =
            rotate_my_oauth_client_secret(State(state), auth, tele(), Path(created.id.clone()))
                .await
                .unwrap();

        assert_eq!(rotated.id, created.id);
        assert_ne!(rotated.client_secret, original_secret);
    }

    #[tokio::test]
    async fn delete_oauth_client() {
        let Some(db) = connect_test_database("h_dev_apps_delete").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_my_oauth_client(
            State(state.clone()),
            auth.clone(),
            tele(),
            Json(CreateDeveloperOAuthClientRequest {
                name: "Delete App".to_string(),
                redirect_uris: vec!["https://example.com/cb".to_string()],
                client_type: Some("confidential".to_string()),
                delegation_scopes: None,
                broker_capability_enabled: None,
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
                target_org_id: None,
            }),
        )
        .await
        .unwrap();

        let Json(resp) =
            delete_my_oauth_client(State(state.clone()), auth.clone(), Path(created.id.clone()))
                .await
                .unwrap();

        assert_eq!(resp["message"], "OAuth client deactivated");

        let Json(fetched) = get_my_oauth_client(State(state), auth, Path(created.id))
            .await
            .unwrap();
        assert!(!fetched.is_active);
    }

    #[tokio::test]
    async fn create_rejects_empty_name() {
        let Some(db) = connect_test_database("h_dev_apps_empty_name").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = create_my_oauth_client(
            State(state),
            auth,
            tele(),
            Json(CreateDeveloperOAuthClientRequest {
                name: "   ".to_string(),
                redirect_uris: vec!["https://example.com/cb".to_string()],
                client_type: None,
                delegation_scopes: None,
                broker_capability_enabled: None,
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
                target_org_id: None,
            }),
        )
        .await;

        assert!(err.is_err());
    }

    // ── Pure function tests (no MongoDB) ──

    #[test]
    fn validate_redirect_uris_accepts_valid_https() {
        let result = validate_redirect_uris(&["https://example.com/callback".to_string()]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec!["https://example.com/callback"]);
    }

    #[test]
    fn validate_redirect_uris_rejects_empty_list() {
        let result = validate_redirect_uris(&[]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_rejects_empty_string() {
        let result = validate_redirect_uris(&["".to_string()]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_rejects_whitespace_only() {
        let result = validate_redirect_uris(&["   ".to_string()]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_rejects_javascript_scheme() {
        let result = validate_redirect_uris(&["javascript:alert(1)".to_string()]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_rejects_data_scheme() {
        let result = validate_redirect_uris(&["data:text/html,<h1>hi</h1>".to_string()]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_rejects_file_scheme() {
        let result = validate_redirect_uris(&["file:///etc/passwd".to_string()]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_rejects_fragment() {
        let result = validate_redirect_uris(&["https://example.com/cb#fragment".to_string()]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_rejects_invalid_url() {
        let result = validate_redirect_uris(&["not a url".to_string()]);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn validate_redirect_uris_deduplicates() {
        let result = validate_redirect_uris(&[
            "https://example.com/cb".to_string(),
            "https://example.com/cb".to_string(),
        ]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn validate_redirect_uris_trims_whitespace() {
        let result = validate_redirect_uris(&["  https://example.com/cb  ".to_string()]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec!["https://example.com/cb"]);
    }

    #[test]
    fn validate_redirect_uris_allows_localhost() {
        let result = validate_redirect_uris(&["http://localhost:3000/callback".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_redirect_uris_allows_custom_scheme() {
        let result = validate_redirect_uris(&["myapp://callback".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_redirect_uris_multiple_valid() {
        let result = validate_redirect_uris(&[
            "https://example.com/cb".to_string(),
            "https://other.com/cb".to_string(),
            "http://localhost:3000/cb".to_string(),
        ]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn normalize_optional_nonempty_returns_none_for_none() {
        assert!(normalize_optional_nonempty(None).is_none());
    }

    #[test]
    fn normalize_optional_nonempty_returns_none_for_empty() {
        assert!(normalize_optional_nonempty(Some("")).is_none());
    }

    #[test]
    fn normalize_optional_nonempty_returns_none_for_whitespace() {
        assert!(normalize_optional_nonempty(Some("   ")).is_none());
    }

    #[test]
    fn normalize_optional_nonempty_trims_and_returns() {
        assert_eq!(
            normalize_optional_nonempty(Some("  hello  ")),
            Some("hello")
        );
    }

    #[test]
    fn normalize_optional_nonempty_preserves_value() {
        assert_eq!(normalize_optional_nonempty(Some("value")), Some("value"));
    }

    #[test]
    fn to_response_maps_oauth_client_fields() {
        use chrono::Utc;
        let client = OauthClient {
            id: "client_1".to_string(),
            client_name: "My App".to_string(),
            client_type: "confidential".to_string(),
            client_secret_hash: "hash".to_string(),
            redirect_uris: vec!["https://ex.com/cb".to_string()],
            allowed_scopes: "openid profile".to_string(),
            grant_types: "authorization_code".to_string(),
            delegation_scopes: "proxy:*".to_string(),
            broker_capability_enabled: true,
            revocation_webhook_url: Some("https://ex.com/revoke".to_string()),
            revocation_webhook_secret_encrypted: None,
            is_active: true,
            created_by: Some("user_1".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let resp = to_response(client, Some("secret_value".to_string()));
        assert_eq!(resp.id, "client_1");
        assert_eq!(resp.client_name, "My App");
        assert_eq!(resp.client_type, "confidential");
        assert!(resp.broker_capability_enabled);
        assert_eq!(resp.client_secret, Some("secret_value".to_string()));
        assert!(resp.created_at.contains('T'));
    }

    #[test]
    fn to_response_omits_secret_when_none() {
        use chrono::Utc;
        let client = OauthClient {
            id: "client_2".to_string(),
            client_name: "Read App".to_string(),
            client_type: "public".to_string(),
            client_secret_hash: String::new(),
            redirect_uris: vec![],
            allowed_scopes: "openid".to_string(),
            grant_types: "authorization_code".to_string(),
            delegation_scopes: String::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            is_active: false,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let resp = to_response(client, None);
        assert!(resp.client_secret.is_none());
        assert!(!resp.is_active);
    }

    #[test]
    fn developer_oauth_client_response_serialization() {
        let resp = DeveloperOAuthClientResponse {
            id: "c1".to_string(),
            client_name: "App".to_string(),
            client_type: "public".to_string(),
            redirect_uris: vec!["https://x.com/cb".to_string()],
            allowed_scopes: "openid".to_string(),
            delegation_scopes: String::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            is_active: true,
            client_secret: None,
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "c1");
        assert!(json.get("client_secret").is_none());
    }

    #[test]
    fn developer_oauth_client_response_includes_secret_when_present() {
        let resp = DeveloperOAuthClientResponse {
            id: "c2".to_string(),
            client_name: "App".to_string(),
            client_type: "confidential".to_string(),
            redirect_uris: vec![],
            allowed_scopes: "openid".to_string(),
            delegation_scopes: String::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            is_active: true,
            client_secret: Some("secret_abc".to_string()),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["client_secret"], "secret_abc");
    }

    #[tokio::test]
    async fn get_nonexistent_client_returns_not_found() {
        let Some(db) = connect_test_database("h_dev_apps_not_found").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err =
            get_my_oauth_client(State(state), auth, Path(uuid::Uuid::new_v4().to_string())).await;

        assert!(err.is_err());
    }
}
