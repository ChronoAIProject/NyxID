use std::sync::Arc;

use mongodb::bson::doc;
use reqwest::Client;
use url::form_urlencoded;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::services::delegation_service::DelegatedCredential;
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::provider_token_exchange_service::{self, TokenExchangeCache};
use crate::services::{
    agent_binding_service, user_api_key_service, user_service_service, user_token_service,
};

/// Request body for proxy forwarding.
pub enum ProxyBody {
    /// Body has been buffered in memory (approval path, node proxy, Codex path).
    Buffered(Option<bytes::Bytes>),
}

/// Result of resolving a proxy target.
pub struct ProxyTarget {
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub credential: String,
    pub service: DownstreamService,
}

pub(crate) struct PreparedDelegatedRequest {
    pub path: String,
    pub query: Option<String>,
    pub delegated_headers: Vec<(String, String)>,
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
    "range",
    "if-range",
    "if-none-match",
    "if-modified-since",
    // OpenClaw gateway session and routing headers
    "x-openclaw-session-key",
    "x-openclaw-agent-id",
    "x-openclaw-model",
    "x-openclaw-message-channel",
];

fn validate_path_injection_prefix(value: &str) -> AppResult<()> {
    if value.trim().is_empty()
        || value.chars().any(char::is_whitespace)
        || value.contains('/')
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.contains('\0')
        || value.contains("..")
        || value.contains('%')
    {
        return Err(AppError::BadRequest(
            "Service requirement is misconfigured for path injection. Please contact your admin."
                .to_string(),
        ));
    }

    Ok(())
}

fn validate_path_injection_credential(value: &str) -> AppResult<()> {
    if value.trim().is_empty()
        || value.chars().any(char::is_whitespace)
        || value.contains('/')
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.contains('\0')
        || value.contains("..")
        || value.contains('%')
    {
        return Err(AppError::BadRequest(
            "Stored provider credential is invalid for path injection. Reconnect the provider."
                .to_string(),
        ));
    }

    Ok(())
}

fn contains_dot_segment(value: &str) -> bool {
    value
        .split('/')
        .any(|segment| segment == "." || segment == "..")
}

fn contains_raw_path_breaker(value: &str) -> bool {
    value.contains('\\')
        || value.contains('\0')
        || value.contains('?')
        || value.contains('#')
        || value.contains("//")
        || contains_dot_segment(value)
}

fn contains_percent_encoded_path_breaker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains("%00")
        || lower.contains("%3f")
        || lower.contains("%23")
        || lower.split('/').any(|segment| {
            let decoded_dots = segment.replace("%2e", ".");
            decoded_dots == "." || decoded_dots == ".."
        })
}

fn contains_nested_percent_encoded_path_breaker(value: &str) -> bool {
    let mut current = value.to_string();

    // Axum decodes one layer before handlers see the wildcard path. Some
    // downstream routers and proxies decode additional layers, so walk a few
    // more rounds and reject anything that would later collapse into a path
    // separator, fragment/query delimiter, null byte, or dot-segment.
    for _ in 0..8 {
        if contains_percent_encoded_path_breaker(&current) {
            return true;
        }

        let decoded = match urlencoding::decode(&current) {
            Ok(decoded) => decoded.into_owned(),
            Err(_) => break,
        };

        if decoded == current {
            break;
        }

        if contains_raw_path_breaker(&decoded) {
            return true;
        }

        current = decoded;
    }

    false
}

pub(crate) fn validate_requested_proxy_path(path: &str) -> AppResult<()> {
    if contains_raw_path_breaker(path) || contains_nested_percent_encoded_path_breaker(path) {
        return Err(AppError::BadRequest("Invalid proxy path".to_string()));
    }

    Ok(())
}

/// When `target.auth_method` is `"path"`, synthesize a `DelegatedCredential`
/// so `build_forward_path` / `prepare_delegated_request` inject the path
/// prefix (e.g. `/bot<token>/`).  Appends in-place and returns the
/// (possibly extended) slice.
pub fn extend_with_path_credential(delegated: &mut Vec<DelegatedCredential>, target: &ProxyTarget) {
    if target.auth_method == "path" && !target.credential.is_empty() {
        delegated.push(DelegatedCredential {
            provider_slug: String::new(),
            injection_method: "path".to_string(),
            injection_key: target.auth_key_name.clone(),
            credential: target.credential.clone(),
        });
    }
}

pub(crate) fn build_forward_path(
    path: &str,
    delegated_credentials: &[DelegatedCredential],
) -> AppResult<String> {
    validate_requested_proxy_path(path)?;

    let mut prefix_segments = Vec::new();
    for cred in delegated_credentials {
        if cred.injection_method == "path" {
            validate_path_injection_prefix(&cred.injection_key)?;
            validate_path_injection_credential(&cred.credential)?;
            prefix_segments.push(format!("{}{}", cred.injection_key, cred.credential));
        }
    }

    let trimmed_path = path.trim_start_matches('/');
    let final_path = if prefix_segments.is_empty() {
        trimmed_path.to_string()
    } else if trimmed_path.is_empty() {
        prefix_segments.join("/")
    } else {
        format!("{}/{}", prefix_segments.join("/"), trimmed_path)
    };

    validate_requested_proxy_path(&final_path)?;
    Ok(final_path)
}

pub(crate) fn prepare_delegated_request(
    path: &str,
    query: Option<&str>,
    delegated_credentials: &[DelegatedCredential],
) -> AppResult<PreparedDelegatedRequest> {
    let mut delegated_headers = Vec::new();
    let mut forwarded_query = query
        .map(str::to_string)
        .filter(|existing| !existing.is_empty());

    for cred in delegated_credentials {
        match cred.injection_method.as_str() {
            "bearer" => delegated_headers.push((
                cred.injection_key.clone(),
                format!("Bearer {}", cred.credential),
            )),
            "header" => {
                delegated_headers.push((cred.injection_key.clone(), cred.credential.clone()));
            }
            "query" => {
                let encoded = form_urlencoded::Serializer::new(String::new())
                    .append_pair(&cred.injection_key, &cred.credential)
                    .finish();
                match forwarded_query.as_mut() {
                    Some(existing) => {
                        existing.push('&');
                        existing.push_str(&encoded);
                    }
                    None => forwarded_query = Some(encoded),
                }
            }
            "path" => {}
            _ => {}
        }
    }

    Ok(PreparedDelegatedRequest {
        path: build_forward_path(path, delegated_credentials)?,
        query: forwarded_query,
        delegated_headers,
    })
}

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

    let base_url = resolve_gateway_url_override(db, user_id, &service)
        .await?
        .unwrap_or_else(|| service.base_url.clone());

    Ok(ProxyTarget {
        base_url,
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

    let base_url = resolve_gateway_url_override(db, user_id, &service)
        .await?
        .unwrap_or_else(|| service.base_url.clone());

    Ok((
        ProxyTarget {
            base_url,
            auth_method: service.auth_method.clone(),
            auth_key_name: service.auth_key_name.clone(),
            credential,
            service,
        },
        has_credential,
    ))
}

/// Result of resolving a proxy target from the UserService model.
pub struct UserServiceResolution {
    pub target: ProxyTarget,
    pub node_id: Option<String>,
    pub user_service_id: String,
    pub has_server_credential: bool,
    /// Set when the resolved UserService was reached via org membership
    /// (the actor has no personal copy). `None` means personal credentials.
    pub org_routing: Option<OrgRouting>,
}

/// Audit metadata for proxy calls that resolved through an org membership
/// instead of the actor's own credentials.
#[derive(Debug, Clone)]
pub struct OrgRouting {
    pub org_user_id: String,
    pub member_user_id: String,
    pub membership_id: String,
}

/// Resolve proxy target from the new UserService model.
///
/// Resolution order (critical -- see ChronoAIProject/NyxID#209 Codex review):
///
/// 1. **Personal new-path `UserService`** (short-circuit). Most common case.
/// 2. **Legacy personal guard.** If the user has a pre-migration personal
///    `UserServiceConnection` or `UserProviderToken` for this slug, return
///    `Ok(None)` so the handler runs its legacy path. The legacy personal
///    connection must outrank any org-shared credential the user might
///    inherit; otherwise joining an org silently retargets the user's own
///    creds, or worse, blocks them with a 403 when the org membership is
///    viewer / scope-restricted.
/// 3. **Org fallback.** Bounded by a wall-clock timeout. Only runs when
///    the user has *no* personal connection of any kind.
///
/// Returns `Ok(Some(UserServiceResolution))` when a target is resolved.
/// Returns `Ok(None)` to signal the caller should fall back to old resolution.
pub async fn resolve_proxy_target_from_user_service(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    _node_ws_manager: &Arc<NodeWsManager>,
    user_id: &str,
    slug: Option<&str>,
    catalog_service_id: Option<&str>,
) -> AppResult<Option<UserServiceResolution>> {
    // 1. Personal lookup (short-circuit for the common case).
    let personal = lookup_user_service(db, user_id, slug, catalog_service_id).await?;
    if let Some(us) = personal {
        return Ok(Some(
            finish_resolution(db, encryption_keys, user_id, us, None).await?,
        ));
    }

    // 2. Legacy personal guard. Preserves the invariant that pre-migration
    //    personal connections beat org-shared credentials. See function doc.
    if user_has_legacy_personal_connection(db, user_id, slug, catalog_service_id).await? {
        return Ok(None);
    }

    // 3. Org fallback. Bounded by a wall-clock timeout so a degraded Mongo
    //    doesn't make every proxy 404 hang.
    let memberships =
        match crate::services::org_service::find_active_memberships_with_timeout(db, user_id).await
        {
            Ok(rows) => rows,
            Err(crate::errors::AppError::OrgQueryTimeout) => return Err(AppError::OrgQueryTimeout),
            Err(e) => return Err(e),
        };
    if memberships.is_empty() {
        return Ok(None);
    }

    // 3. Walk memberships in priority order. find_active_memberships_with_timeout
    //    has already moved primary_org_id to the front.
    let mut role_denied = false;
    for membership in &memberships {
        let org_us =
            lookup_user_service(db, &membership.org_user_id, slug, catalog_service_id).await?;
        let Some(org_us) = org_us else {
            continue;
        };

        // Role check: Viewer cannot proxy.
        if !membership.role.can_proxy() {
            role_denied = true;
            tracing::debug!(
                user_id = %user_id,
                org_user_id = %membership.org_user_id,
                role = ?membership.role,
                "Org membership role insufficient for proxy"
            );
            continue;
        }

        // Scope check: allowed_service_ids may restrict access to a subset.
        if !membership.allows_service(&org_us.id) {
            role_denied = true;
            tracing::debug!(
                user_id = %user_id,
                org_user_id = %membership.org_user_id,
                user_service_id = %org_us.id,
                "User not in allowed_service_ids scope for this org membership"
            );
            continue;
        }

        let routing = OrgRouting {
            org_user_id: membership.org_user_id.clone(),
            member_user_id: user_id.to_string(),
            membership_id: membership.id.clone(),
        };
        return Ok(Some(
            finish_resolution(
                db,
                encryption_keys,
                &membership.org_user_id,
                org_us,
                Some(routing),
            )
            .await?,
        ));
    }

    // No org service matched. If at least one was found but blocked by role
    // or scope, surface that as a 403 instead of a generic 404 -- the user
    // gets a clearer error and the audit trail captures the denial.
    if role_denied {
        return Err(AppError::OrgRoleInsufficient(
            "your role in the owning org does not permit using this service".to_string(),
        ));
    }
    Ok(None)
}

/// Return true when the user has a legacy (pre-migration) personal
/// connection for the given service slug or catalog_service_id.
///
/// "Legacy personal connection" means one of:
///
/// - a row in `user_service_connections` keyed by the corresponding
///   `DownstreamService.id`, OR
/// - a row in `user_provider_tokens` keyed by the service's
///   `provider_config_id` with a non-revoked status.
///
/// Used by the proxy resolver to defer to the legacy path BEFORE running
/// the org fallback. Legacy personal credentials must outrank org-shared
/// ones during migration; otherwise joining an org silently retargets a
/// user's own creds or hits an org scope/role 403.
///
/// Expensive? One indexed lookup in `downstream_services` plus at most one
/// count on each of `user_service_connections` / `user_provider_tokens`.
/// The short-circuits keep the common "no legacy at all" case to ~2 round
/// trips. Users fully migrated to `UserService` never hit this path.
async fn user_has_legacy_personal_connection(
    db: &mongodb::Database,
    user_id: &str,
    slug: Option<&str>,
    catalog_service_id: Option<&str>,
) -> AppResult<bool> {
    // Resolve to a DownstreamService so we can look up the legacy tables.
    let downstream: Option<DownstreamService> = if let Some(csid) = catalog_service_id {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": csid })
            .await?
    } else if let Some(s) = slug {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "slug": s, "is_active": true })
            .await?
    } else {
        return Ok(false);
    };

    let Some(downstream) = downstream else {
        return Ok(false);
    };

    // 1. Direct user -> service connection (covers `UserServiceConnection`
    //    credentials in the legacy path).
    let conn_count = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .count_documents(doc! {
            "user_id": user_id,
            "service_id": &downstream.id,
        })
        .await?;
    if conn_count > 0 {
        return Ok(true);
    }

    // 2. Provider token (covers legacy provider-backed services like the
    //    old OpenAI/Anthropic connections that used UserProviderToken).
    if let Some(provider_config_id) = &downstream.provider_config_id {
        let token_count = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .count_documents(doc! {
                "user_id": user_id,
                "provider_config_id": provider_config_id,
                "status": { "$in": ["active", "expired", "refresh_failed"] },
            })
            .await?;
        if token_count > 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Find the effective owner of a `UserService` that matches the given
/// slug or catalog service id, scanning the actor's personal scope first
/// and then any org memberships. Used by callers (e.g. SSH tunnel,
/// channel handlers) that need to know whose approval policy applies
/// without doing the full credential resolution.
///
/// Resolve a proxy target from a specific `UserService` id, bypassing
/// the auto-resolution cascade. Used when the caller passes
/// `?_nyxid_via=<user_service_id>` on the proxy route.
///
/// The caller gets the id from `GET /api/v1/user-services` or
/// `GET /api/v1/keys`, which already list both personal and org-
/// inherited services tagged with `credential_source`. This endpoint
/// lets them explicitly choose which credential to use when both
/// exist for the same slug.
///
/// Access check mirrors what the auto-resolution cascade enforces:
/// - **Direct owner:** always allowed.
/// - **Org admin:** allowed if `allowed_service_ids` scope passes.
/// - **Org member (non-viewer):** allowed if `role.can_proxy()` AND
///   `allowed_service_ids` scope passes.
/// - **Viewer / Forbidden:** denied.
///
/// `expected_slug` and `expected_catalog_service_id` constrain the
/// override to the service named in the route path. Without this check
/// a caller could pass a UserService ID for a *different* service than
/// the one the URL names, silently proxying through an unrelated
/// credential. At least one of the two must be `Some`; the function
/// rejects the resolution when the found UserService doesn't match.
pub async fn resolve_proxy_target_by_user_service_id(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    actor_user_id: &str,
    user_service_id: &str,
    expected_slug: Option<&str>,
    expected_catalog_service_id: Option<&str>,
) -> AppResult<Option<UserServiceResolution>> {
    let svc = match user_service_service::find_user_service_by_id(db, user_service_id).await? {
        Some(s) => s,
        None => return Ok(None),
    };

    // Verify the selected UserService matches the route's identity.
    // The slug handler passes expected_slug; the catalog-id handler
    // passes expected_catalog_service_id. Both must match if provided.
    if let Some(slug) = expected_slug
        && svc.slug != slug
    {
        return Err(AppError::BadRequest(format!(
            "_nyxid_via UserService '{user_service_id}' has slug '{}', \
             but the route requested '{slug}'",
            svc.slug
        )));
    }
    if let Some(catalog_id) = expected_catalog_service_id {
        let svc_catalog = svc.catalog_service_id.as_deref().unwrap_or("");
        if svc_catalog != catalog_id {
            return Err(AppError::BadRequest(format!(
                "_nyxid_via UserService '{user_service_id}' belongs to catalog \
                 service '{svc_catalog}', but the route requested '{catalog_id}'"
            )));
        }
    }

    // Access gate: resolve the actor's relationship to the service owner.
    let access =
        crate::services::org_service::resolve_owner_access(db, actor_user_id, &svc.user_id).await?;
    let allowed = match &access {
        crate::services::org_service::OwnerAccess::Direct => true,
        crate::services::org_service::OwnerAccess::AsOrgAdmin { .. } => {
            access.allows_resource(&svc.id)
        }
        crate::services::org_service::OwnerAccess::AsOrgMember { role, .. } => {
            role.can_proxy() && access.allows_resource(&svc.id)
        }
        crate::services::org_service::OwnerAccess::Forbidden => false,
    };
    if !allowed {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have proxy access to this service".to_string(),
        ));
    }

    // Build the org_routing context if the service is org-owned.
    let org_routing = if svc.user_id != actor_user_id {
        // The service belongs to an org; build the routing context from
        // the OwnerAccess that we already resolved above. We know the
        // access is at least AsOrgAdmin or AsOrgMember (Viewer was
        // rejected), so we can extract the membership_id.
        let (org_user_id, membership_id) = match &access {
            crate::services::org_service::OwnerAccess::AsOrgAdmin {
                org_user_id,
                membership_id,
                ..
            }
            | crate::services::org_service::OwnerAccess::AsOrgMember {
                org_user_id,
                membership_id,
                ..
            } => (org_user_id.clone(), membership_id.clone()),
            _ => unreachable!("Direct and Forbidden already handled"),
        };
        Some(OrgRouting {
            org_user_id,
            member_user_id: actor_user_id.to_string(),
            membership_id,
        })
    } else {
        None
    };

    let owner_id = svc.user_id.clone();
    Ok(Some(
        finish_resolution(db, encryption_keys, &owner_id, svc, org_routing).await?,
    ))
}

/// Mirrors `resolve_proxy_target_from_user_service` exactly so that the
/// approval policy resolution sees the *same* effective owner the proxy
/// would actually pick at request time. In particular it:
///
/// 1. Returns the actor when there is a personal `UserService`.
/// 2. Returns the actor when the actor has a legacy personal connection
///    (`UserServiceConnection` or `UserProviderToken`) -- those still
///    outrank org-shared credentials during migration.
/// 3. Walks active memberships in `primary_org_id`-priority order
///    (`find_active_memberships_with_timeout`) and applies the same
///    role + scope filters as the proxy resolver.
/// 4. Returns `None` when no UserService is found anywhere; in that
///    case the caller falls back to the actor's own approval policy.
///
/// Does NOT decrypt credentials or load endpoints. Pure ownership lookup.
pub async fn find_effective_service_owner(
    db: &mongodb::Database,
    actor_user_id: &str,
    slug: Option<&str>,
    catalog_service_id: Option<&str>,
) -> AppResult<Option<String>> {
    // 1. Personal lookup (short-circuit).
    if let Some(svc) = lookup_user_service(db, actor_user_id, slug, catalog_service_id).await? {
        return Ok(Some(svc.user_id));
    }

    // 2. Legacy personal guard. Same invariant as the proxy resolver:
    //    pre-migration personal connections outrank org-shared
    //    credentials. Returning the actor here keeps the approval policy
    //    aligned with the legacy path.
    if user_has_legacy_personal_connection(db, actor_user_id, slug, catalog_service_id).await? {
        return Ok(Some(actor_user_id.to_string()));
    }

    // 3. Org fallback in priority order. Use the same timeout-bounded
    //    membership lookup as the proxy resolver so the priority moves
    //    `primary_org_id` to the front. We swallow `OrgQueryTimeout`
    //    because this is called outside the proxy hot path -- the
    //    caller still gets a deterministic answer (None) and the proxy
    //    will surface the timeout itself if it bites later.
    let memberships =
        match crate::services::org_service::find_active_memberships_with_timeout(db, actor_user_id)
            .await
        {
            Ok(rows) => rows,
            Err(crate::errors::AppError::OrgQueryTimeout) => return Ok(None),
            Err(e) => return Err(e),
        };
    for membership in memberships {
        let Some(org_us) =
            lookup_user_service(db, &membership.org_user_id, slug, catalog_service_id).await?
        else {
            continue;
        };
        // Mirror the proxy resolver's role + scope filters.
        if !membership.role.can_proxy() {
            continue;
        }
        if !membership.allows_service(&org_us.id) {
            continue;
        }
        return Ok(Some(org_us.user_id));
    }

    Ok(None)
}

/// Look up a `UserService` for the given owner by either slug or catalog
/// service id. Pure data access, no decryption or side effects.
async fn lookup_user_service(
    db: &mongodb::Database,
    owner_id: &str,
    slug: Option<&str>,
    catalog_service_id: Option<&str>,
) -> AppResult<Option<crate::models::user_service::UserService>> {
    if let Some(slug) = slug {
        user_service_service::find_by_slug(db, owner_id, slug).await
    } else if let Some(csid) = catalog_service_id {
        user_service_service::find_by_catalog_service_id(db, owner_id, csid).await
    } else {
        Ok(None)
    }
}

/// Verify that an auto-provisioned UserService is still eligible.
///
/// Called at proxy time for services with `source == "auto_provision"`.
/// Rechecks the full "truly no-auth" predicate on the catalog entry and,
/// for private services, verifies the user still has a valid consent for
/// one of its `developer_app_ids`.
///
/// Returns `Ok(())` if the service is still eligible or is not auto-provisioned.
/// Returns `Err(NotFound)` if the service should no longer be accessible.
async fn verify_auto_provision_eligibility(
    db: &mongodb::Database,
    user_service: &crate::models::user_service::UserService,
    effective_owner_id: &str,
) -> AppResult<()> {
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
    };

    // Only check auto-provisioned services
    if user_service.source.as_deref() != Some("auto_provision") {
        return Ok(());
    }

    let catalog_id = match user_service.catalog_service_id.as_deref() {
        Some(id) => id,
        None => {
            // Auto-provisioned services must always have a catalog link.
            // A missing one is a data integrity issue -- reject rather than
            // silently allowing a malformed row through.
            return Err(AppError::NotFound(
                "Service is no longer available".to_string(),
            ));
        }
    };

    // Load the catalog entry
    let ds = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": catalog_id })
        .await?;

    let ds = match ds {
        Some(ds) => ds,
        None => {
            return Err(AppError::NotFound(
                "Service is no longer available".to_string(),
            ));
        }
    };

    // Re-check the full "truly no-auth" predicate
    if !ds.is_active
        || ds.auth_method != "none"
        || ds.requires_user_credential
        || (ds.service_category != "connection" && ds.service_category != "internal")
        || ds.service_type != "http"
    {
        return Err(AppError::NotFound(
            "Service is no longer available".to_string(),
        ));
    }

    // Check for SPR (master credential injection makes it not truly no-auth)
    let spr_count = db
        .collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
        .count_documents(doc! { "service_id": catalog_id })
        .await?;
    if spr_count > 0 {
        return Err(AppError::NotFound(
            "Service is no longer available".to_string(),
        ));
    }

    // Check visibility/consent rules
    if ds.visibility == "private" {
        match ds.developer_app_ids.as_ref() {
            Some(app_ids) if !app_ids.is_empty() => {
                let app_id_refs: Vec<&str> = app_ids.iter().map(|s| s.as_str()).collect();
                let consented = crate::services::unified_key_service::load_valid_app_consents(
                    db,
                    effective_owner_id,
                    &app_id_refs,
                )
                .await?;
                if !app_ids.iter().any(|id| consented.contains(id.as_str())) {
                    return Err(AppError::NotFound(
                        "Service is no longer available".to_string(),
                    ));
                }
            }
            _ => {
                return Err(AppError::NotFound(
                    "Service is no longer available".to_string(),
                ));
            }
        }
    }

    Ok(())
}

async fn finish_resolution(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    effective_owner_id: &str,
    user_service: crate::models::user_service::UserService,
    org_routing: Option<OrgRouting>,
) -> AppResult<UserServiceResolution> {
    // For auto-provisioned services, verify the catalog entry is still eligible
    // before allowing the proxy request through.
    verify_auto_provision_eligibility(db, &user_service, effective_owner_id).await?;

    // Load the endpoint
    let endpoint = db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! { "_id": &user_service.endpoint_id })
        .await?
        .ok_or_else(|| {
            tracing::error!(
                endpoint_id = %user_service.endpoint_id,
                "UserService references missing endpoint"
            );
            AppError::Internal("Data integrity error: endpoint not found".to_string())
        })?;

    // Handle no-auth services (may have no api_key_id)
    if user_service.auth_method == "none" {
        let now = chrono::Utc::now();
        let token_exchange_config =
            load_token_exchange_config_for_user_service(db, &user_service).await?;
        let minimal_service =
            build_minimal_downstream_service(&user_service, &endpoint, now, token_exchange_config);

        return Ok(UserServiceResolution {
            target: ProxyTarget {
                base_url: endpoint.url.clone(),
                auth_method: user_service.auth_method.clone(),
                auth_key_name: user_service.auth_key_name.clone(),
                credential: String::new(),
                service: minimal_service,
            },
            node_id: user_service.node_id.clone(),
            user_service_id: user_service.id.clone(),
            has_server_credential: true,
            org_routing,
        });
    }

    // Load the api key (required for auth services)
    let ak_id = user_service.api_key_id.as_deref().ok_or_else(|| {
        tracing::error!(
            user_service_id = %user_service.id,
            "Non-none auth service missing api_key_id"
        );
        AppError::Internal("Data integrity error: api_key_id missing".to_string())
    })?;

    let api_key = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": ak_id })
        .await?
        .ok_or_else(|| {
            tracing::error!(
                api_key_id = %ak_id,
                "UserService references missing API key"
            );
            AppError::Internal("Data integrity error: API key not found".to_string())
        })?;

    let api_key =
        maybe_refresh_provider_backed_api_key(db, encryption_keys, effective_owner_id, api_key)
            .await?;

    // Node-routed services: resolve what we can but don't block on API key status
    // since the node agent handles credential injection locally.
    if user_service.node_id.is_some() {
        let credential = match resolve_user_api_key_credential(&api_key, encryption_keys).await {
            Ok(cred) => cred,
            Err(e) => {
                tracing::debug!(
                    api_key_id = %api_key.id,
                    error = %e,
                    "Could not resolve server credential for node-routed service (non-fatal)"
                );
                None
            }
        };
        let has_server_credential = credential.is_some();

        let now = chrono::Utc::now();
        let token_exchange_config =
            load_token_exchange_config_for_user_service(db, &user_service).await?;
        let minimal_service =
            build_minimal_downstream_service(&user_service, &endpoint, now, token_exchange_config);

        return Ok(UserServiceResolution {
            target: ProxyTarget {
                base_url: endpoint.url.clone(),
                auth_method: user_service.auth_method.clone(),
                auth_key_name: user_service.auth_key_name.clone(),
                credential: credential.unwrap_or_default(),
                service: minimal_service,
            },
            node_id: user_service.node_id.clone(),
            user_service_id: user_service.id.clone(),
            has_server_credential,
            org_routing,
        });
    }

    if api_key.status != "active" {
        return Err(AppError::BadRequest(format!(
            "API key is {}",
            api_key.status
        )));
    }

    let credential = resolve_user_api_key_credential(&api_key, encryption_keys).await?;

    // Direct routing: require a server-side credential.
    let credential = credential.ok_or_else(|| missing_user_api_key_credential_error(&api_key))?;

    // Fire-and-forget: update last_used_at
    let db_clone = db.clone();
    let key_id = api_key.id.clone();
    tokio::spawn(async move {
        user_api_key_service::touch_last_used(&db_clone, &key_id).await;
    });

    let now = chrono::Utc::now();
    let token_exchange_config =
        load_token_exchange_config_for_user_service(db, &user_service).await?;
    let minimal_service =
        build_minimal_downstream_service(&user_service, &endpoint, now, token_exchange_config);

    Ok(UserServiceResolution {
        target: ProxyTarget {
            base_url: endpoint.url.clone(),
            auth_method: user_service.auth_method.clone(),
            auth_key_name: user_service.auth_key_name.clone(),
            credential,
            service: minimal_service,
        },
        node_id: user_service.node_id.clone(),
        user_service_id: user_service.id.clone(),
        has_server_credential: true,
        org_routing,
    })
}

/// Resolve a per-agent credential override for the given API key + service.
///
/// If an `AgentServiceBinding` exists that maps this agent (API key) to a
/// different `UserApiKey` for the given service, loads and decrypts that
/// credential. Returns `None` if no override exists.
pub async fn resolve_agent_credential_override(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    api_key_id: &str,
    user_service_id: &str,
) -> AppResult<Option<String>> {
    let override_key_id = agent_binding_service::resolve_credential_override(
        db,
        api_key_id,
        user_service_id,
        user_id,
    )
    .await?;

    let Some(override_key_id) = override_key_id else {
        return Ok(None);
    };

    let api_key = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "_id": &override_key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| {
            tracing::error!(
                override_key_id = %override_key_id,
                "Agent binding references missing UserApiKey"
            );
            AppError::Internal("Bound credential not found".to_string())
        })?;

    let api_key =
        maybe_refresh_provider_backed_api_key(db, encryption_keys, user_id, api_key).await?;

    if api_key.status != "active" {
        return Err(AppError::BadRequest(format!(
            "Override credential is {}",
            api_key.status
        )));
    }

    let credential = resolve_user_api_key_credential(&api_key, encryption_keys).await?;

    // Fire-and-forget: update last_used_at on the override key
    if credential.is_some() {
        let db_clone = db.clone();
        let key_id = api_key.id.clone();
        tokio::spawn(async move {
            user_api_key_service::touch_last_used(&db_clone, &key_id).await;
        });
    }

    Ok(credential)
}

async fn maybe_refresh_provider_backed_api_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    api_key: UserApiKey,
) -> AppResult<UserApiKey> {
    let needs_refresh = api_key.credential_type == "oauth2"
        && (api_key.access_token_encrypted.is_none()
            || api_key
                .expires_at
                .is_some_and(|exp| exp <= chrono::Utc::now() + chrono::Duration::minutes(5)));

    if !needs_refresh {
        return Ok(api_key);
    }

    let provider_config_id = match api_key.provider_config_id.as_deref() {
        Some(provider_config_id) => provider_config_id,
        None => return Ok(api_key),
    };

    match user_token_service::get_active_token(db, encryption_keys, user_id, provider_config_id)
        .await
    {
        Ok(_) => {
            user_api_key_service::sync_provider_token_to_api_keys(db, user_id, provider_config_id)
                .await?;

            db.collection::<UserApiKey>(USER_API_KEYS)
                .find_one(doc! { "_id": &api_key.id })
                .await?
                .ok_or_else(|| {
                    AppError::Internal("API key disappeared after provider sync".to_string())
                })
        }
        Err(AppError::NotFound(_)) => Ok(api_key),
        Err(error) => Err(error),
    }
}

async fn resolve_user_api_key_credential(
    api_key: &UserApiKey,
    encryption_keys: &EncryptionKeys,
) -> AppResult<Option<String>> {
    let encrypted = match api_key.credential_type.as_str() {
        "oauth2" => api_key.access_token_encrypted.as_ref(),
        "node_managed" | "ssh_certificate" => None,
        _ => api_key.credential_encrypted.as_ref(),
    };

    let Some(encrypted) = encrypted else {
        return Ok(None);
    };

    let decrypted_bytes = Zeroizing::new(encryption_keys.decrypt(encrypted).await?);
    let credential = String::from_utf8((*decrypted_bytes).clone()).map_err(|e| {
        tracing::error!("Credential UTF-8 decode failed: {e}");
        AppError::Internal("Failed to decode credential".to_string())
    })?;

    if credential.is_empty() {
        Ok(None)
    } else {
        Ok(Some(credential))
    }
}

fn missing_user_api_key_credential_error(api_key: &UserApiKey) -> AppError {
    match api_key.credential_type.as_str() {
        "oauth2" if api_key.provider_config_id.is_some() => AppError::BadRequest(
            "OAuth connection is not complete. Connect your account first.".to_string(),
        ),
        "oauth2" => AppError::BadRequest("OAuth token has no credential stored".to_string()),
        _ => AppError::BadRequest(
            "No credential stored. Add a credential or route through a node.".to_string(),
        ),
    }
}

/// Load the catalog `token_exchange_config` for a user service, if one is
/// required. Returns `Ok(None)` for auth methods that don't use a token
/// exchange config. Fails loudly if the user service is configured for
/// `token_exchange` but the catalog link is missing or the catalog row
/// lacks the config.
///
/// Extracted as its own function so the DB fetch has a single home and
/// the pure struct-assembly logic in `build_minimal_downstream_service`
/// stays unit-testable.
async fn load_token_exchange_config_for_user_service(
    db: &mongodb::Database,
    user_service: &crate::models::user_service::UserService,
) -> AppResult<Option<crate::models::downstream_service::TokenExchangeConfig>> {
    if user_service.auth_method != "token_exchange" {
        return Ok(None);
    }
    let catalog_id = user_service.catalog_service_id.as_deref().ok_or_else(|| {
        AppError::BadRequest(
            "token_exchange services must be linked to a catalog entry".to_string(),
        )
    })?;
    let svc = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! { "_id": catalog_id })
        .await?
        .ok_or_else(|| {
            tracing::error!(
                user_service_id = %user_service.id,
                catalog_service_id = %catalog_id,
                "UserService references missing catalog DownstreamService"
            );
            AppError::Internal(
                "Data integrity error: catalog service not found for token_exchange".to_string(),
            )
        })?;
    // A catalog entry with auth_method=token_exchange MUST have a config.
    // If it doesn't, surface the integrity error here rather than 500ing
    // in the proxy's forward_request.
    if svc.token_exchange_config.is_none() {
        return Err(AppError::Internal(format!(
            "Catalog service '{}' is missing token_exchange_config",
            svc.slug
        )));
    }
    Ok(svc.token_exchange_config)
}

/// Build a minimal DownstreamService struct for backward compatibility
/// with existing proxy pipeline code that expects a `ProxyTarget.service`.
///
/// Pure function - the caller is responsible for fetching any catalog
/// `token_exchange_config` via `load_token_exchange_config_for_user_service`
/// and passing it in. This keeps the function unit-testable without a
/// live MongoDB connection.
fn build_minimal_downstream_service(
    user_service: &crate::models::user_service::UserService,
    endpoint: &UserEndpoint,
    now: chrono::DateTime<chrono::Utc>,
    token_exchange_config: Option<crate::models::downstream_service::TokenExchangeConfig>,
) -> DownstreamService {
    DownstreamService {
        id: user_service
            .catalog_service_id
            .clone()
            .unwrap_or_else(|| user_service.id.clone()),
        name: endpoint.label.clone(),
        slug: user_service.slug.clone(),
        description: None,
        base_url: endpoint.url.clone(),
        service_type: "http".to_string(),
        visibility: "public".to_string(),
        auth_method: user_service.auth_method.clone(),
        auth_key_name: user_service.auth_key_name.clone(),
        credential_encrypted: vec![],
        auth_type: None,
        openapi_spec_url: None,
        asyncapi_spec_url: None,
        streaming_supported: true,
        ssh_config: None,
        oauth_client_id: None,
        service_category: "connection".to_string(),
        requires_user_credential: true,
        is_active: true,
        created_by: "system".to_string(),
        identity_propagation_mode: user_service.identity_propagation_mode.clone(),
        identity_include_user_id: user_service.identity_include_user_id,
        identity_include_email: user_service.identity_include_email,
        identity_include_name: user_service.identity_include_name,
        identity_jwt_audience: user_service.identity_jwt_audience.clone(),
        forward_access_token: user_service.forward_access_token,
        inject_delegation_token: user_service.inject_delegation_token,
        delegation_token_scope: user_service.delegation_token_scope.clone(),
        provider_config_id: None,
        homepage_url: None,
        repository_url: None,
        issues_url: None,
        capabilities: None,
        auth_notes: None,
        known_limitations: None,
        required_permissions: None,
        examples_url: None,
        recommended_skills: None,
        custom_user_agent: user_service.custom_user_agent.clone(),
        developer_app_ids: None,
        token_exchange_config,
        created_at: now,
        updated_at: now,
    }
}

/// For services linked to a provider with `requires_gateway_url`, look up the
/// user's provider token and return their per-user gateway URL.
///
/// Returns `Ok(None)` for providers that don't require a gateway URL.
/// Returns `Err` if the provider requires a gateway URL but the user hasn't
/// connected one -- this prevents fallback to the placeholder base_url.
async fn resolve_gateway_url_override(
    db: &mongodb::Database,
    user_id: &str,
    service: &DownstreamService,
) -> AppResult<Option<String>> {
    let provider_config_id = match &service.provider_config_id {
        Some(id) => id,
        None => return Ok(None),
    };

    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "_id": provider_config_id })
        .await?;

    let provider = match provider {
        Some(p) if p.requires_gateway_url => p,
        _ => return Ok(None),
    };

    let user_token = db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": &provider.id,
            "status": "active",
        })
        .await?;

    match user_token.and_then(|t| t.gateway_url) {
        Some(url) if !url.is_empty() => Ok(Some(url)),
        _ => Err(AppError::BadRequest(format!(
            "Connect your {} instance first (provide your gateway URL in Providers)",
            provider.name
        ))),
    }
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
    body: ProxyBody,
    identity_headers: Vec<(String, String)>,
    delegated_credentials: Vec<DelegatedCredential>,
    // The caller's raw NyxID access token, used when auth_method is "nyxid_token".
    caller_token: Option<&str>,
    // Shared generic token exchange cache (used by `token_exchange`).
    token_exchange_cache: &TokenExchangeCache,
) -> AppResult<reqwest::Response> {
    let mut all_delegated = delegated_credentials;
    extend_with_path_credential(&mut all_delegated, target);
    let prepared = prepare_delegated_request(path, query, &all_delegated)?;

    // TODO(SEC-H1): Re-validate the resolved IP at proxy time to prevent DNS rebinding.
    // Currently base_url is only validated at service creation/update time. An attacker
    // could change DNS to point to a private IP after validation. Consider using a custom
    // DNS resolver or reqwest's `resolve` feature to check the resolved IP before connecting.

    let url = if let Some(q) = prepared.query.as_deref() {
        format!(
            "{}/{}?{}",
            target.base_url.trim_end_matches('/'),
            prepared.path,
            q
        )
    } else {
        format!(
            "{}/{}",
            target.base_url.trim_end_matches('/'),
            prepared.path
        )
    };

    let mut request = client.request(method.clone(), &url);

    // Copy only allowed headers (allowlist approach).
    // When a custom User-Agent is configured for this service, skip the
    // client's User-Agent so we can inject the override below without
    // producing duplicate header values.
    let has_custom_ua = target.service.custom_user_agent.is_some();
    for (name, value) in headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if has_custom_ua && name_lower == "user-agent" {
            continue;
        }
        if ALLOWED_FORWARD_HEADERS.contains(&name_lower.as_str()) {
            request = request.header(name, value);
        }
    }

    // Override User-Agent if the service specifies a custom one.
    // By default (None), the client's User-Agent is forwarded as-is.
    if let Some(ref ua) = target.service.custom_user_agent {
        request = request.header("user-agent", ua.as_str());
    }

    // Inject identity propagation headers
    for (name, value) in &identity_headers {
        request = request.header(name, value);
    }

    // Body injection for `body` auth method must happen before the body is
    // attached to the request. We mutate `body` in place; the actual attach
    // happens further down in the existing `match body { ... }` block.
    //
    // Injection is skipped for methods that cannot carry a request body
    // (GET/HEAD/DELETE/OPTIONS). Injecting into those would produce malformed
    // requests that stricter downstreams (e.g. Cloudflare-fronted APIs) reject
    // with 400 Bad Request. Callers misusing body auth on such methods will
    // simply see the downstream's own auth-missing error.
    let body = if target.auth_method == "body" && method_can_have_body(&method) {
        if target.auth_key_name.is_empty() {
            return Err(AppError::Internal(
                "Body auth method requires a non-empty auth_key_name".to_string(),
            ));
        }
        match body {
            ProxyBody::Buffered(existing) => {
                let merged = inject_credential_into_json_body(
                    existing.as_deref(),
                    &target.auth_key_name,
                    &target.credential,
                )?;
                ProxyBody::Buffered(Some(merged))
            }
        }
    } else {
        body
    };

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
        "bot_bearer" => {
            // Discord bot tokens use `Authorization: Bot <token>` instead of
            // the standard `Bearer` scheme. Sets the literal header value.
            request = request.header("Authorization", format!("Bot {}", target.credential));
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
        "body" => {
            // Body injection already happened above; nothing to add to headers.
        }
        "path" => {
            // Path injection already handled above via synthesized
            // DelegatedCredential + build_forward_path.
        }
        "token_exchange" => {
            // Declarative server-side token exchange. The service's
            // `TokenExchangeConfig` describes how to POST the stored
            // credential JSON, extract a token from the response, cache
            // it, and inject it on outbound requests. Covers Lark/Feishu
            // tenant tokens, OAuth 2.0 client_credentials, and similar
            // provider flows without per-provider code.
            let exchange_config =
                target
                    .service
                    .token_exchange_config
                    .as_ref()
                    .ok_or_else(|| {
                        AppError::Internal(
                        "token_exchange auth method requires token_exchange_config on the service"
                            .to_string(),
                    )
                    })?;
            let credential_map = provider_token_exchange_service::parse_credential(
                &target.credential,
                &exchange_config.credential_fields,
            )?;
            let token = provider_token_exchange_service::get_cached_exchange_token(
                token_exchange_cache,
                client,
                &target.base_url,
                &target.credential,
                exchange_config,
                &credential_map,
            )
            .await?;
            request = provider_token_exchange_service::apply_injection(
                request,
                &exchange_config.injection,
                &token,
            )?;
        }
        _ => {
            return Err(AppError::Internal(format!(
                "Unknown auth method: {}",
                target.auth_method
            )));
        }
    }

    // Forward the caller's NyxID access token when the service is configured for it.
    // This is used by platform apps that trust NyxID JWTs directly.
    if target.service.forward_access_token
        && let Some(token) = caller_token
    {
        request = request.bearer_auth(token);
    }

    // Inject delegated provider credentials that are represented as headers.
    for (name, value) in &prepared.delegated_headers {
        request = request.header(name, value);
    }

    if let ProxyBody::Buffered(Some(ref body_bytes)) = body {
        // Log request body for LLM proxy calls to diagnose truncation issues
        if url.contains("/responses") {
            let body_str = String::from_utf8_lossy(body_bytes);
            let preview = if body_str.len() > 2048 {
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

    match body {
        ProxyBody::Buffered(Some(body_bytes)) => {
            request = request.body(body_bytes);
        }
        ProxyBody::Buffered(None) => {}
    }

    let response = request.send().await.map_err(|e| {
        tracing::error!("Proxy request to {} failed: {e}", target.base_url);
        AppError::Internal("Proxy request failed".to_string())
    })?;

    Ok(response)
}

/// Whether an HTTP method semantically supports a request body.
///
/// Used to guard body-auth credential injection: injecting a JSON body on
/// GET/HEAD/DELETE produces malformed requests that Cloudflare-fronted APIs
/// (notably Lark) reject at the edge with 400 Bad Request before reaching
/// the origin server.
fn method_can_have_body(method: &reqwest::Method) -> bool {
    matches!(
        *method,
        reqwest::Method::POST | reqwest::Method::PUT | reqwest::Method::PATCH
    )
}

/// Merge a credential into the top level of a JSON request body.
///
/// Used by the `body` auth method (e.g. Lark/Feishu `tenant_access_token`
/// exchange where `app_secret` must be in the request body). The credential
/// is added under `key`. If the body is empty, a new JSON object is created.
/// If the caller already set the same key, the caller's value wins -- this
/// lets clients override the injected secret in test scenarios without
/// silent overwrite.
fn inject_credential_into_json_body(
    existing: Option<&[u8]>,
    key: &str,
    credential: &str,
) -> Result<bytes::Bytes, AppError> {
    let mut value: serde_json::Value = match existing {
        Some(bytes) if !bytes.is_empty() => serde_json::from_slice(bytes).map_err(|e| {
            AppError::BadRequest(format!(
                "Body auth method requires a JSON request body: {e}"
            ))
        })?,
        _ => serde_json::Value::Object(serde_json::Map::new()),
    };

    let obj = value.as_object_mut().ok_or_else(|| {
        AppError::BadRequest(
            "Body auth method requires a JSON object as the request body".to_string(),
        )
    })?;

    // Caller's value wins. Only inject if the key is missing.
    if !obj.contains_key(key) {
        obj.insert(
            key.to_string(),
            serde_json::Value::String(credential.to_string()),
        );
    }

    let bytes = serde_json::to_vec(&value).map_err(|e| {
        AppError::Internal(format!("Failed to re-serialize body after injection: {e}"))
    })?;

    Ok(bytes::Bytes::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode, Uri},
        routing::post,
    };
    use chrono::Utc;
    use tokio::{net::TcpListener, sync::mpsc};

    #[derive(Debug)]
    struct CapturedRequest {
        path: String,
        query: Option<String>,
        content_type: Option<String>,
        user_agent: Option<String>,
        body: Vec<u8>,
    }

    async fn capture_request(
        State(sender): State<mpsc::UnboundedSender<CapturedRequest>>,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        let _ = sender.send(CapturedRequest {
            path: uri.path().to_string(),
            query: uri.query().map(ToString::to_string),
            content_type: headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(ToString::to_string),
            user_agent: headers
                .get(reqwest::header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(ToString::to_string),
            body: body.to_vec(),
        });

        StatusCode::OK
    }

    /// Fresh empty token exchange cache for tests that don't exercise
    /// `token_exchange`. Dedicated tests for the cache itself live in
    /// `provider_token_exchange_service::tests`.
    fn empty_token_cache() -> TokenExchangeCache {
        TokenExchangeCache::new()
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
                auth_method: "none".to_string(),
                auth_key_name: "Authorization".to_string(),
                credential_encrypted: vec![],
                auth_type: None,
                openapi_spec_url: None,
                asyncapi_spec_url: None,
                streaming_supported: false,
                ssh_config: None,
                service_type: "http".to_string(),
                visibility: "public".to_string(),
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
                forward_access_token: false,
                inject_delegation_token: false,
                delegation_token_scope: "llm:proxy".to_string(),
                provider_config_id: None,
                homepage_url: None,
                repository_url: None,
                issues_url: None,
                capabilities: None,
                auth_notes: None,
                known_limitations: None,
                required_permissions: None,
                examples_url: None,
                recommended_skills: None,
                custom_user_agent: None,
                developer_app_ids: None,
                token_exchange_config: None,
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
            ProxyBody::Buffered(Some(bytes::Bytes::from_static(b"PK\x03\x04"))),
            vec![],
            vec![],
            None,
            &empty_token_cache(),
        )
        .await
        .expect("proxy request should succeed");

        assert_eq!(response.status(), StatusCode::OK);

        let captured = receiver.recv().await.expect("captured request");
        assert_eq!(captured.path, "/upload");
        assert_eq!(captured.content_type.as_deref(), Some("application/zip"));
        assert_eq!(captured.body, b"PK\x03\x04");

        server.abort();
    }

    #[tokio::test]
    async fn forward_request_passes_through_user_agent_by_default() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let app = Router::new()
            .route("/api/v1/test", post(capture_request))
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
            reqwest::header::USER_AGENT,
            "OpenAI/Python 2.30.0".parse().unwrap(),
        );

        let target = make_proxy_target(format!("http://{addr}"));
        let response = forward_request(
            &Client::new(),
            &target,
            reqwest::Method::POST,
            "api/v1/test",
            None,
            headers,
            ProxyBody::Buffered(Some(Bytes::from_static(b"{}"))),
            vec![],
            vec![],
            None,
            &empty_token_cache(),
        )
        .await
        .expect("proxy request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let captured = receiver.recv().await.expect("captured request");
        assert_eq!(
            captured.user_agent.as_deref(),
            Some("OpenAI/Python 2.30.0"),
            "client User-Agent should be forwarded when no custom_user_agent is set"
        );

        server.abort();
    }

    #[tokio::test]
    async fn forward_request_overrides_user_agent_when_service_has_custom() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let app = Router::new()
            .route("/api/v1/test", post(capture_request))
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
            reqwest::header::USER_AGENT,
            "OpenAI/Python 2.30.0".parse().unwrap(),
        );

        let mut target = make_proxy_target(format!("http://{addr}"));
        target.service.custom_user_agent = Some("NyxID-Proxy/1.0".to_string());

        let response = forward_request(
            &Client::new(),
            &target,
            reqwest::Method::POST,
            "api/v1/test",
            None,
            headers,
            ProxyBody::Buffered(Some(Bytes::from_static(b"{}"))),
            vec![],
            vec![],
            None,
            &empty_token_cache(),
        )
        .await
        .expect("proxy request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let captured = receiver.recv().await.expect("captured request");
        assert_eq!(
            captured.user_agent.as_deref(),
            Some("NyxID-Proxy/1.0"),
            "custom_user_agent should replace the client's User-Agent"
        );

        server.abort();
    }

    #[tokio::test]
    async fn forward_request_injects_delegated_path_credentials_into_url() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let app = Router::new()
            .route("/{*path}", post(capture_request))
            .with_state(sender);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test app");
        });

        let response = forward_request(
            &Client::new(),
            &make_proxy_target(format!("http://{addr}")),
            reqwest::Method::POST,
            "sendMessage",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![DelegatedCredential {
                provider_slug: "telegram-bot".to_string(),
                injection_method: "path".to_string(),
                injection_key: "bot".to_string(),
                credential: "123456:ABC-DEF".to_string(),
            }],
            None,
            &empty_token_cache(),
        )
        .await
        .expect("proxy request should succeed");

        assert_eq!(response.status(), StatusCode::OK);

        let captured = receiver.recv().await.expect("captured request");
        assert_eq!(captured.path, "/bot123456:ABC-DEF/sendMessage");
        assert_eq!(captured.query, None);

        server.abort();
    }

    #[test]
    fn prepare_delegated_request_appends_query_params_and_headers() {
        let prepared = prepare_delegated_request(
            "models",
            Some("stream=true"),
            &[
                DelegatedCredential {
                    provider_slug: "github".to_string(),
                    injection_method: "bearer".to_string(),
                    injection_key: "Authorization".to_string(),
                    credential: "user-token".to_string(),
                },
                DelegatedCredential {
                    provider_slug: "custom".to_string(),
                    injection_method: "header".to_string(),
                    injection_key: "X-Provider-Key".to_string(),
                    credential: "secret".to_string(),
                },
                DelegatedCredential {
                    provider_slug: "custom".to_string(),
                    injection_method: "query".to_string(),
                    injection_key: "api_key".to_string(),
                    credential: "abc 123".to_string(),
                },
            ],
        )
        .expect("delegated request should prepare");

        assert_eq!(prepared.path, "models");
        assert_eq!(
            prepared.query.as_deref(),
            Some("stream=true&api_key=abc+123")
        );
        assert_eq!(
            prepared.delegated_headers,
            vec![
                ("Authorization".to_string(), "Bearer user-token".to_string()),
                ("X-Provider-Key".to_string(), "secret".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn forward_request_rejects_backslash_in_requested_path_injection() {
        let err = forward_request(
            &Client::new(),
            &make_proxy_target("http://127.0.0.1".to_string()),
            reqwest::Method::POST,
            "folder\\sendMessage",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &empty_token_cache(),
        )
        .await
        .expect_err("backslash in requested path should be rejected");

        assert!(
            err.to_string().contains("Invalid proxy path"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn forward_request_rejects_percent_encoded_requested_path_injection() {
        for path in [
            "sendMessage?chat_id=1",
            "sendMessage#fragment",
            "folder%2FsendMessage",
            "folder%2fsendMessage",
            "folder%252FsendMessage",
            "folder%25252FsendMessage",
            "folder%3Fchat_id=1",
            "folder%3fchat_id=1",
            "folder%253Fchat_id=1",
            "folder%25253Fchat_id=1",
            "folder%23fragment",
            "folder%2523fragment",
            "folder%252523fragment",
            "%2e%2e",
            "%252e%252e",
            "%25252e%25252e",
            "%2e.",
            ".%2e",
            "%2E%2E",
            "%2E.",
            ".%2E",
            "folder%5CsendMessage",
            "folder%5csendMessage",
            "folder%255CsendMessage",
            "folder%25255CsendMessage",
            "%00",
            "%2500",
            "%252500",
        ] {
            let err = forward_request(
                &Client::new(),
                &make_proxy_target("http://127.0.0.1".to_string()),
                reqwest::Method::POST,
                path,
                None,
                reqwest::header::HeaderMap::new(),
                ProxyBody::Buffered(None),
                vec![],
                vec![],
                None,
                &empty_token_cache(),
            )
            .await
            .expect_err("percent-encoded requested path breaker should be rejected");

            assert!(
                err.to_string().contains("Invalid proxy path"),
                "unexpected error for '{path}': {err}"
            );
        }
    }

    #[tokio::test]
    async fn forward_request_allows_non_segment_dot_sequences_in_path_injection() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let app = Router::new()
            .route("/{*path}", post(capture_request))
            .with_state(sender);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test app");
        });

        let response = forward_request(
            &Client::new(),
            &make_proxy_target(format!("http://{addr}")),
            reqwest::Method::POST,
            "v1/foo..bar/foo%2ebar",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &empty_token_cache(),
        )
        .await
        .expect("non-segment dot sequences should be allowed");

        assert_eq!(response.status(), StatusCode::OK);

        let captured = receiver.recv().await.expect("captured request");
        assert_eq!(captured.path, "/v1/foo..bar/foo%2ebar");

        server.abort();
    }

    #[tokio::test]
    async fn forward_request_rejects_invalid_path_injection_credentials() {
        let err = forward_request(
            &Client::new(),
            &make_proxy_target("http://127.0.0.1".to_string()),
            reqwest::Method::POST,
            "sendMessage",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![DelegatedCredential {
                provider_slug: "telegram-bot".to_string(),
                injection_method: "path".to_string(),
                injection_key: "bot".to_string(),
                credential: "bad/token".to_string(),
            }],
            None,
            &empty_token_cache(),
        )
        .await
        .expect_err("invalid path credential should be rejected");

        assert!(
            err.to_string().contains("Reconnect the provider"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn forward_request_rejects_blank_or_whitespace_path_injection_credentials() {
        for credential in ["", "   ", "123 456", " 123456:ABC-DEF"] {
            let err = forward_request(
                &Client::new(),
                &make_proxy_target("http://127.0.0.1".to_string()),
                reqwest::Method::POST,
                "sendMessage",
                None,
                reqwest::header::HeaderMap::new(),
                ProxyBody::Buffered(None),
                vec![],
                vec![DelegatedCredential {
                    provider_slug: "telegram-bot".to_string(),
                    injection_method: "path".to_string(),
                    injection_key: "bot".to_string(),
                    credential: credential.to_string(),
                }],
                None,
                &empty_token_cache(),
            )
            .await
            .expect_err("blank or whitespace path credential should be rejected");

            assert!(
                err.to_string().contains("Reconnect the provider"),
                "unexpected error for '{credential}': {err}"
            );
        }
    }

    #[tokio::test]
    async fn forward_request_rejects_invalid_path_injection_prefix() {
        let err = forward_request(
            &Client::new(),
            &make_proxy_target("http://127.0.0.1".to_string()),
            reqwest::Method::POST,
            "sendMessage",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![DelegatedCredential {
                provider_slug: "telegram-bot".to_string(),
                injection_method: "path".to_string(),
                injection_key: "bot/".to_string(),
                credential: "123456:ABC-DEF".to_string(),
            }],
            None,
            &empty_token_cache(),
        )
        .await
        .expect_err("invalid path prefix should be rejected");

        assert!(
            err.to_string().contains("Please contact your admin"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn forward_request_rejects_blank_or_whitespace_path_injection_prefix() {
        for injection_key in ["", "   ", " bot"] {
            let err = forward_request(
                &Client::new(),
                &make_proxy_target("http://127.0.0.1".to_string()),
                reqwest::Method::POST,
                "sendMessage",
                None,
                reqwest::header::HeaderMap::new(),
                ProxyBody::Buffered(None),
                vec![],
                vec![DelegatedCredential {
                    provider_slug: "telegram-bot".to_string(),
                    injection_method: "path".to_string(),
                    injection_key: injection_key.to_string(),
                    credential: "123456:ABC-DEF".to_string(),
                }],
                None,
                &empty_token_cache(),
            )
            .await
            .expect_err("blank or whitespace path prefix should be rejected");

            assert!(
                err.to_string().contains("Please contact your admin"),
                "unexpected error for '{injection_key}': {err}"
            );
        }
    }

    #[tokio::test]
    async fn forward_request_rejects_percent_encoded_path_injection_credential() {
        let err = forward_request(
            &Client::new(),
            &make_proxy_target("http://127.0.0.1".to_string()),
            reqwest::Method::POST,
            "sendMessage",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![DelegatedCredential {
                provider_slug: "telegram-bot".to_string(),
                injection_method: "path".to_string(),
                injection_key: "bot".to_string(),
                credential: "123%2f456".to_string(),
            }],
            None,
            &empty_token_cache(),
        )
        .await
        .expect_err("percent-encoded path credential should be rejected");

        assert!(
            err.to_string().contains("Reconnect the provider"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn prepare_delegated_request_applies_telegram_bot_path_injection() {
        // Regression test: the node routing path calls prepare_delegated_request
        // (not forward_request) so path-injection prefixes must work via that
        // entry point too.  Before the fix in 1209b96, node-routed requests
        // skipped delegated credential resolution entirely.
        let prepared = prepare_delegated_request(
            "sendMessage",
            Some("chat_id=123"),
            &[DelegatedCredential {
                provider_slug: "telegram-bot".to_string(),
                injection_method: "path".to_string(),
                injection_key: "bot".to_string(),
                credential: "123456:ABC-DEF".to_string(),
            }],
        )
        .expect("delegated request should prepare");

        assert_eq!(prepared.path, "bot123456:ABC-DEF/sendMessage");
        assert_eq!(prepared.query.as_deref(), Some("chat_id=123"));
        assert!(
            prepared.delegated_headers.is_empty(),
            "path injection should not produce headers"
        );
    }

    #[tokio::test]
    async fn forward_request_rejects_percent_encoded_path_injection_prefix() {
        let err = forward_request(
            &Client::new(),
            &make_proxy_target("http://127.0.0.1".to_string()),
            reqwest::Method::POST,
            "sendMessage",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![DelegatedCredential {
                provider_slug: "telegram-bot".to_string(),
                injection_method: "path".to_string(),
                injection_key: "bot%2f".to_string(),
                credential: "123456:ABC-DEF".to_string(),
            }],
            None,
            &empty_token_cache(),
        )
        .await
        .expect_err("percent-encoded path prefix should be rejected");

        assert!(
            err.to_string().contains("Please contact your admin"),
            "unexpected error: {err}"
        );
    }

    // ─── inject_credential_into_json_body tests ─────────────────────────

    #[test]
    fn body_injection_merges_into_existing_object() {
        let body = br#"{"app_id":"cli_xxx"}"#.to_vec();
        let result =
            inject_credential_into_json_body(Some(&body), "app_secret", "secret_value").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(parsed["app_id"], "cli_xxx");
        assert_eq!(parsed["app_secret"], "secret_value");
    }

    #[test]
    fn body_injection_creates_object_when_body_empty() {
        let result = inject_credential_into_json_body(None, "app_secret", "secret_value").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(parsed["app_secret"], "secret_value");
    }

    #[test]
    fn body_injection_creates_object_when_body_is_empty_bytes() {
        let result =
            inject_credential_into_json_body(Some(&[]), "app_secret", "secret_value").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(parsed["app_secret"], "secret_value");
    }

    #[test]
    fn body_injection_does_not_overwrite_caller_value() {
        let body = br#"{"app_secret":"caller_value"}"#.to_vec();
        let result =
            inject_credential_into_json_body(Some(&body), "app_secret", "server_value").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(parsed["app_secret"], "caller_value");
    }

    #[test]
    fn body_injection_rejects_non_json_body() {
        let body = b"not json".to_vec();
        let err = inject_credential_into_json_body(Some(&body), "app_secret", "secret_value")
            .unwrap_err();
        assert!(err.to_string().contains("JSON"));
    }

    #[test]
    fn body_injection_rejects_json_array() {
        let body = br#"["a","b"]"#.to_vec();
        let err = inject_credential_into_json_body(Some(&body), "app_secret", "secret_value")
            .unwrap_err();
        assert!(err.to_string().contains("JSON object"));
    }

    // ─── method_can_have_body tests ─────────────────────────────────────

    #[test]
    fn method_can_have_body_accepts_post_put_patch() {
        assert!(method_can_have_body(&reqwest::Method::POST));
        assert!(method_can_have_body(&reqwest::Method::PUT));
        assert!(method_can_have_body(&reqwest::Method::PATCH));
    }

    #[test]
    fn method_can_have_body_rejects_get_head_delete_options() {
        assert!(!method_can_have_body(&reqwest::Method::GET));
        assert!(!method_can_have_body(&reqwest::Method::HEAD));
        assert!(!method_can_have_body(&reqwest::Method::DELETE));
        assert!(!method_can_have_body(&reqwest::Method::OPTIONS));
    }

    // ─── token_exchange integration tests (Lark as example) ──────────

    use axum::{Json, routing::get};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct LarkMockState {
        captured_authorization: Arc<std::sync::Mutex<Option<String>>>,
        token_exchange_count: Arc<AtomicUsize>,
    }

    async fn mock_lark_token_endpoint_handler(
        State(state): State<LarkMockState>,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        // Sanity-check the request body carries both credentials.
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert!(parsed["app_id"].as_str().is_some());
        assert!(parsed["app_secret"].as_str().is_some());
        state.token_exchange_count.fetch_add(1, Ordering::SeqCst);
        Json(serde_json::json!({
            "code": 0,
            "msg": "ok",
            "tenant_access_token": "t-test-token",
            "expire": 7200,
        }))
    }

    async fn lark_api_handler(
        State(state): State<LarkMockState>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        let auth = headers
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string);
        *state.captured_authorization.lock().unwrap() = auth;
        Json(serde_json::json!({"code": 0, "data": {"items": []}}))
    }

    fn make_lark_token_exchange_config() -> crate::models::downstream_service::TokenExchangeConfig {
        use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};
        TokenExchangeConfig {
            endpoint: "{base_url}/open-apis/auth/v3/tenant_access_token/internal".to_string(),
            request_encoding: "json".to_string(),
            request_template: serde_json::json!({
                "app_id": "$app_id",
                "app_secret": "$app_secret",
            }),
            token_response_path: "tenant_access_token".to_string(),
            ttl_response_path: Some("expire".to_string()),
            default_ttl_secs: 7200,
            injection: "bearer".to_string(),
            error_code_path: Some("code".to_string()),
            error_message_path: Some("msg".to_string()),
            credential_fields: vec![
                CredentialFieldSpec {
                    name: "app_id".to_string(),
                    label: "App ID".to_string(),
                    placeholder: None,
                    secret: false,
                },
                CredentialFieldSpec {
                    name: "app_secret".to_string(),
                    label: "App Secret".to_string(),
                    placeholder: None,
                    secret: true,
                },
            ],
        }
    }

    fn make_lark_proxy_target(base_url: String) -> ProxyTarget {
        let now = Utc::now();
        ProxyTarget {
            base_url: base_url.clone(),
            auth_method: "token_exchange".to_string(),
            auth_key_name: String::new(),
            credential: r#"{"app_id":"cli_test","app_secret":"super-secret"}"#.to_string(),
            service: DownstreamService {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Lark Bot".to_string(),
                slug: "api-lark-bot".to_string(),
                description: None,
                base_url,
                auth_method: "token_exchange".to_string(),
                auth_key_name: String::new(),
                credential_encrypted: vec![],
                auth_type: None,
                openapi_spec_url: None,
                asyncapi_spec_url: None,
                streaming_supported: false,
                ssh_config: None,
                service_type: "http".to_string(),
                visibility: "public".to_string(),
                oauth_client_id: None,
                service_category: "external".to_string(),
                requires_user_credential: true,
                is_active: true,
                created_by: "test".to_string(),
                identity_propagation_mode: "none".to_string(),
                identity_include_user_id: false,
                identity_include_email: false,
                identity_include_name: false,
                identity_jwt_audience: None,
                forward_access_token: false,
                inject_delegation_token: false,
                delegation_token_scope: "llm:proxy".to_string(),
                provider_config_id: None,
                homepage_url: None,
                repository_url: None,
                issues_url: None,
                capabilities: None,
                auth_notes: None,
                known_limitations: None,
                required_permissions: None,
                recommended_skills: None,
                examples_url: None,
                custom_user_agent: None,
                developer_app_ids: None,
                token_exchange_config: Some(make_lark_token_exchange_config()),
                created_at: now,
                updated_at: now,
            },
        }
    }

    async fn start_lark_mock() -> (String, LarkMockState, tokio::task::JoinHandle<()>) {
        let state = LarkMockState {
            captured_authorization: Arc::new(std::sync::Mutex::new(None)),
            token_exchange_count: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post(mock_lark_token_endpoint_handler),
            )
            .route("/open-apis/im/v1/chats", get(lark_api_handler))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test app");
        });
        (format!("http://{addr}"), state, server)
    }

    #[tokio::test]
    async fn token_exchange_injects_bearer_on_downstream_request() {
        let (base_url, mock, server) = start_lark_mock().await;
        let cache = TokenExchangeCache::new();

        let response = forward_request(
            &Client::new(),
            &make_lark_proxy_target(base_url),
            reqwest::Method::GET,
            "open-apis/im/v1/chats",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &cache,
        )
        .await
        .expect("lark proxy request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(mock.token_exchange_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            mock.captured_authorization.lock().unwrap().as_deref(),
            Some("Bearer t-test-token")
        );

        server.abort();
    }

    #[tokio::test]
    async fn token_exchange_caches_across_calls() {
        let (base_url, mock, server) = start_lark_mock().await;
        let cache = TokenExchangeCache::new();
        let target = make_lark_proxy_target(base_url);

        // First call triggers a token exchange.
        forward_request(
            &Client::new(),
            &target,
            reqwest::Method::GET,
            "open-apis/im/v1/chats",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &cache,
        )
        .await
        .expect("first call");

        // Second call reuses the cached token.
        forward_request(
            &Client::new(),
            &target,
            reqwest::Method::GET,
            "open-apis/im/v1/chats",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &cache,
        )
        .await
        .expect("second call");

        assert_eq!(
            mock.token_exchange_count.load(Ordering::SeqCst),
            1,
            "token exchange should happen exactly once across two proxy calls"
        );
        server.abort();
    }

    #[tokio::test]
    async fn token_exchange_rejects_malformed_credential() {
        let (base_url, _mock, server) = start_lark_mock().await;
        let mut target = make_lark_proxy_target(base_url);
        target.credential = "not valid json".to_string();

        let err = forward_request(
            &Client::new(),
            &target,
            reqwest::Method::GET,
            "open-apis/im/v1/chats",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &TokenExchangeCache::new(),
        )
        .await
        .expect_err("malformed credential should error");

        assert!(
            err.to_string().contains("token_exchange"),
            "unexpected error: {err}"
        );
        server.abort();
    }

    // ─── build_minimal_downstream_service regression test ────────────

    fn make_user_service_token_exchange() -> crate::models::user_service::UserService {
        crate::models::user_service::UserService {
            id: "us-1".to_string(),
            user_id: "user-1".to_string(),
            slug: "api-lark-bot".to_string(),
            endpoint_id: "ep-1".to_string(),
            api_key_id: Some("ak-1".to_string()),
            auth_method: "token_exchange".to_string(),
            auth_key_name: String::new(),
            catalog_service_id: Some("cat-1".to_string()),
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_endpoint() -> UserEndpoint {
        UserEndpoint {
            id: "ep-1".to_string(),
            user_id: "user-1".to_string(),
            label: "Lark Bot".to_string(),
            url: "https://open.larksuite.com".to_string(),
            catalog_service_id: Some("cat-1".to_string()),
            openapi_spec_url: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn build_minimal_downstream_service_carries_token_exchange_config_through() {
        // Regression: before this was wired, the synthetic DownstreamService
        // built by the user-service resolver hard-coded
        // `token_exchange_config: None`, so every proxy request to a
        // token_exchange service 500'd with
        // "token_exchange auth method requires token_exchange_config on
        // the service" -- even though the catalog row had a perfectly
        // valid config.
        let user_service = make_user_service_token_exchange();
        let endpoint = make_endpoint();
        let config = make_lark_token_exchange_config();

        let svc = build_minimal_downstream_service(
            &user_service,
            &endpoint,
            Utc::now(),
            Some(config.clone()),
        );

        let carried = svc.token_exchange_config.expect("config must be carried");
        assert_eq!(carried.endpoint, config.endpoint);
        assert_eq!(carried.token_response_path, config.token_response_path);
        assert_eq!(carried.credential_fields.len(), 2);
    }

    #[test]
    fn build_minimal_downstream_service_omits_config_for_non_token_exchange() {
        let mut user_service = make_user_service_token_exchange();
        user_service.auth_method = "bearer".to_string();
        let endpoint = make_endpoint();

        let svc = build_minimal_downstream_service(&user_service, &endpoint, Utc::now(), None);

        assert!(svc.token_exchange_config.is_none());
        assert_eq!(svc.auth_method, "bearer");
    }

    // ─── body auth method guard regression tests ─────────────────────

    #[derive(Clone, Default)]
    struct BodyAuthCaptureState {
        captured_body: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    }

    async fn body_capture_get(
        State(state): State<BodyAuthCaptureState>,
        body: Bytes,
    ) -> StatusCode {
        *state.captured_body.lock().unwrap() = Some(body.to_vec());
        StatusCode::OK
    }

    fn make_body_auth_target(base_url: String) -> ProxyTarget {
        let now = Utc::now();
        ProxyTarget {
            base_url: base_url.clone(),
            auth_method: "body".to_string(),
            auth_key_name: "app_secret".to_string(),
            credential: "super-secret".to_string(),
            service: DownstreamService {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Body Auth Service".to_string(),
                slug: "body-auth-service".to_string(),
                description: None,
                base_url,
                auth_method: "body".to_string(),
                auth_key_name: "app_secret".to_string(),
                credential_encrypted: vec![],
                auth_type: None,
                openapi_spec_url: None,
                asyncapi_spec_url: None,
                streaming_supported: false,
                ssh_config: None,
                service_type: "http".to_string(),
                visibility: "public".to_string(),
                oauth_client_id: None,
                service_category: "external".to_string(),
                requires_user_credential: true,
                is_active: true,
                created_by: "test".to_string(),
                identity_propagation_mode: "none".to_string(),
                identity_include_user_id: false,
                identity_include_email: false,
                identity_include_name: false,
                identity_jwt_audience: None,
                forward_access_token: false,
                inject_delegation_token: false,
                delegation_token_scope: "llm:proxy".to_string(),
                provider_config_id: None,
                homepage_url: None,
                repository_url: None,
                issues_url: None,
                capabilities: None,
                auth_notes: None,
                known_limitations: None,
                required_permissions: None,
                recommended_skills: None,
                examples_url: None,
                custom_user_agent: None,
                developer_app_ids: None,
                token_exchange_config: None,
                created_at: now,
                updated_at: now,
            },
        }
    }

    #[tokio::test]
    async fn body_auth_skips_injection_on_get_request() {
        let state = BodyAuthCaptureState::default();
        let app = Router::new()
            .route("/chats", get(body_capture_get))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test app");
        });

        let response = forward_request(
            &Client::new(),
            &make_body_auth_target(format!("http://{addr}")),
            reqwest::Method::GET,
            "chats",
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(None),
            vec![],
            vec![],
            None,
            &empty_token_cache(),
        )
        .await
        .expect("GET with body auth should forward without body injection");

        assert_eq!(response.status(), StatusCode::OK);
        // The downstream must receive an empty body -- the secret must NOT
        // leak into a GET request via the body injection code path.
        let captured = state.captured_body.lock().unwrap().clone();
        assert_eq!(captured.as_deref(), Some(&b""[..]));

        server.abort();
    }

    // ─── old HTTP flow regression test (PR #220 backward-compat) ─────

    async fn body_capture_post(
        State(state): State<BodyAuthCaptureState>,
        body: Bytes,
    ) -> StatusCode {
        *state.captured_body.lock().unwrap() = Some(body.to_vec());
        StatusCode::OK
    }

    #[tokio::test]
    async fn body_auth_still_merges_secret_into_post_body_after_refactor() {
        // Regression: the #220 refactor introduced the generic
        // `token_exchange` auth method and migrated the `api-lark-bot`
        // catalog row. Users who had already run `nyxid service add
        // api-lark-bot` under #205 still have UserService rows with
        // `auth_method: "body"` and UserApiKey rows with a raw
        // `app_secret` string. Their existing integration hits the
        // proxy POSTing `{"app_id": "cli_xxx"}` to the Lark token
        // exchange endpoint and expects NyxID to merge `app_secret`
        // into the body server-side -- that's the whole contract of
        // the #205 body-injection flow.
        //
        // This test replays that exact request shape end-to-end
        // through `forward_request` and asserts the downstream sees
        // the merged body. If someone later rips out the body-auth
        // arm thinking it's obsolete, this test fails.
        let state = BodyAuthCaptureState::default();
        let app = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post(body_capture_post),
            )
            .with_state(state.clone());

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
            "application/json".parse().unwrap(),
        );

        let response = forward_request(
            &Client::new(),
            &make_body_auth_target(format!("http://{addr}")),
            reqwest::Method::POST,
            "open-apis/auth/v3/tenant_access_token/internal",
            None,
            headers,
            ProxyBody::Buffered(Some(bytes::Bytes::from_static(br#"{"app_id":"cli_xxx"}"#))),
            vec![],
            vec![],
            None,
            &empty_token_cache(),
        )
        .await
        .expect("POST with body auth should merge credential into JSON body");

        assert_eq!(response.status(), StatusCode::OK);

        // The downstream must see the merged body. `app_secret` comes
        // from `make_body_auth_target` which uses "super-secret".
        let captured_raw = state.captured_body.lock().unwrap().clone();
        let captured_bytes = captured_raw.expect("downstream captured a body");
        let parsed: serde_json::Value =
            serde_json::from_slice(&captured_bytes).expect("downstream body is valid JSON");
        assert_eq!(parsed["app_id"], "cli_xxx");
        assert_eq!(parsed["app_secret"], "super-secret");

        server.abort();
    }

    #[test]
    fn build_minimal_downstream_service_preserves_body_auth_method() {
        // Regression: existing UserService rows with auth_method=body
        // (from the #205 api-lark-bot seed) must keep their auth_method
        // when the proxy builds the synthetic DownstreamService at
        // request time. If the resolver accidentally promoted them to
        // token_exchange based on the catalog row, their existing
        // credential (raw app_secret string, not JSON) would fail to
        // parse and the proxy would 500.
        let mut user_service = make_user_service_token_exchange();
        user_service.auth_method = "body".to_string();
        user_service.auth_key_name = "app_secret".to_string();
        let endpoint = make_endpoint();

        let svc = build_minimal_downstream_service(&user_service, &endpoint, Utc::now(), None);

        assert_eq!(svc.auth_method, "body");
        assert_eq!(svc.auth_key_name, "app_secret");
        assert!(svc.token_exchange_config.is_none());
    }
}
