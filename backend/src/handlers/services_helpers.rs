use mongodb::bson::doc;
use serde::Serialize;
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
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
        created_by: s.created_by,
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
    }
}

/// Validate that a URL has a valid scheme and hostname.
///
/// Cloud metadata endpoints (169.254.169.254, metadata.google.internal)
/// are blocked in every environment.  Private IPs and localhost are
/// allowed so that self-hosted nodes and services remain reachable.
pub fn validate_base_url(url: &str) -> AppResult<()> {
    // Must start with https:// or http://
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(AppError::ValidationError(
            "base_url must start with https:// or http://".to_string(),
        ));
    }

    // Parse the URL to extract the hostname
    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::ValidationError("Invalid base_url format".to_string()))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::ValidationError("base_url must contain a hostname".to_string()))?;

    // Block cloud metadata endpoints -- dangerous in any environment
    if is_cloud_metadata_host(host) {
        return Err(AppError::ValidationError(
            "URL must not point to a cloud metadata endpoint".to_string(),
        ));
    }

    Ok(())
}

/// Returns true if the hostname is a known cloud metadata endpoint.
fn is_cloud_metadata_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    normalized == "metadata.google.internal"
        || normalized == "169.254.169.254"
        || normalized == "[fd00:ec2::254]"
}

/// Validate an optional documentation spec URL.
pub fn validate_optional_spec_url(url: &str) -> AppResult<()> {
    if url.len() > 2048 {
        return Err(AppError::ValidationError(
            "Spec URL must not exceed 2048 characters".to_string(),
        ));
    }

    validate_base_url(url)
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

#[cfg(test)]
mod tests {
    use super::{validate_base_url, validate_optional_spec_url};

    #[test]
    fn validate_base_url_accepts_public_url() {
        assert!(validate_base_url("https://api.example.com").is_ok());
        assert!(validate_base_url("http://api.example.com").is_ok());
    }

    #[test]
    fn validate_base_url_accepts_private_ips() {
        assert!(validate_base_url("http://localhost:3000").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_base_url("http://192.168.1.50:3000").is_ok());
        assert!(validate_base_url("http://10.0.0.5:8080").is_ok());
        assert!(validate_base_url("http://100.64.0.10:3000").is_ok());
        assert!(validate_base_url("http://172.16.0.1:3000").is_ok());
    }

    #[test]
    fn validate_base_url_rejects_cloud_metadata() {
        assert!(validate_base_url("http://metadata.google.internal").is_err());
        assert!(validate_base_url("http://169.254.169.254").is_err());
    }

    #[test]
    fn validate_base_url_rejects_invalid_scheme() {
        assert!(validate_base_url("ftp://example.com").is_err());
        assert!(validate_base_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn validate_optional_spec_url_accepts_public_https_url() {
        assert!(validate_optional_spec_url("https://example.com/openapi.json").is_ok());
    }

    #[test]
    fn validate_optional_spec_url_rejects_metadata() {
        assert!(validate_optional_spec_url("http://169.254.169.254/latest").is_err());
    }
}
