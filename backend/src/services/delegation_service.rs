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
pub struct DelegatedCredential {
    #[allow(dead_code)]
    pub provider_slug: String,
    pub injection_method: String,
    pub injection_key: String,
    pub credential: String,
}

/// Resolve all provider tokens needed for a downstream service.
/// Returns credentials ready for injection. Returns an error if a required
/// provider token is missing (CR-15).
///
/// Uses batch queries for provider lookups (CR-4/5/6: fix N+1).
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

        let injection_key =
            req.injection_key
                .clone()
                .unwrap_or_else(|| match req.injection_method.as_str() {
                    "bearer" => "Authorization".to_string(),
                    "query" => "api_key".to_string(),
                    _ => "X-API-Key".to_string(),
                });

        credentials.push(DelegatedCredential {
            provider_slug: provider.slug.clone(),
            injection_method: req.injection_method.clone(),
            injection_key,
            credential,
        });
    }

    Ok(credentials)
}
