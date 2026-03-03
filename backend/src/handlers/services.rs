use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::crypto::aes;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, oauth_client_service};

use super::services_helpers::{
    DeleteServiceResponse, fetch_service, require_admin, require_admin_or_creator,
    service_to_response, validate_base_url,
};

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub base_url: String,
    /// Accepts "auth_method" or "auth_type" from frontend
    #[serde(alias = "auth_type")]
    pub auth_method: Option<String>,
    pub auth_key_name: Option<String>,
    pub credential: Option<String>,
    /// "provider", "connection", or "internal". Defaults to "connection".
    pub service_category: Option<String>,
}

impl std::fmt::Debug for CreateServiceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateServiceRequest")
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("description", &self.description)
            .field("base_url", &self.base_url)
            .field("auth_method", &self.auth_method)
            .field("auth_key_name", &self.auth_key_name)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("service_category", &self.service_category)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_type: Option<String>,
    pub auth_key_name: String,
    pub is_active: bool,
    pub oauth_client_id: Option<String>,
    pub api_spec_url: Option<String>,
    pub service_category: String,
    pub requires_user_credential: bool,
    pub identity_propagation_mode: String,
    pub identity_include_user_id: bool,
    pub identity_include_email: bool,
    pub identity_include_name: bool,
    pub identity_jwt_audience: Option<String>,
    pub inject_delegation_token: bool,
    pub delegation_token_scope: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceListResponse {
    pub services: Vec<ServiceResponse>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServiceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub base_url: Option<String>,
    pub is_active: Option<bool>,
    pub api_spec_url: Option<String>,
    pub identity_propagation_mode: Option<String>,
    pub identity_include_user_id: Option<bool>,
    pub identity_include_email: Option<bool>,
    pub identity_include_name: Option<bool>,
    pub identity_jwt_audience: Option<String>,
    pub inject_delegation_token: Option<bool>,
    pub delegation_token_scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OidcCredentialsResponse {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: String,
    pub delegation_scopes: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRedirectUrisRequest {
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RedirectUrisResponse {
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RegenerateSecretResponse {
    pub client_secret: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ListServicesQuery {
    pub category: Option<String>,
}

// --- Handlers ---
// TODO(SEC-7): Credential endpoints (get_oidc_credentials, update_redirect_uris,
// regenerate_oidc_secret) should have stricter per-endpoint rate limiting (e.g.,
// 5 requests/minute) instead of sharing the global rate limiter. This requires
// a separate PerIpRateLimiter applied as middleware on these specific routes.

/// GET /api/v1/services
///
/// List all downstream services. Supports optional `?category=` filter.
pub async fn list_services(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Query(query): Query<ListServicesQuery>,
) -> AppResult<Json<ServiceListResponse>> {
    let mut filter = doc! { "is_active": true };
    if let Some(ref category) = query.category {
        let valid = ["provider", "connection", "internal"];
        if !valid.contains(&category.as_str()) {
            return Err(AppError::ValidationError(format!(
                "Invalid category filter: {category}. Must be one of: {}",
                valid.join(", ")
            )));
        }
        filter.insert("service_category", category.as_str());
    }

    let services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    let items: Vec<ServiceResponse> = services.into_iter().map(service_to_response).collect();

    Ok(Json(ServiceListResponse { services: items }))
}

/// POST /api/v1/services
///
/// Register a new downstream service. Requires admin privileges.
pub async fn create_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateServiceRequest>,
) -> AppResult<Json<ServiceResponse>> {
    require_admin(&state, &auth_user).await?;

    if body.name.is_empty() || body.base_url.is_empty() {
        return Err(AppError::ValidationError(
            "name and base_url are required".to_string(),
        ));
    }

    // CR-18: Validate input lengths before doing work based on input
    if body.name.len() > 200 || body.base_url.len() > 2048 {
        return Err(AppError::ValidationError(
            "Input exceeds maximum length".to_string(),
        ));
    }

    // Derive slug from name if not provided (CR-9: collapse consecutive hyphens)
    let slug = body.slug.clone().unwrap_or_else(|| {
        body.name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    });

    // CR-3: Validate slug is non-empty after derivation
    if slug.is_empty() {
        return Err(AppError::ValidationError(
            "Service name must contain at least one alphanumeric character".to_string(),
        ));
    }

    if slug.len() > 100 {
        return Err(AppError::ValidationError(
            "Slug exceeds maximum length of 100 characters".to_string(),
        ));
    }

    // Preserve the original auth_type value before mapping
    let auth_type_original = body.auth_method.clone();

    // Map frontend auth_type values to backend auth_method values
    let auth_method = match body.auth_method.as_deref() {
        Some("api_key") => "header".to_string(),
        Some("oauth2") | Some("bearer") => "bearer".to_string(),
        Some("basic") => "basic".to_string(),
        Some("none") => "none".to_string(),
        Some(other) => other.to_string(),
        None => "header".to_string(),
    };

    let auth_key_name = body
        .auth_key_name
        .clone()
        .unwrap_or_else(|| match auth_method.as_str() {
            "bearer" => "Authorization".to_string(),
            "basic" => "Authorization".to_string(),
            "query" => "api_key".to_string(),
            "none" => String::new(),
            _ => "X-API-Key".to_string(),
        });

    let credential = body.credential.clone().unwrap_or_default();

    let valid_methods = ["header", "bearer", "query", "basic", "oidc", "none"];
    if !valid_methods.contains(&auth_method.as_str()) {
        return Err(AppError::ValidationError(format!(
            "auth_method must be one of: {}",
            valid_methods.join(", ")
        )));
    }

    // Validate base_url against SSRF
    validate_base_url(&body.base_url, state.config.is_development())?;

    // Check slug uniqueness
    let existing = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": &slug })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "A service with this slug already exists".to_string(),
        ));
    }

    let user_id_str = auth_user.user_id.to_string();

    // For OIDC services, auto-provision an OAuth client
    let (encrypted_cred, oauth_client_id) = if auth_method == "oidc" {
        let callback_url = format!("{}/callback", body.base_url.trim_end_matches('/'));
        let client_name = format!("{} OIDC Client", body.name);
        let (client, raw_secret) = oauth_client_service::create_client(
            &state.db,
            &client_name,
            &[callback_url],
            "confidential",
            &user_id_str,
            "",
        )
        .await?;

        let secret_to_encrypt = raw_secret.unwrap_or_default();
        let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
        let enc = aes::encrypt(secret_to_encrypt.as_bytes(), &encryption_key)?;

        (enc, Some(client.id))
    } else {
        let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
        let enc = aes::encrypt(credential.as_bytes(), &encryption_key)?;
        (enc, None)
    };

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    // Derive service_category and requires_user_credential
    let service_category = if auth_method == "oidc" {
        // OIDC services are always providers
        "provider".to_string()
    } else if auth_method == "none" {
        // No-auth services are always internal (auto-connected)
        "internal".to_string()
    } else {
        match body.service_category.as_deref() {
            Some("provider") => {
                return Err(AppError::ValidationError(
                    "Only OIDC services can be categorized as provider".to_string(),
                ));
            }
            Some("internal") => "internal".to_string(),
            Some("connection") | None => "connection".to_string(),
            Some(other) => {
                return Err(AppError::ValidationError(format!(
                    "Invalid service_category: {other}. Must be provider, connection, or internal"
                )));
            }
        }
    };

    let requires_user_credential = service_category == "connection";

    let new_service = DownstreamService {
        id: id.clone(),
        name: body.name.clone(),
        slug: slug.clone(),
        description: body.description.clone(),
        base_url: body.base_url.clone(),
        auth_method: auth_method.clone(),
        auth_type: auth_type_original,
        auth_key_name: auth_key_name.clone(),
        credential_encrypted: encrypted_cred,
        api_spec_url: None,
        oauth_client_id: oauth_client_id.clone(),
        service_category,
        requires_user_credential,
        is_active: true,
        created_by: user_id_str.clone(),
        identity_propagation_mode: "none".to_string(),
        identity_include_user_id: false,
        identity_include_email: false,
        identity_include_name: false,
        identity_jwt_audience: None,
        inject_delegation_token: false,
        delegation_token_scope: "llm:proxy".to_string(),
        provider_config_id: None,
        created_at: now,
        updated_at: now,
    };

    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&new_service)
        .await?;

    tracing::info!(service_id = %id, name = %body.name, created_by = %auth_user.user_id, "Service created");

    // CR-1: Audit log for service creation
    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str.clone()),
        "service_created".to_string(),
        Some(serde_json::json!({ "service_id": &id, "name": &body.name })),
        None,
        None,
    );

    Ok(Json(service_to_response(new_service)))
}

/// DELETE /api/v1/services/:service_id
///
/// Deactivate a downstream service. Requires admin or service creator.
pub async fn delete_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<DeleteServiceResponse>> {
    // CR-4: Use shared require_admin_or_creator helper instead of inline check
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let now = Utc::now();
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // SEC-M2: Cascade deactivation - wipe all user credentials for this service
    use crate::models::user_service_connection::{
        COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
    };
    state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .update_many(
            doc! { "service_id": &service_id, "is_active": true },
            doc! { "$set": {
                "is_active": false,
                "credential_encrypted": bson::Bson::Null,
                "credential_type": bson::Bson::Null,
                "credential_label": bson::Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(service_id = %service_id, deactivated_by = %auth_user.user_id, "Service deactivated");

    // CR-1: Audit log for service deletion
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "service_deleted".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
    );

    // CR-16: Use typed response struct
    Ok(Json(DeleteServiceResponse {
        message: "Service deactivated".to_string(),
    }))
}

/// GET /api/v1/services/{service_id}
///
/// Get a single service by ID. Requires authentication.
pub async fn get_service(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<ServiceResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    Ok(Json(service_to_response(service)))
}

/// PUT /api/v1/services/{service_id}
///
/// Update a downstream service. Requires admin or original creator.
pub async fn update_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<UpdateServiceRequest>,
) -> AppResult<Json<ServiceResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    // Build the $set document with only provided fields
    let mut set_doc = doc! {};

    if let Some(ref name) = body.name {
        if name.is_empty() || name.len() > 200 {
            return Err(AppError::ValidationError(
                "name must be between 1 and 200 characters".to_string(),
            ));
        }
        set_doc.insert("name", name.as_str());
    }

    if let Some(ref description) = body.description {
        if description.len() > 500 {
            return Err(AppError::ValidationError(
                "description must not exceed 500 characters".to_string(),
            ));
        }
        set_doc.insert("description", description.as_str());
    }

    if let Some(ref base_url) = body.base_url {
        validate_base_url(base_url, state.config.is_development())?;
        if base_url.len() > 2048 {
            return Err(AppError::ValidationError(
                "base_url must not exceed 2048 characters".to_string(),
            ));
        }
        set_doc.insert("base_url", base_url.as_str());
    }

    if let Some(is_active) = body.is_active {
        set_doc.insert("is_active", is_active);
    }

    if let Some(ref api_spec_url) = body.api_spec_url {
        if api_spec_url.len() > 2048 {
            return Err(AppError::ValidationError(
                "api_spec_url must not exceed 2048 characters".to_string(),
            ));
        }
        set_doc.insert("api_spec_url", api_spec_url.as_str());
    }

    if let Some(ref mode) = body.identity_propagation_mode {
        let valid_modes = ["none", "headers", "jwt", "both"];
        if !valid_modes.contains(&mode.as_str()) {
            return Err(AppError::ValidationError(format!(
                "identity_propagation_mode must be one of: {}",
                valid_modes.join(", ")
            )));
        }
        set_doc.insert("identity_propagation_mode", mode.as_str());
    }
    if let Some(include_uid) = body.identity_include_user_id {
        set_doc.insert("identity_include_user_id", include_uid);
    }
    if let Some(include_email) = body.identity_include_email {
        set_doc.insert("identity_include_email", include_email);
    }
    if let Some(include_name) = body.identity_include_name {
        set_doc.insert("identity_include_name", include_name);
    }
    if let Some(ref audience) = body.identity_jwt_audience {
        if audience.len() > 2048 {
            return Err(AppError::ValidationError(
                "identity_jwt_audience must not exceed 2048 characters".to_string(),
            ));
        }
        set_doc.insert("identity_jwt_audience", audience.as_str());
    }
    if let Some(inject) = body.inject_delegation_token {
        set_doc.insert("inject_delegation_token", inject);
    }
    if let Some(ref scope) = body.delegation_token_scope {
        // H3: Default empty scope to "llm:proxy" so tokens always have permissions
        let scope = if scope.is_empty() {
            "llm:proxy"
        } else {
            scope.as_str()
        };

        // H2: Validate against known delegation scopes
        let valid_scopes = ["llm:proxy", "proxy:*", "llm:status"];
        for s in scope.split_whitespace() {
            if !valid_scopes.contains(&s) {
                return Err(AppError::ValidationError(format!(
                    "Invalid delegation_token_scope '{}'. Must be one of: {}",
                    s,
                    valid_scopes.join(", ")
                )));
            }
        }

        set_doc.insert("delegation_token_scope", scope);
    }

    if set_doc.is_empty() {
        return Err(AppError::ValidationError(
            "At least one field must be provided for update".to_string(),
        ));
    }

    let now = Utc::now();
    set_doc.insert("updated_at", bson::DateTime::from_chrono(now));

    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(doc! { "_id": &service_id }, doc! { "$set": &set_doc })
        .await?;

    // If base_url changed and service has an OIDC client, update default redirect URI
    if let (Some(new_base_url), Some(oauth_client_id)) = (&body.base_url, &service.oauth_client_id)
    {
        let new_callback = format!("{}/callback", new_base_url.trim_end_matches('/'));
        oauth_client_service::update_redirect_uris(&state.db, oauth_client_id, &[new_callback])
            .await?;
    }

    tracing::info!(service_id = %service_id, updated_by = %auth_user.user_id, "Service updated");

    // CR-1: Audit log for service update
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "service_updated".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
    );

    // Re-fetch the updated service to return fresh data
    let updated = fetch_service(&state, &service_id).await?;
    Ok(Json(service_to_response(updated)))
}

/// GET /api/v1/services/{service_id}/oidc-credentials
///
/// Retrieve OIDC client credentials. Admin only.
pub async fn get_oidc_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<OidcCredentialsResponse>> {
    require_admin(&state, &auth_user).await?;

    let service = fetch_service(&state, &service_id).await?;

    if service.auth_method != "oidc" {
        return Err(AppError::BadRequest(
            "Service is not an OIDC service".to_string(),
        ));
    }

    let oauth_client_id = service
        .oauth_client_id
        .ok_or_else(|| AppError::Internal("OIDC service missing oauth_client_id".to_string()))?;

    // Decrypt the client secret from credential_encrypted
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let decrypted_bytes = aes::decrypt(&service.credential_encrypted, &encryption_key)?;
    let client_secret = String::from_utf8(decrypted_bytes)
        .map_err(|e| AppError::Internal(format!("Failed to decode decrypted secret: {e}")))?;

    // Fetch the OAuth client for redirect URIs and scopes
    let oauth_client = oauth_client_service::get_client(&state.db, &oauth_client_id).await?;

    // CR-7: redirect_uris is now Vec<String> on the model, no deserialization needed
    let redirect_uris = oauth_client.redirect_uris;

    // Build OIDC discovery endpoints from config base_url
    let base = state.config.base_url.trim_end_matches('/');

    tracing::info!(
        service_id = %service_id,
        accessed_by = %auth_user.user_id,
        "OIDC credentials accessed"
    );

    // CR-1/SEC-4: Audit log for credential access
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "oidc_credentials_accessed".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
    );

    Ok(Json(OidcCredentialsResponse {
        client_id: oauth_client_id,
        client_secret,
        redirect_uris,
        allowed_scopes: oauth_client.allowed_scopes,
        delegation_scopes: oauth_client.delegation_scopes,
        issuer: state.config.jwt_issuer.clone(),
        authorization_endpoint: format!("{base}/oauth/authorize"),
        token_endpoint: format!("{base}/oauth/token"),
        userinfo_endpoint: format!("{base}/oauth/userinfo"),
        jwks_uri: format!("{base}/.well-known/jwks.json"),
    }))
}

/// PUT /api/v1/services/{service_id}/redirect-uris
///
/// Update redirect URIs for an OIDC service. Admin only.
pub async fn update_redirect_uris(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<UpdateRedirectUrisRequest>,
) -> AppResult<Json<RedirectUrisResponse>> {
    require_admin(&state, &auth_user).await?;

    let service = fetch_service(&state, &service_id).await?;

    if service.auth_method != "oidc" {
        return Err(AppError::BadRequest(
            "Service is not an OIDC service".to_string(),
        ));
    }

    let oauth_client_id = service
        .oauth_client_id
        .ok_or_else(|| AppError::Internal("OIDC service missing oauth_client_id".to_string()))?;

    if body.redirect_uris.is_empty() {
        return Err(AppError::ValidationError(
            "At least one redirect URI is required".to_string(),
        ));
    }

    // CR-13/SEC-8: Limit count and length of redirect URIs
    if body.redirect_uris.len() > 10 {
        return Err(AppError::ValidationError(
            "Maximum 10 redirect URIs allowed".to_string(),
        ));
    }

    // Validate each URI (SEC-1: restrict to http/https schemes)
    for uri in &body.redirect_uris {
        if uri.len() > 2048 {
            return Err(AppError::ValidationError(
                "Redirect URI exceeds max length of 2048 characters".to_string(),
            ));
        }
        let parsed = url::Url::parse(uri)
            .map_err(|_| AppError::ValidationError(format!("Invalid redirect URI: {uri}")))?;
        let scheme = parsed.scheme();
        if scheme != "https" && scheme != "http" {
            return Err(AppError::ValidationError(format!(
                "Redirect URI must use https or http scheme: {uri}"
            )));
        }
    }

    oauth_client_service::update_redirect_uris(&state.db, &oauth_client_id, &body.redirect_uris)
        .await?;

    // Touch updated_at on the service
    let now = Utc::now();
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id },
            doc! { "$set": { "updated_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;

    tracing::info!(
        service_id = %service_id,
        updated_by = %auth_user.user_id,
        "Redirect URIs updated"
    );

    // CR-1: Audit log for redirect URI update
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "redirect_uris_updated".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
    );

    Ok(Json(RedirectUrisResponse {
        redirect_uris: body.redirect_uris,
    }))
}

/// POST /api/v1/services/{service_id}/regenerate-secret
///
/// Regenerate the OIDC client secret. Admin only.
pub async fn regenerate_oidc_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<RegenerateSecretResponse>> {
    require_admin(&state, &auth_user).await?;

    let service = fetch_service(&state, &service_id).await?;

    if service.auth_method != "oidc" {
        return Err(AppError::BadRequest(
            "Service is not an OIDC service".to_string(),
        ));
    }

    let oauth_client_id = service
        .oauth_client_id
        .ok_or_else(|| AppError::Internal("OIDC service missing oauth_client_id".to_string()))?;

    // Generate a new secret
    let new_secret = generate_random_token();
    let new_hash = hash_token(&new_secret);

    // SEC-5: These two updates are not wrapped in a MongoDB transaction. If the
    // server crashes after the first update but before the second, the system will
    // be in an inconsistent state (new hash stored but old encrypted secret remains).
    // A MongoDB multi-document transaction requires a replica set or sharded cluster.
    // TODO: Use a MongoDB session with start_transaction/commit_transaction when
    // running on a replica set.

    // Update the OauthClient with the new hash
    let now = Utc::now();
    state
        .db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": &oauth_client_id },
            doc! { "$set": {
                "client_secret_hash": &new_hash,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // Encrypt the new secret and update credential_encrypted on the service
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let encrypted = aes::encrypt(new_secret.as_bytes(), &encryption_key)?;

    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id },
            doc! { "$set": {
                "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted },
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(
        service_id = %service_id,
        regenerated_by = %auth_user.user_id,
        "OIDC client secret regenerated"
    );

    // CR-1/SEC-4: Audit log for secret regeneration
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "oidc_secret_regenerated".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
    );

    Ok(Json(RegenerateSecretResponse {
        client_secret: new_secret,
        message: "Previous secret is now invalidated. Store this secret securely.".to_string(),
    }))
}
