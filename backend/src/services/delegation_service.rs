use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
};
use crate::services::user_token_service;

/// A resolved credential ready for injection into a proxied request.
#[derive(Clone)]
pub struct DelegatedCredential {
    #[allow(dead_code)]
    pub provider_slug: String,
    pub injection_method: String,
    pub injection_key: String,
    pub credential: String,
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
/// LEGACY-PATH ONLY — multi-connection invariant.
/// This function reads `UserProviderToken` rows via
/// `user_token_service::get_active_token`, which is keyed by
/// `(user_id, provider_config_id)` and has no notion of `connection_id`.
/// That is correct because **every caller gates this behind the legacy
/// `DownstreamService` path**: `handlers/proxy.rs` skips it when
/// `resolved_user_service_id.is_some()`, `handlers/llm_gateway.rs` skips
/// it when `resolved_via_user_service`, and `services/mcp_service.rs`
/// skips it for `McpToolSource::UserManaged`. A multi-connection
/// `UserApiKey` (`connection_id: Some`) only ever backs a new-path
/// `UserService`, so it can never reach this function — its token is
/// injected directly from `target.credential` by the proxy.
///
/// If a future refactor ever routed a multi-connection request here, the
/// failure mode is *safe*: `get_active_token` returns `NotFound` (no
/// `user_provider_tokens` row exists for a connection-scoped key), which
/// surfaces as an explicit `BadRequest("... connection required")` for
/// required providers — never a silent wrong-credential injection.
pub async fn resolve_delegated_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service_id: &str,
) -> AppResult<Vec<DelegatedCredential>> {
    // Load all requirements for this service
    let requirements: Vec<ServiceProviderRequirement> = db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .find(doc! { "service_id": service_id })
        .await?
        .try_collect()
        .await?;

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

    for req in &requirements {
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
            credential,
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
            credential: "secret-token".to_string(),
        };
        let cloned = cred.clone();
        assert_eq!(cloned.injection_method, "bearer");
        assert_eq!(cloned.credential, "secret-token");
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
            id: provider_id.clone(),
            slug: "github".to_string(),
            name: "GitHub".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://github.com/login/oauth/authorize".to_string()),
            token_url: Some("https://github.com/login/oauth/access_token".to_string()),
            revocation_url: None,
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
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "test".to_string(),
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
            resolve_delegated_credentials(&db, &encryption_keys, &org_user_id, &service_id)
                .await
                .unwrap();

        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].provider_slug, "github");
        assert_eq!(credentials[0].injection_method, "bearer");
        assert_eq!(credentials[0].injection_key, "Authorization");
        assert_eq!(credentials[0].credential, "org-access-token");
    }
}
