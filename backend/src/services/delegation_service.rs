use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
};
use crate::services::connection_expiry_service::ConnectionExpiryNotifier;
use crate::services::user_token_service;

/// A resolved credential ready for injection into a proxied request.
#[derive(Clone)]
pub struct DelegatedCredential {
    #[allow(dead_code)]
    pub provider_slug: String,
    pub injection_method: String,
    pub injection_key: String,
    pub credential: zeroize::Zeroizing<String>,
}

fn normalize_delegated_injection(
    provider_slug: &str,
    injection_method: &str,
    injection_key: Option<&str>,
) -> (String, String) {
    if provider_slug == "telegram-bot" {
        return ("path".to_string(), "bot".to_string());
    }

    let key = injection_key
        .map(String::from)
        .unwrap_or_else(|| match injection_method {
            "bearer" => "Authorization".to_string(),
            "query" => "api_key".to_string(),
            "path" => String::new(),
            _ => "X-API-Key".to_string(),
        });

    (injection_method.to_string(), key)
}

/// Resolve all provider tokens needed for a downstream service.
/// Returns credentials ready for injection. Returns an error if a required
/// provider token is missing (CR-15).
///
/// Uses batch queries for provider lookups (CR-4/5/6: fix N+1).
///
/// Legacy provider-token semantics: lookup uses `(owner, provider_config_id)`
/// in `UserProviderToken`, never an arbitrary modern `UserApiKey` or connection.
/// Explicit UserService credentials are injected directly from the resolved
/// target. Automatic no-auth connections use the same token materialization
/// only through `resolve_optional_auto_provision_credentials`, which verifies
/// persisted connection, owner, destination and optional-only eligibility.
pub async fn resolve_delegated_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service_id: &str,
    connection_expiry_notifier: Option<&ConnectionExpiryNotifier>,
) -> AppResult<Vec<DelegatedCredential>> {
    // Load all requirements for this service
    let requirements: Vec<ServiceProviderRequirement> = db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .find(doc! { "service_id": service_id })
        .await?
        .try_collect()
        .await?;

    materialize_provider_requirements(
        db,
        encryption_keys,
        user_id,
        service_id,
        &requirements,
        connection_expiry_notifier,
    )
    .await
}

/// Resolve optional legacy provider tokens for an automatic no-auth connection.
/// The caller supplies the effective owner already authorized by route resolution.
/// Re-read the persisted connection and bind the held target to its catalog and
/// endpoint before touching tokens. Explicit, node-managed, platform-bound and
/// credential-backed connections retain their existing credential path.
pub(crate) async fn resolve_optional_auto_provision_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    effective_owner_id: &str,
    user_service_id: &str,
    catalog_service_id: &str,
    target: &super::proxy_service::ProxyTarget,
    connection_expiry_notifier: Option<&ConnectionExpiryNotifier>,
) -> AppResult<Vec<DelegatedCredential>> {
    let snapshot = read_optional_auto_provision_snapshot(
        db,
        effective_owner_id,
        user_service_id,
        catalog_service_id,
        target,
    )
    .await?;
    materialize_optional_auto_provision_snapshot(
        db,
        encryption_keys,
        target,
        &snapshot,
        connection_expiry_notifier,
    )
    .await
}

pub(crate) struct OptionalAutoProvisionSnapshot {
    pub digest: Option<String>,
    effective_owner_id: String,
    user_service_id: String,
    catalog_service_id: String,
    bindings: Vec<OptionalProviderBinding>,
}

struct OptionalProviderBinding {
    provider_slug: String,
    injection_method: String,
    injection_key: String,
    token: Option<crate::models::user_provider_token::UserProviderToken>,
}

/// Use the exact requirement/provider policy and token lineage captured by the
/// metadata-only snapshot, rather than selecting them again after approval.
pub(crate) async fn materialize_optional_auto_provision_snapshot(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    target: &super::proxy_service::ProxyTarget,
    snapshot: &OptionalAutoProvisionSnapshot,
    notifier: Option<&ConnectionExpiryNotifier>,
) -> AppResult<Vec<DelegatedCredential>> {
    if snapshot.digest.is_none() {
        return Ok(Vec::new());
    }
    let mut credentials = Vec::new();
    for binding in &snapshot.bindings {
        let Some(token) = &binding.token else {
            continue;
        };
        let decrypted = match user_token_service::get_active_token_at_revision(
            db,
            encryption_keys,
            token,
            notifier,
        )
        .await
        {
            Ok(token) => token,
            Err(error @ AppError::Conflict(_)) => return Err(error),
            Err(_) => continue, // Optional unavailable credentials remain optional.
        };
        let credential = match decrypted.token_type.as_str() {
            "api_key" => decrypted.api_key,
            "oauth2" => decrypted.access_token,
            _ => None,
        };
        if let Some(credential) = credential {
            credentials.push(DelegatedCredential {
                provider_slug: binding.provider_slug.clone(),
                injection_method: binding.injection_method.clone(),
                injection_key: binding.injection_key.clone(),
                credential: credential.into(),
            });
        }
    }
    // Refresh can await a provider. Recheck all live route gates and authority
    // before returning the vector, whose policy and lineage stayed pinned.
    let live = read_optional_auto_provision_snapshot(
        db,
        &snapshot.effective_owner_id,
        &snapshot.user_service_id,
        &snapshot.catalog_service_id,
        target,
    )
    .await?;
    if live.digest != snapshot.digest {
        return Err(AppError::Conflict(
            "Optional provider authority changed during resolution".into(),
        ));
    }
    Ok(credentials)
}

/// Metadata-only authority for the automatic optional-provider exception.
/// Never decrypts, refreshes, or touches usage timestamps. The separate contract
/// leaves the real exact-execution v1/v2 projections byte-compatible.
pub(crate) async fn read_optional_auto_provision_snapshot(
    db: &mongodb::Database,
    effective_owner_id: &str,
    user_service_id: &str,
    catalog_service_id: &str,
    target: &super::proxy_service::ProxyTarget,
) -> AppResult<OptionalAutoProvisionSnapshot> {
    use crate::models::user_provider_token::{COLLECTION_NAME as TOKENS, UserProviderToken};
    let mut requirements = optional_auto_provision_requirements(
        db,
        effective_owner_id,
        user_service_id,
        catalog_service_id,
        target,
    )
    .await?;
    if requirements.is_empty() {
        return Ok(OptionalAutoProvisionSnapshot {
            digest: None,
            effective_owner_id: effective_owner_id.to_owned(),
            user_service_id: user_service_id.to_owned(),
            catalog_service_id: catalog_service_id.to_owned(),
            bindings: Vec::new(),
        });
    }
    requirements.sort_by(|a, b| {
        a.provider_config_id
            .cmp(&b.provider_config_id)
            .then(a.id.cmp(&b.id))
    });
    // Two batch reads preserve the legacy unique (owner, provider) selection
    // while keeping snapshot work independent of the number of requirements.
    let provider_ids: Vec<&str> = requirements
        .iter()
        .map(|r| r.provider_config_id.as_str())
        .collect();
    let providers: Vec<ProviderConfig> = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find(doc! { "_id": { "$in": &provider_ids } })
        .await?
        .try_collect()
        .await?;
    let mut providers: HashMap<String, ProviderConfig> =
        providers.into_iter().map(|p| (p.id.clone(), p)).collect();
    let active_provider_ids: Vec<&str> = providers
        .values()
        .filter(|p| p.is_active)
        .map(|p| p.id.as_str())
        .collect();
    // Match get_active_token's established legacy selection exactly.
    let tokens: Vec<UserProviderToken> = if active_provider_ids.is_empty() {
        Vec::new()
    } else {
        db.collection::<UserProviderToken>(TOKENS)
            .find(doc! {
                "user_id": effective_owner_id,
                "provider_config_id": { "$in": &active_provider_ids },
                "status": { "$in": ["active", "expired"] },
            })
            .await?
            .try_collect()
            .await?
    };
    let mut tokens: HashMap<String, UserProviderToken> = tokens
        .into_iter()
        .map(|t| (t.provider_config_id.clone(), t))
        .collect();
    let mut projection = Vec::with_capacity(requirements.len());
    let mut bindings = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        let provider = providers.remove(&requirement.provider_config_id);
        let active = provider.as_ref().is_some_and(|p| p.is_active);
        let token = tokens.remove(&requirement.provider_config_id);
        let injection = provider.as_ref().map(|p| {
            normalize_delegated_injection(
                &p.slug,
                &requirement.injection_method,
                requirement.injection_key.as_deref(),
            )
        });
        projection.push(serde_json::json!({
            "provider_config_id": requirement.provider_config_id,
            "required": requirement.required,
            "scopes": requirement.scopes,
            "provider_slug": provider.as_ref().map(|p| &p.slug),
            "provider_active": active,
            "injection": injection,
            "token": token.as_ref().map(|t| serde_json::json!({
                "id": t.id,
                "state_version": t.state_version,
                "token_type": t.token_type,
                // Presence is authority; ciphertext and refresh timestamps are not.
                "material_present": match t.token_type.as_str() {
                    "api_key" => t.api_key_encrypted.is_some(),
                    "oauth2" => t.access_token_encrypted.is_some(),
                    _ => false,
                },
            })),
        }));
        if active
            && let Some(provider) = provider
            && let Some((method, key)) = injection
        {
            bindings.push(OptionalProviderBinding {
                provider_slug: provider.slug,
                injection_method: method,
                injection_key: key,
                token,
            });
        }
    }
    Ok(OptionalAutoProvisionSnapshot {
        effective_owner_id: effective_owner_id.to_owned(),
        user_service_id: user_service_id.to_owned(),
        catalog_service_id: catalog_service_id.to_owned(),
        bindings,
        digest: Some(super::mcp_service::canonical_sha256(serde_json::json!({
            "contract_version": "nyxid-exact-optional-provider-authority.v1",
            "owner_id": effective_owner_id,
            "user_service_id": user_service_id,
            "catalog_service_id": catalog_service_id,
            "requirements": projection,
        }))),
    })
}

async fn optional_auto_provision_requirements(
    db: &mongodb::Database,
    effective_owner_id: &str,
    user_service_id: &str,
    catalog_service_id: &str,
    target: &super::proxy_service::ProxyTarget,
) -> AppResult<Vec<ServiceProviderRequirement>> {
    use crate::models::{
        downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
        user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint},
        user_service::{AUTO_PROVISION_SOURCE, COLLECTION_NAME as SERVICES, UserService},
    };

    // Most explicit connections can be excluded from the resolved target alone.
    // Keep that hot path free of an extra database read.
    if target.auth_method != "none"
        || !target.credential.is_empty()
        || target.connection_id.is_some()
    {
        return Ok(vec![]);
    }
    let Some(connection) = super::service_history::collection::<UserService>(db, SERVICES)
        .find_one(doc! { "_id": user_service_id })
        .await?
    else {
        return Ok(vec![]);
    };
    if connection.source.as_deref() != Some(AUTO_PROVISION_SOURCE)
        || !connection.is_active
        || connection.deleted_at.is_some()
        || connection.auth_method != "none"
        || connection.api_key_id.is_some()
        || connection.node_id.is_some()
        || connection.service_type != "http"
        || connection.credential_binding.as_deref() == Some("platform")
        || connection.user_id != effective_owner_id
        || connection.catalog_service_id.as_deref() != Some(catalog_service_id)
    {
        return Ok(vec![]);
    }

    let unavailable = || AppError::NotFound("Service is no longer available".into());
    let catalog = db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! { "_id": catalog_service_id })
        .await?
        .ok_or_else(unavailable)?;
    if catalog.platform_key.is_some() {
        return Ok(vec![]);
    }
    super::proxy_service::verify_auto_provision_eligibility(db, &connection, effective_owner_id)
        .await?;
    let requirements: Vec<ServiceProviderRequirement> = db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .find(doc! { "service_id": catalog_service_id })
        .await?
        .try_collect()
        .await?;
    // Do not silently filter away a requirement made mandatory after target
    // resolution. Reject the stale automatic route before materializing tokens.
    if requirements.iter().any(|r| r.required) {
        return Err(unavailable());
    }
    // With no provider credentials to inherit, preserve ordinary no-auth
    // routing, including existing endpoints after a catalog URL edit.
    if requirements.is_empty() {
        return Ok(vec![]);
    }
    let endpoint = db
        .collection::<UserEndpoint>(ENDPOINTS)
        .find_one(doc! { "_id": &connection.endpoint_id })
        .await?
        .ok_or_else(unavailable)?;
    // Automatic no-auth routes use the catalog destination. Named target
    // selection requires a credential-bearing route and is not this exception.
    if endpoint.user_id != effective_owner_id
        || endpoint.catalog_service_id.as_deref() != Some(catalog_service_id)
        || endpoint.url != catalog.base_url
        || target.base_url != endpoint.url
        || target.service.base_url != endpoint.url
        || target.service.id != catalog_service_id
        || target.service.slug != connection.slug
        || target.auth_key_name != connection.auth_key_name
        || target.target_id.is_some()
    {
        return Err(unavailable());
    }
    Ok(requirements)
}

async fn materialize_provider_requirements(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service_id: &str,
    requirements: &[ServiceProviderRequirement],
    connection_expiry_notifier: Option<&ConnectionExpiryNotifier>,
) -> AppResult<Vec<DelegatedCredential>> {
    if requirements.is_empty() {
        return Ok(vec![]);
    }

    // Batch fetch all providers in a single query (fix N+1)
    let provider_ids: Vec<&str> = requirements
        .iter()
        .map(|r| r.provider_config_id.as_str())
        .collect();
    let providers: Vec<ProviderConfig> = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find(doc! { "_id": { "$in": &provider_ids }, "is_active": true })
        .await?
        .try_collect()
        .await?;
    let provider_map: HashMap<&str, &ProviderConfig> =
        providers.iter().map(|p| (p.id.as_str(), p)).collect();

    let mut credentials = Vec::new();

    for req in requirements {
        let provider = match provider_map.get(req.provider_config_id.as_str()) {
            Some(p) => *p,
            None => {
                if req.required {
                    return Err(AppError::BadRequest(format!(
                        "Required provider '{}' is not found or inactive. Please contact your admin.",
                        req.provider_config_id
                    )));
                }
                continue;
            }
        };

        // Try to get the user's active token for this provider
        let token_result = user_token_service::get_active_token(
            db,
            encryption_keys,
            user_id,
            &req.provider_config_id,
            connection_expiry_notifier,
        )
        .await;

        let decrypted = match token_result {
            Ok(t) => t,
            Err(e) => {
                if req.required {
                    // CR-15: Return error for required providers without tokens
                    return Err(AppError::BadRequest(format!(
                        "Provider '{}' connection required. Please connect your {} account first.",
                        provider.name, provider.name
                    )));
                }
                tracing::debug!(
                    service_id = %service_id,
                    provider_slug = %provider.slug,
                    error = %e,
                    "Optional provider token not available"
                );
                continue;
            }
        };

        // Determine the credential value
        let credential_value = match decrypted.token_type.as_str() {
            "api_key" => decrypted.api_key,
            "oauth2" => decrypted.access_token,
            _ => None,
        };

        let credential = match credential_value {
            Some(c) => c,
            None => continue,
        };

        let (injection_method, injection_key) = normalize_delegated_injection(
            &provider.slug,
            &req.injection_method,
            req.injection_key.as_deref(),
        );

        credentials.push(DelegatedCredential {
            provider_slug: provider.slug.clone(),
            injection_method,
            injection_key,
            credential: credential.into(),
        });
    }

    Ok(credentials)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{normalize_delegated_injection, resolve_delegated_credentials};
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
    };
    use crate::models::user_provider_token::{
        COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
    };
    use crate::test_utils::{connect_test_database, test_encryption_keys};

    #[test]
    fn telegram_bot_legacy_bearer_injection_is_mapped_to_path() {
        let (method, key) =
            normalize_delegated_injection("telegram-bot", "bearer", Some("Authorization"));

        assert_eq!(method, "path");
        assert_eq!(key, "bot");
    }

    #[test]
    fn telegram_bot_custom_prefix_is_ignored_and_canonicalized() {
        let (method, key) = normalize_delegated_injection("telegram-bot", "path", Some("custom"));

        assert_eq!(method, "path");
        assert_eq!(key, "bot");
    }

    #[test]
    fn standard_bearer_injection_keeps_default_header() {
        let (method, key) = normalize_delegated_injection("github", "bearer", None);

        assert_eq!(method, "bearer");
        assert_eq!(key, "Authorization");
    }

    #[test]
    fn query_injection_defaults_to_api_key() {
        let (method, key) = normalize_delegated_injection("openai", "query", None);
        assert_eq!(method, "query");
        assert_eq!(key, "api_key");
    }

    #[test]
    fn path_injection_defaults_to_empty_key() {
        let (method, key) = normalize_delegated_injection("openai", "path", None);
        assert_eq!(method, "path");
        assert_eq!(key, "");
    }

    #[test]
    fn unknown_method_defaults_to_x_api_key() {
        let (method, key) = normalize_delegated_injection("custom", "header", None);
        assert_eq!(method, "header");
        assert_eq!(key, "X-API-Key");
    }

    #[test]
    fn explicit_key_overrides_default() {
        let (method, key) =
            normalize_delegated_injection("openai", "bearer", Some("X-Custom-Auth"));
        assert_eq!(method, "bearer");
        assert_eq!(key, "X-Custom-Auth");
    }

    #[test]
    fn query_with_explicit_key() {
        let (method, key) = normalize_delegated_injection("anthropic", "query", Some("x-api-key"));
        assert_eq!(method, "query");
        assert_eq!(key, "x-api-key");
    }

    #[test]
    fn telegram_bot_ignores_all_overrides() {
        let (method, key) = normalize_delegated_injection("telegram-bot", "query", Some("token"));
        assert_eq!(method, "path");
        assert_eq!(key, "bot");
    }

    #[test]
    fn delegated_credential_clone() {
        let cred = super::DelegatedCredential {
            provider_slug: "github".to_string(),
            injection_method: "bearer".to_string(),
            injection_key: "Authorization".to_string(),
            credential: "secret-token".to_string().into(),
        };
        let cloned = cred.clone();
        assert_eq!(cloned.injection_method, "bearer");
        assert_eq!(cloned.credential.as_str(), "secret-token");
    }

    #[tokio::test]
    async fn resolve_delegated_credentials_uses_org_owner_tokens_when_passed_owner_id() {
        let Some(db) = connect_test_database("delegation_service").await else {
            eprintln!("skipping delegation_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let now = Utc::now();
        let actor_user_id = uuid::Uuid::new_v4().to_string();
        let org_user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let provider = ProviderConfig {
            secret_context_versions: Default::default(),
            id: provider_id.clone(),
            slug: "github".to_string(),
            name: "GitHub".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://github.com/login/oauth/authorize".to_string()),
            token_url: Some("https://github.com/login/oauth/access_token".to_string()),
            revocation_url: None,
            revocation: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            token_request_encoding: None,
            oauth_request_headers: Default::default(),
            supports_oauth_scopes: true,
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            authorization_nonce_required: false,
            requires_gateway_url: false,
            created_by: "test".to_string(),
            revocation_seed_version: 0,
            created_at: now,
            updated_at: now,
        };
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();

        let requirement = ServiceProviderRequirement {
            id: uuid::Uuid::new_v4().to_string(),
            service_id: service_id.clone(),
            provider_config_id: provider_id.clone(),
            required: true,
            scopes: None,
            injection_method: "bearer".to_string(),
            injection_key: Some("Authorization".to_string()),
            created_at: now,
            updated_at: now,
        };
        db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
            .insert_one(&requirement)
            .await
            .unwrap();

        let token = UserProviderToken {
            secret_context_versions: Default::default(),
            id: uuid::Uuid::new_v4().to_string(),
            user_id: org_user_id.clone(),
            provider_config_id: provider_id,
            connection_id: None,
            credential_user_id: None,
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(
                encryption_keys.encrypt(b"org-access-token").await.unwrap(),
            ),
            refresh_token_encrypted: None,
            token_scopes: Some("repo".to_string()),
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            state_version: 1,
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: Some("Org GitHub".to_string()),
            metadata: Some(
                [("actor_user_id".to_string(), actor_user_id)]
                    .into_iter()
                    .collect(),
            ),
            gateway_url: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(&token)
            .await
            .unwrap();

        let credentials =
            resolve_delegated_credentials(&db, &encryption_keys, &org_user_id, &service_id, None)
                .await
                .unwrap();

        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].provider_slug, "github");
        assert_eq!(credentials[0].injection_method, "bearer");
        assert_eq!(credentials[0].injection_key, "Authorization");
        assert_eq!(credentials[0].credential.as_str(), "org-access-token");
    }
}
