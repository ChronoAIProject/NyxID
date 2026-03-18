use mongodb::bson::doc;
use serde::Serialize;

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
        inject_delegation_token: s.inject_delegation_token,
        delegation_token_scope: s.delegation_token_scope,
        created_by: s.created_by,
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
    }
}

/// Validate that a base_url is safe to proxy to (not a private/internal address).
///
/// In development mode, private/internal addresses (localhost, 127.0.0.1, etc.)
/// are allowed so that locally-running downstream services can be registered.
pub fn validate_base_url(url: &str, allow_private: bool) -> AppResult<()> {
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

    // Skip private-address checks in development mode
    if allow_private {
        return Ok(());
    }

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

    // Block common private IP ranges (CR-2/SEC-3: includes IPv6 private ranges)
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let is_private = match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_loopback()
                    || ipv4.is_private()
                    || ipv4.is_link_local()
                    || ipv4.octets()[0] == 169 && ipv4.octets()[1] == 254
            }
            std::net::IpAddr::V6(ipv6) => {
                ipv6.is_loopback()
                    || (ipv6.segments()[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
                    || (ipv6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                    || ipv6.to_ipv4_mapped().is_some_and(|v4| v4.is_private() || v4.is_loopback())
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

/// Validate an optional documentation spec URL using the same SSRF rules as base_url.
pub fn validate_optional_spec_url(url: &str, allow_private: bool) -> AppResult<()> {
    if url.len() > 2048 {
        return Err(AppError::ValidationError(
            "Spec URL must not exceed 2048 characters".to_string(),
        ));
    }

    validate_base_url(url, allow_private)
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
#[derive(Debug, Serialize)]
pub struct DeleteServiceResponse {
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::{validate_base_url, validate_optional_spec_url};

    #[test]
    fn validate_optional_spec_url_accepts_public_https_url() {
        assert!(validate_optional_spec_url("https://example.com/openapi.json", false).is_ok());
    }

    #[test]
    fn validate_optional_spec_url_rejects_private_host() {
        assert!(validate_optional_spec_url("http://127.0.0.1/openapi.json", false).is_err());
    }

    #[test]
    fn validate_base_url_accepts_private_hosts_in_development() {
        assert!(validate_base_url("http://127.0.0.1:3000", true).is_ok());
    }
}
