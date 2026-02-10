use mongodb::bson::doc;
use reqwest::Client;
use zeroize::Zeroizing;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES};
use crate::models::user_service_connection::{UserServiceConnection, COLLECTION_NAME as USER_SERVICE_CONNECTIONS};
use crate::services::delegation_service::DelegatedCredential;

/// Result of resolving a proxy target.
pub struct ProxyTarget {
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub credential: String,
    pub service: DownstreamService,
}

/// Headers that are safe to forward to downstream services.
/// Uses an allowlist approach to prevent leaking sensitive headers.
const ALLOWED_FORWARD_HEADERS: &[&str] = &[
    "content-type",
    "accept",
    "accept-language",
    "accept-encoding",
    "content-length",
    "user-agent",
    "x-request-id",
    "x-correlation-id",
];

/// Resolve the downstream service and credential for a proxy request.
///
/// Enforces that the user has an active connection. For "connection" services,
/// uses the per-user credential. For "internal" services, uses the master credential.
/// Provider services are not proxyable.
pub async fn resolve_proxy_target(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    service_id: &str,
) -> AppResult<ProxyTarget> {
    // Load the downstream service
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Downstream service not found".to_string()))?;

    if !service.is_active {
        return Err(AppError::BadRequest("Service is inactive".to_string()));
    }

    // Provider services cannot be proxied to
    if service.service_category == "provider" {
        return Err(AppError::BadRequest(
            "Provider services are not proxyable".to_string(),
        ));
    }

    // Require an active user connection
    let user_conn = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "is_active": true,
        })
        .await?
        .ok_or_else(|| {
            AppError::Forbidden(
                "You must connect to this service before making requests".to_string(),
            )
        })?;

    // Determine which credential to use
    let credential_encrypted = if service.requires_user_credential {
        // Connection services: must have per-user credential
        user_conn.credential_encrypted.ok_or_else(|| {
            AppError::BadRequest(
                "Connection is missing credential. Please reconnect with your API key.".to_string(),
            )
        })?
    } else {
        // Internal services: use master credential
        service.credential_encrypted.clone()
    };

    // SEC-M3: Wrap raw decrypted bytes in Zeroizing so they are zeroed on drop
    let decrypted_bytes = Zeroizing::new(aes::decrypt(&credential_encrypted, encryption_key)?);
    let credential = String::from_utf8((*decrypted_bytes).clone())
        .map_err(|e| {
            tracing::error!("Credential UTF-8 decode failed: {e}");
            AppError::Internal("Failed to decode credential".to_string())
        })?;

    Ok(ProxyTarget {
        base_url: service.base_url.clone(),
        auth_method: service.auth_method.clone(),
        auth_key_name: service.auth_key_name.clone(),
        credential,
        service,
    })
}

/// Forward a request to the downstream service with credential injection,
/// identity propagation headers, and delegated provider credentials.
///
/// Uses an allowlist for headers to prevent leaking sensitive data.
/// Preserves the original HTTP method for all auth methods including query auth.
pub async fn forward_request(
    client: &Client,
    target: &ProxyTarget,
    method: reqwest::Method,
    path: &str,
    query: Option<&str>,
    headers: reqwest::header::HeaderMap,
    body: Option<bytes::Bytes>,
    identity_headers: Vec<(String, String)>,
    delegated_credentials: Vec<DelegatedCredential>,
) -> AppResult<reqwest::Response> {
    // SEC-H3: Reject paths containing traversal sequences
    if path.contains("..") || path.contains("//") {
        return Err(AppError::BadRequest("Invalid proxy path".to_string()));
    }

    // TODO(SEC-H1): Re-validate the resolved IP at proxy time to prevent DNS rebinding.
    // Currently base_url is only validated at service creation/update time. An attacker
    // could change DNS to point to a private IP after validation. Consider using a custom
    // DNS resolver or reqwest's `resolve` feature to check the resolved IP before connecting.

    let url = if let Some(q) = query {
        format!("{}/{}?{}", target.base_url.trim_end_matches('/'), path.trim_start_matches('/'), q)
    } else {
        format!("{}/{}", target.base_url.trim_end_matches('/'), path.trim_start_matches('/'))
    };

    let mut request = client.request(method.clone(), &url);

    // Copy only allowed headers (allowlist approach)
    for (name, value) in headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if ALLOWED_FORWARD_HEADERS.contains(&name_lower.as_str()) {
            request = request.header(name, value);
        }
    }

    // Inject identity propagation headers
    for (name, value) in &identity_headers {
        request = request.header(name, value);
    }

    // Inject credentials based on auth method
    match target.auth_method.as_str() {
        "header" => {
            request = request.header(&target.auth_key_name, &target.credential);
        }
        "bearer" => {
            request = request.bearer_auth(&target.credential);
        }
        "query" => {
            // Use the request builder's query method to properly URL-encode parameters.
            // This preserves the original HTTP method, headers, and body.
            request = request.query(&[(&target.auth_key_name, &target.credential)]);
        }
        "basic" => {
            // credential format: "username:password"
            let parts: Vec<&str> = target.credential.splitn(2, ':').collect();
            if parts.len() == 2 {
                request = request.basic_auth(parts[0], Some(parts[1]));
            } else {
                return Err(AppError::Internal(
                    "Basic auth credential must be in 'username:password' format".to_string(),
                ));
            }
        }
        _ => {
            return Err(AppError::Internal(format!(
                "Unknown auth method: {}",
                target.auth_method
            )));
        }
    }

    // Inject delegated provider credentials
    for cred in &delegated_credentials {
        match cred.injection_method.as_str() {
            "bearer" => {
                request = request.header(
                    &cred.injection_key,
                    format!("Bearer {}", cred.credential),
                );
            }
            "header" => {
                request = request.header(&cred.injection_key, &cred.credential);
            }
            "query" => {
                request = request.query(&[(&cred.injection_key, &cred.credential)]);
            }
            _ => {}
        }
    }

    if let Some(body_bytes) = body {
        request = request.body(body_bytes);
    }

    let response = request
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Proxy request to {} failed: {e}", target.base_url);
            AppError::Internal("Proxy request failed".to_string())
        })?;

    Ok(response)
}
