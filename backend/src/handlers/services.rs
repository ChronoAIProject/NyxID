use axum::{
    extract::{Path, State},
    Json,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::models::downstream_service::{DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::AppState;

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub credential: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceListResponse {
    pub services: Vec<ServiceResponse>,
}

// --- Helpers ---

/// Verify that the authenticated user has admin privileges.
async fn require_admin(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin {
        return Err(AppError::Forbidden(
            "Admin access required".to_string(),
        ));
    }

    Ok(())
}

/// Validate that a base_url is safe to proxy to (not a private/internal address).
fn validate_base_url(url: &str) -> AppResult<()> {
    // Must start with https:// or http://
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(AppError::ValidationError(
            "base_url must start with https:// or http://".to_string(),
        ));
    }

    // Parse the URL to extract the hostname
    let parsed = url::Url::parse(url).map_err(|_| {
        AppError::ValidationError("Invalid base_url format".to_string())
    })?;

    let host = parsed.host_str().ok_or_else(|| {
        AppError::ValidationError("base_url must contain a hostname".to_string())
    })?;

    // Block private/reserved hostnames
    let blocked_hosts = [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "[::1]",
        "metadata.google.internal",
    ];
    let host_lower = host.to_lowercase();
    for blocked in &blocked_hosts {
        if host_lower == *blocked {
            return Err(AppError::ValidationError(
                "base_url must not point to a private or internal address".to_string(),
            ));
        }
    }

    // Block common private IP ranges
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let is_private = match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_loopback()
                    || ipv4.is_private()
                    || ipv4.is_link_local()
                    || ipv4.octets()[0] == 169 && ipv4.octets()[1] == 254 // link-local
            }
            std::net::IpAddr::V6(ipv6) => {
                ipv6.is_loopback()
            }
        };

        if is_private {
            return Err(AppError::ValidationError(
                "base_url must not point to a private or internal IP address".to_string(),
            ));
        }
    }

    Ok(())
}

// --- Handlers ---

/// GET /api/v1/services
///
/// List all downstream services. Requires authentication.
pub async fn list_services(
    State(state): State<AppState>,
    _auth_user: AuthUser,
) -> AppResult<Json<ServiceListResponse>> {
    let services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! { "is_active": true })
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    let items: Vec<ServiceResponse> = services
        .into_iter()
        .map(|s| ServiceResponse {
            id: s.id,
            name: s.name,
            slug: s.slug,
            description: s.description,
            base_url: s.base_url,
            auth_method: s.auth_method,
            auth_key_name: s.auth_key_name,
            is_active: s.is_active,
            created_at: s.created_at.to_rfc3339(),
        })
        .collect();

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
    // Require admin to create services
    require_admin(&state, &auth_user).await?;

    if body.name.is_empty() || body.slug.is_empty() || body.base_url.is_empty() {
        return Err(AppError::ValidationError(
            "name, slug, and base_url are required".to_string(),
        ));
    }

    // Validate input lengths
    if body.name.len() > 200 || body.slug.len() > 100 || body.base_url.len() > 2048 {
        return Err(AppError::ValidationError(
            "Input exceeds maximum length".to_string(),
        ));
    }

    let valid_methods = ["header", "bearer", "query", "basic"];
    if !valid_methods.contains(&body.auth_method.as_str()) {
        return Err(AppError::ValidationError(format!(
            "auth_method must be one of: {}",
            valid_methods.join(", ")
        )));
    }

    // Validate base_url against SSRF
    validate_base_url(&body.base_url)?;

    // Check slug uniqueness
    let existing = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": &body.slug })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "A service with this slug already exists".to_string(),
        ));
    }

    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let encrypted_cred = aes::encrypt(body.credential.as_bytes(), &encryption_key)?;

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let new_service = DownstreamService {
        id: id.clone(),
        name: body.name.clone(),
        slug: body.slug.clone(),
        description: body.description.clone(),
        base_url: body.base_url.clone(),
        auth_method: body.auth_method.clone(),
        auth_key_name: body.auth_key_name.clone(),
        credential_encrypted: encrypted_cred,
        is_active: true,
        created_by: auth_user.user_id.to_string(),
        created_at: now,
        updated_at: now,
    };

    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&new_service)
        .await?;

    tracing::info!(service_id = %id, name = %body.name, created_by = %auth_user.user_id, "Service created");

    Ok(Json(ServiceResponse {
        id,
        name: body.name,
        slug: body.slug,
        description: body.description,
        base_url: body.base_url,
        auth_method: body.auth_method,
        auth_key_name: body.auth_key_name,
        is_active: true,
        created_at: now.to_rfc3339(),
    }))
}

/// DELETE /api/v1/services/:service_id
///
/// Deactivate a downstream service. Requires admin or service creator.
pub async fn delete_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let service_id_str = service_id.clone();

    let service = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": &service_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    // Require admin or original creator
    let user_id_str = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin && service.created_by != user_id_str {
        return Err(AppError::Forbidden(
            "Only admins or the service creator can deactivate services".to_string(),
        ));
    }

    let now = Utc::now();
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id_str },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(service_id = %service_id, deactivated_by = %auth_user.user_id, "Service deactivated");

    Ok(Json(serde_json::json!({ "message": "Service deactivated" })))
}
