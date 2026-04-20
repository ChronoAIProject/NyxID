use mongodb::bson::doc;
use serde::Serialize;
use utoipa::ToSchema;

use futures::TryStreamExt;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;

use super::services::{ServiceResponse, SshServiceConfigResponse};

/// Verify that the authenticated user has admin privileges.
pub async fn require_admin(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let user_id = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    Ok(())
}

/// Verify admin or service creator.
pub async fn require_admin_or_creator(
    state: &AppState,
    auth_user: &AuthUser,
    service_created_by: &str,
) -> AppResult<()> {
    let user_id_str = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if !user_model.is_admin && service_created_by != user_id_str {
        return Err(AppError::Forbidden(
            "Only admins or the service creator can perform this action".to_string(),
        ));
    }

    Ok(())
}

/// Fetch a service by ID or return NotFound.
pub async fn fetch_service(state: &AppState, service_id: &str) -> AppResult<DownstreamService> {
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))
}

/// Build a `ServiceResponse` from a `DownstreamService` model.
pub fn service_to_response(s: DownstreamService) -> ServiceResponse {
    ServiceResponse {
        id: s.id,
        name: s.name,
        slug: s.slug,
        description: s.description,
        base_url: s.base_url,
        service_type: s.service_type,
        visibility: s.visibility,
        auth_method: s.auth_method,
        auth_type: s.auth_type,
        auth_key_name: s.auth_key_name,
        is_active: s.is_active,
        oauth_client_id: s.oauth_client_id,
        openapi_spec_url: s.openapi_spec_url.clone(),
        api_spec_url: s.openapi_spec_url,
        asyncapi_spec_url: s.asyncapi_spec_url,
        streaming_supported: s.streaming_supported,
        ssh_config: s.ssh_config.map(|ssh| SshServiceConfigResponse {
            host: ssh.host,
            port: ssh.port,
            certificate_auth_enabled: ssh.certificate_auth_enabled,
            certificate_ttl_minutes: ssh.certificate_ttl_minutes,
            allowed_principals: ssh.allowed_principals,
            ca_public_key: ssh.ca_public_key,
        }),
        service_category: s.service_category,
        requires_user_credential: s.requires_user_credential,
        identity_propagation_mode: s.identity_propagation_mode,
        identity_include_user_id: s.identity_include_user_id,
        identity_include_email: s.identity_include_email,
        identity_include_name: s.identity_include_name,
        identity_jwt_audience: s.identity_jwt_audience,
        forward_access_token: s.forward_access_token,
        inject_delegation_token: s.inject_delegation_token,
        delegation_token_scope: s.delegation_token_scope,
        homepage_url: s.homepage_url,
        repository_url: s.repository_url,
        issues_url: s.issues_url,
        capabilities: s.capabilities,
        auth_notes: s.auth_notes,
        known_limitations: s.known_limitations,
        required_permissions: s.required_permissions,
        examples_url: s.examples_url,
        recommended_skills: s.recommended_skills,
        custom_user_agent: s.custom_user_agent,
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            s.default_request_headers,
        ),
        developer_app_ids: s.developer_app_ids,
        created_by: s.created_by,
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
    }
}

/// Validate that `developer_app_ids` reference active OAuth clients.
///
/// Any mutation of `developer_app_ids` is admin-only because it affects
/// cross-user auto-provisioning -- both setting and clearing the field.
/// Each referenced OAuth client must exist and be active; deleted or
/// unknown IDs are rejected. An empty list is a valid admin-authorized
/// clear operation.
pub async fn validate_developer_app_ids(
    state: &AppState,
    auth_user: &AuthUser,
    app_ids: &[String],
) -> AppResult<()> {
    // Any mutation (set or clear) requires admin -- clearing also has
    // cross-user impact (stops auto-provisioning for consented users).
    require_admin(state, auth_user).await?;

    if app_ids.is_empty() {
        return Ok(());
    }

    if app_ids.len() > 50 {
        return Err(AppError::ValidationError(
            "developer_app_ids must not exceed 50 entries".to_string(),
        ));
    }

    let id_refs: Vec<&str> = app_ids.iter().map(|s| s.as_str()).collect();
    let active_clients: Vec<OauthClient> = state
        .db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find(doc! {
            "_id": { "$in": &id_refs },
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;

    let found_ids: std::collections::HashSet<&str> =
        active_clients.iter().map(|c| c.id.as_str()).collect();

    for id in app_ids {
        if !found_ids.contains(id.as_str()) {
            return Err(AppError::ValidationError(format!(
                "developer_app_ids references unknown or inactive OAuth client: {id}"
            )));
        }
    }

    Ok(())
}

pub fn require_http_service(service: &DownstreamService) -> AppResult<()> {
    if service.service_type != "http" {
        return Err(AppError::BadRequest(
            "This operation is only supported for HTTP services".to_string(),
        ));
    }

    Ok(())
}

/// Typed response for delete operations (CR-16).
#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteServiceResponse {
    pub message: String,
}
