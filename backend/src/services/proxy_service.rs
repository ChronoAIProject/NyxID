use mongodb::bson::doc;
use reqwest::Client;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
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
    // content-length intentionally excluded: reqwest recalculates it from the
    // actual body, and forwarding the original value causes mismatches when
    // middleware or translators modify the request body.
    "user-agent",
    "x-request-id",
    "x-correlation-id",
];

/// Resolve a downstream service by its slug.
/// Returns the service document or NotFound.
pub async fn resolve_service_by_slug(
    db: &mongodb::Database,
    slug: &str,
) -> AppResult<DownstreamService> {
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))
}

/// Resolve the downstream service and credential for a proxy request.
///
/// Enforces that the user has an active connection. For "connection" services,
/// uses the per-user credential. For "internal" services, uses the master credential.
/// Provider services are not proxyable.
pub async fn resolve_proxy_target(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
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

    if service.service_type != "http" {
        return Err(AppError::BadRequest(
            "SSH services are not available through the HTTP proxy".to_string(),
        ));
    }

    // Provider services cannot be proxied to
    if service.service_category == "provider" {
        return Err(AppError::BadRequest(
            "Provider services are not proxyable".to_string(),
        ));
    }

    // Check for user connection (required for credential services, optional for auto-connect)
    let user_conn = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
        })
        .await?;

    // If user has explicitly disconnected (is_active: false), block access
    if let Some(ref conn) = user_conn
        && !conn.is_active
    {
        return Err(AppError::Forbidden(
            "You have disconnected from this service".to_string(),
        ));
    }

    // For services requiring user credentials, a connection record is mandatory
    if service.requires_user_credential && user_conn.is_none() {
        return Err(AppError::Forbidden(
            "You must connect to this service before making requests".to_string(),
        ));
    }

    // No-auth services: skip credential handling entirely
    if service.auth_method == "none" {
        return Ok(ProxyTarget {
            base_url: service.base_url.clone(),
            auth_method: service.auth_method.clone(),
            auth_key_name: service.auth_key_name.clone(),
            credential: String::new(),
            service,
        });
    }

    // Determine which credential to use
    let credential_encrypted = if service.requires_user_credential {
        // Connection services: must have per-user credential
        user_conn
            .and_then(|c| c.credential_encrypted)
            .ok_or_else(|| {
                AppError::BadRequest(
                    "Connection is missing credential. Please reconnect with your API key."
                        .to_string(),
                )
            })?
    } else {
        // Internal services: use master credential
        service.credential_encrypted.clone()
    };

    // SEC-M3: Wrap raw decrypted bytes in Zeroizing so they are zeroed on drop
    let decrypted_bytes = Zeroizing::new(encryption_keys.decrypt(&credential_encrypted).await?);
    let credential = String::from_utf8((*decrypted_bytes).clone()).map_err(|e| {
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

/// Resolve proxy target with lenient credential handling for node-routed requests.
///
/// Unlike `resolve_proxy_target()`, this does NOT require a connection record or
/// credential for "connection" services. Returns `(ProxyTarget, has_credential)`
/// where `has_credential` indicates whether a server-side credential was resolved
/// (i.e. standard proxy fallback is viable).
pub async fn resolve_proxy_target_lenient(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service_id: &str,
) -> AppResult<(ProxyTarget, bool)> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Downstream service not found".to_string()))?;

    if !service.is_active {
        return Err(AppError::BadRequest("Service is inactive".to_string()));
    }

    if service.service_type != "http" {
        return Err(AppError::BadRequest(
            "SSH services are not available through the HTTP proxy".to_string(),
        ));
    }

    if service.service_category == "provider" {
        return Err(AppError::BadRequest(
            "Provider services are not proxyable".to_string(),
        ));
    }

    // No-auth services: no credential needed
    if service.auth_method == "none" {
        return Ok((
            ProxyTarget {
                base_url: service.base_url.clone(),
                auth_method: service.auth_method.clone(),
                auth_key_name: service.auth_key_name.clone(),
                credential: String::new(),
                service,
            },
            true,
        ));
    }

    // Try to resolve a credential, but don't fail if missing
    let user_conn = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
        })
        .await?;

    // Respect explicit disconnection even in lenient mode
    if let Some(ref conn) = user_conn
        && !conn.is_active
    {
        return Err(AppError::Forbidden(
            "You have disconnected from this service".to_string(),
        ));
    }

    let credential_encrypted = if service.requires_user_credential {
        user_conn.and_then(|c| c.credential_encrypted)
    } else {
        Some(service.credential_encrypted.clone())
    };

    let (credential, has_credential) = match credential_encrypted {
        Some(enc) => {
            let decrypted_bytes = Zeroizing::new(encryption_keys.decrypt(&enc).await?);
            let cred = String::from_utf8((*decrypted_bytes).clone()).map_err(|e| {
                tracing::error!("Credential UTF-8 decode failed: {e}");
                AppError::Internal("Failed to decode credential".to_string())
            })?;
            (cred, true)
        }
        None => (String::new(), false),
    };

    Ok((
        ProxyTarget {
            base_url: service.base_url.clone(),
            auth_method: service.auth_method.clone(),
            auth_key_name: service.auth_key_name.clone(),
            credential,
            service,
        },
        has_credential,
    ))
}

/// Forward a request to the downstream service with credential injection,
/// identity propagation headers, and delegated provider credentials.
///
/// Uses an allowlist for headers to prevent leaking sensitive data.
/// Preserves the original HTTP method for all auth methods including query auth.
#[allow(clippy::too_many_arguments)]
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
        format!(
            "{}/{}?{}",
            target.base_url.trim_end_matches('/'),
            path.trim_start_matches('/'),
            q
        )
    } else {
        format!(
            "{}/{}",
            target.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
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
        "none" => {
            // No credential injection
        }
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
                request =
                    request.header(&cred.injection_key, format!("Bearer {}", cred.credential));
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

    if let Some(ref body_bytes) = body {
        // Log request body for LLM proxy calls to diagnose truncation issues
        if url.contains("/responses") {
            let body_str = String::from_utf8_lossy(body_bytes);
            let preview = if body_str.len() > 2048 {
                // Find a safe char boundary at or before 2048 bytes
                let mut end = 2048;
                while end > 0 && !body_str.is_char_boundary(end) {
                    end -= 1;
                }
                format!(
                    "{}...(truncated, total {} bytes)",
                    &body_str[..end],
                    body_str.len()
                )
            } else {
                body_str.to_string()
            };
            tracing::info!(
                url = %url,
                body_len = body_bytes.len(),
                body = %preview,
                "Proxy LLM request body"
            );
        }
    }

    if let Some(body_bytes) = body {
        request = request.body(body_bytes);
    }

    let response = request.send().await.map_err(|e| {
        tracing::error!("Proxy request to {} failed: {e}", target.base_url);
        AppError::Internal("Proxy request failed".to_string())
    })?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use chrono::Utc;
    use tokio::{net::TcpListener, sync::mpsc};

    #[derive(Debug)]
    struct CapturedRequest {
        content_type: Option<String>,
        body: Vec<u8>,
    }

    async fn capture_request(
        State(sender): State<mpsc::UnboundedSender<CapturedRequest>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        let _ = sender.send(CapturedRequest {
            content_type: headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(ToString::to_string),
            body: body.to_vec(),
        });

        StatusCode::OK
    }

    fn make_proxy_target(base_url: String) -> ProxyTarget {
        let now = Utc::now();
        ProxyTarget {
            base_url: base_url.clone(),
            auth_method: "none".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential: String::new(),
            service: DownstreamService {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Upload Service".to_string(),
                slug: "upload-service".to_string(),
                description: Some("Receives binary uploads".to_string()),
                base_url,
                service_type: "http".to_string(),
                visibility: "public".to_string(),
                auth_method: "none".to_string(),
                auth_key_name: "Authorization".to_string(),
                credential_encrypted: vec![],
                auth_type: None,
                openapi_spec_url: None,
                asyncapi_spec_url: None,
                streaming_supported: false,
                ssh_config: None,
                oauth_client_id: None,
                service_category: "internal".to_string(),
                requires_user_credential: false,
                is_active: true,
                created_by: "test".to_string(),
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
            },
        }
    }

    #[tokio::test]
    async fn forward_request_preserves_binary_body_and_content_type() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let app = Router::new()
            .route("/upload", post(capture_request))
            .with_state(sender);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test app");
        });

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/zip".parse().unwrap(),
        );
        let response = forward_request(
            &Client::new(),
            &make_proxy_target(format!("http://{addr}")),
            reqwest::Method::POST,
            "upload",
            None,
            headers,
            Some(bytes::Bytes::from_static(b"PK\x03\x04")),
            vec![],
            vec![],
        )
        .await
        .expect("proxy request should succeed");

        assert_eq!(response.status(), StatusCode::OK);

        let captured = receiver.recv().await.expect("captured request");
        assert_eq!(captured.content_type.as_deref(), Some("application/zip"));
        assert_eq!(captured.body, b"PK\x03\x04");

        server.abort();
    }
}
