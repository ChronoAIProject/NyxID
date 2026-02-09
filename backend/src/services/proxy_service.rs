use mongodb::bson::doc;
use reqwest::Client;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES};
use crate::models::user_service_connection::{UserServiceConnection, COLLECTION_NAME as USER_SERVICE_CONNECTIONS};

/// Result of resolving a proxy target.
pub struct ProxyTarget {
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub credential: String,
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
/// Checks for a per-user credential first, falling back to the
/// service-level master credential.
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

    // Check for per-user credential override
    let user_conn = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "is_active": true,
        })
        .await?;

    let credential_encrypted = match user_conn.and_then(|c| c.credential_encrypted) {
        Some(user_cred) => user_cred,
        None => service.credential_encrypted,
    };

    let credential = String::from_utf8(
        aes::decrypt(&credential_encrypted, encryption_key)?
    )
    .map_err(|e| AppError::Internal(format!("Credential is not valid UTF-8: {e}")))?;

    Ok(ProxyTarget {
        base_url: service.base_url,
        auth_method: service.auth_method,
        auth_key_name: service.auth_key_name,
        credential,
    })
}

/// Forward a request to the downstream service with credential injection.
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
) -> AppResult<reqwest::Response> {
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

    if let Some(body_bytes) = body {
        request = request.body(body_bytes);
    }

    let response = request
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Proxy request failed: {e}")))?;

    Ok(response)
}
