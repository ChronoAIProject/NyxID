use axum::{
    extract::{Path, State},
    Json,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::provider_config::{ProviderConfig, COLLECTION_NAME as PROVIDER_CONFIGS};
use crate::models::service_provider_requirement::{
    ServiceProviderRequirement, COLLECTION_NAME,
};
use crate::mw::auth::AuthUser;
use crate::services::audit_service;
use crate::AppState;

use super::services_helpers::{fetch_service, require_admin};

/// Headers that must not be used as injection keys for security reasons.
const BLOCKED_INJECTION_KEYS: &[&str] = &[
    "host",
    "authorization",
    "cookie",
    "set-cookie",
    "transfer-encoding",
    "content-length",
    "connection",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
];

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct AddRequirementRequest {
    pub provider_config_id: String,
    pub required: bool,
    pub scopes: Option<Vec<String>>,
    pub injection_method: String,
    pub injection_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RequirementResponse {
    pub id: String,
    pub service_id: String,
    pub provider_config_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub required: bool,
    pub scopes: Option<Vec<String>>,
    pub injection_method: String,
    pub injection_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct RequirementListResponse {
    pub requirements: Vec<RequirementResponse>,
}

#[derive(Debug, Serialize)]
pub struct DeleteRequirementResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/services/{service_id}/requirements
pub async fn list_requirements(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<RequirementListResponse>> {
    // Verify service exists
    let _service = fetch_service(&state, &service_id).await?;

    let requirements: Vec<ServiceProviderRequirement> = state
        .db
        .collection::<ServiceProviderRequirement>(COLLECTION_NAME)
        .find(doc! { "service_id": &service_id })
        .await?
        .try_collect()
        .await?;

    // Batch fetch all referenced providers in a single query (fix N+1)
    let provider_ids: Vec<&str> = requirements
        .iter()
        .map(|r| r.provider_config_id.as_str())
        .collect();
    let providers: Vec<ProviderConfig> = if provider_ids.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find(doc! { "_id": { "$in": &provider_ids } })
            .await?
            .try_collect()
            .await?
    };
    let provider_map: std::collections::HashMap<&str, &ProviderConfig> = providers
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let items: Vec<RequirementResponse> = requirements
        .into_iter()
        .map(|req| {
            let (provider_name, provider_slug) =
                match provider_map.get(req.provider_config_id.as_str()) {
                    Some(p) => (p.name.clone(), p.slug.clone()),
                    None => ("Unknown".to_string(), "unknown".to_string()),
                };
            RequirementResponse {
                id: req.id,
                service_id: req.service_id,
                provider_config_id: req.provider_config_id,
                provider_name,
                provider_slug,
                required: req.required,
                scopes: req.scopes,
                injection_method: req.injection_method,
                injection_key: req.injection_key,
                created_at: req.created_at.to_rfc3339(),
                updated_at: req.updated_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(RequirementListResponse { requirements: items }))
}

/// POST /api/v1/services/{service_id}/requirements
pub async fn add_requirement(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<AddRequirementRequest>,
) -> AppResult<Json<RequirementResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify service exists
    let _service = fetch_service(&state, &service_id).await?;

    // Verify provider exists
    let provider = state
        .db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": &body.provider_config_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found or inactive".to_string()))?;

    // Validate injection_method
    let valid_methods = ["bearer", "header", "query"];
    if !valid_methods.contains(&body.injection_method.as_str()) {
        return Err(AppError::ValidationError(format!(
            "injection_method must be one of: {}",
            valid_methods.join(", ")
        )));
    }

    // Validate injection_key against blocklist
    if let Some(ref key) = body.injection_key {
        let key_lower = key.to_lowercase();
        if BLOCKED_INJECTION_KEYS.contains(&key_lower.as_str()) {
            return Err(AppError::ValidationError(format!(
                "injection_key '{}' is blocked for security reasons",
                key
            )));
        }
    }

    // Check for duplicate requirement
    let existing = state
        .db
        .collection::<ServiceProviderRequirement>(COLLECTION_NAME)
        .find_one(doc! {
            "service_id": &service_id,
            "provider_config_id": &body.provider_config_id,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "This provider requirement already exists for this service".to_string(),
        ));
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let requirement = ServiceProviderRequirement {
        id: id.clone(),
        service_id: service_id.clone(),
        provider_config_id: body.provider_config_id.clone(),
        required: body.required,
        scopes: body.scopes,
        injection_method: body.injection_method,
        injection_key: body.injection_key,
        created_at: now,
        updated_at: now,
    };

    state
        .db
        .collection::<ServiceProviderRequirement>(COLLECTION_NAME)
        .insert_one(&requirement)
        .await?;

    tracing::info!(
        requirement_id = %id,
        service_id = %service_id,
        provider_id = %requirement.provider_config_id,
        "Service provider requirement added"
    );

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "service_requirement_added".to_string(),
        Some(serde_json::json!({
            "service_id": &service_id,
            "provider_id": &requirement.provider_config_id,
        })),
        None,
        None,
    );

    Ok(Json(RequirementResponse {
        id: requirement.id,
        service_id: requirement.service_id,
        provider_config_id: requirement.provider_config_id,
        provider_name: provider.name,
        provider_slug: provider.slug,
        required: requirement.required,
        scopes: requirement.scopes,
        injection_method: requirement.injection_method,
        injection_key: requirement.injection_key,
        created_at: requirement.created_at.to_rfc3339(),
        updated_at: requirement.updated_at.to_rfc3339(),
    }))
}

/// DELETE /api/v1/services/{service_id}/requirements/{requirement_id}
pub async fn remove_requirement(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, requirement_id)): Path<(String, String)>,
) -> AppResult<Json<DeleteRequirementResponse>> {
    require_admin(&state, &auth_user).await?;

    let result = state
        .db
        .collection::<ServiceProviderRequirement>(COLLECTION_NAME)
        .delete_one(doc! { "_id": &requirement_id, "service_id": &service_id })
        .await?;

    if result.deleted_count == 0 {
        return Err(AppError::NotFound(
            "Requirement not found".to_string(),
        ));
    }

    tracing::info!(
        requirement_id = %requirement_id,
        service_id = %service_id,
        "Service provider requirement removed"
    );

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "service_requirement_removed".to_string(),
        Some(serde_json::json!({
            "service_id": &service_id,
            "requirement_id": &requirement_id,
        })),
        None,
        None,
    );

    Ok(Json(DeleteRequirementResponse {
        message: "Requirement removed".to_string(),
    }))
}
