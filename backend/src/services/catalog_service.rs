use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, ServiceCapabilities,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
};

/// A catalog entry combining DownstreamService + ProviderConfig info.
pub struct CatalogEntry {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub provider_config_id: Option<String>,
    pub provider_type: Option<String>,
    pub requires_gateway_url: bool,
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
    pub credential_mode: Option<String>,
    // SSH fields
    pub service_type: String,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub ssh_ca_public_key: Option<String>,
    pub ssh_allowed_principals: Option<Vec<String>>,
    pub ssh_certificate_ttl_minutes: Option<u32>,
    // OAuth config fields (for node-native OAuth)
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub device_code_url: Option<String>,
    pub device_verification_url: Option<String>,
    pub device_token_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub supports_pkce: bool,
    pub device_code_format: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub extra_auth_params: Option<HashMap<String, String>>,
    pub oauth_client_id: Option<String>,
    pub client_id_param_name: Option<String>,
    /// Whether this catalog entry needs credential setup instead of direct no-auth access.
    pub requires_credential: bool,
    // --- Rich metadata for AI agent discovery ---
    pub openapi_spec_url: Option<String>,
    pub asyncapi_spec_url: Option<String>,
    pub homepage_url: Option<String>,
    pub repository_url: Option<String>,
    pub issues_url: Option<String>,
    pub capabilities: Option<ServiceCapabilities>,
    pub auth_notes: Option<String>,
    pub known_limitations: Option<String>,
    pub required_permissions: Option<Vec<String>>,
    pub examples_url: Option<String>,
    pub recommended_skills: Option<Vec<String>>,
    /// Declared credential fields for `token_exchange` services. When set,
    /// clients should render one input per field (text vs password per the
    /// `secret` flag) and compose a JSON object from the values before
    /// submitting. The full `TokenExchangeConfig` stays server-side --
    /// clients never see the endpoint URL, request template, or injection
    /// format, only what to collect from the user.
    pub token_exchange_credential_fields:
        Option<Vec<crate::models::downstream_service::CredentialFieldSpec>>,
}

fn build_catalog_entry(
    svc: DownstreamService,
    provider: Option<&ProviderConfig>,
    spr: Option<&ServiceProviderRequirement>,
    oauth_client_id: Option<String>,
) -> CatalogEntry {
    // A service requires a credential if:
    // 1. It requires per-user credentials (connection services), OR
    // 2. It has an actual auth method (not "none"), OR
    // 3. It has auth_method "none" but an SPR exists (uses master credentials)
    let requires_credential =
        svc.requires_user_credential || svc.auth_method != "none" || spr.is_some();
    CatalogEntry {
        service_type: svc.service_type.clone(),
        ssh_host: svc.ssh_config.as_ref().map(|c| c.host.clone()),
        ssh_port: svc.ssh_config.as_ref().map(|c| c.port),
        ssh_ca_public_key: svc
            .ssh_config
            .as_ref()
            .and_then(|c| c.ca_public_key.clone()),
        ssh_allowed_principals: svc
            .ssh_config
            .as_ref()
            .map(|c| c.allowed_principals.clone()),
        ssh_certificate_ttl_minutes: svc.ssh_config.as_ref().map(|c| c.certificate_ttl_minutes),
        slug: svc.slug,
        name: svc.name,
        description: svc.description,
        base_url: svc.base_url,
        // For internal services (auth_method="none"), resolve actual injection
        // from ServiceProviderRequirement (e.g., bearer/Authorization, header/x-api-key, query/key)
        auth_method: if svc.auth_method == "none" {
            spr.map(|r| r.injection_method.clone())
                .unwrap_or_else(|| svc.auth_method)
        } else {
            svc.auth_method
        },
        auth_key_name: if svc.auth_key_name.is_empty() {
            spr.and_then(|r| r.injection_key.clone())
                .unwrap_or_else(|| "Authorization".to_string())
        } else {
            svc.auth_key_name
        },
        provider_config_id: provider.map(|p| p.id.clone()),
        provider_type: provider.map(|p| p.provider_type.clone()),
        requires_gateway_url: provider.is_some_and(|p| p.requires_gateway_url),
        api_key_instructions: provider.and_then(|p| p.api_key_instructions.clone()),
        api_key_url: provider.and_then(|p| p.api_key_url.clone()),
        icon_url: provider.and_then(|p| p.icon_url.clone()),
        documentation_url: provider.and_then(|p| p.documentation_url.clone()),
        credential_mode: provider.map(|p| p.credential_mode.clone()),
        // OAuth config
        authorization_url: provider.and_then(|p| p.authorization_url.clone()),
        token_url: provider.and_then(|p| p.token_url.clone()),
        device_code_url: provider.and_then(|p| p.device_code_url.clone()),
        device_verification_url: provider.and_then(|p| p.device_verification_url.clone()),
        device_token_url: provider.and_then(|p| p.device_token_url.clone()),
        default_scopes: provider.and_then(|p| p.default_scopes.clone()),
        supports_pkce: provider.is_some_and(|p| p.supports_pkce),
        device_code_format: provider.map(|p| p.device_code_format.clone()),
        token_endpoint_auth_method: provider.map(|p| p.token_endpoint_auth_method.clone()),
        extra_auth_params: provider.and_then(|p| p.extra_auth_params.clone()),
        oauth_client_id,
        client_id_param_name: provider.and_then(|p| p.client_id_param_name.clone()),
        requires_credential,
        openapi_spec_url: svc.openapi_spec_url,
        asyncapi_spec_url: svc.asyncapi_spec_url,
        homepage_url: svc.homepage_url,
        repository_url: svc.repository_url,
        issues_url: svc.issues_url,
        capabilities: svc.capabilities,
        auth_notes: svc.auth_notes,
        known_limitations: svc.known_limitations,
        required_permissions: svc.required_permissions,
        examples_url: svc.examples_url,
        recommended_skills: svc.recommended_skills,
        token_exchange_credential_fields: svc.token_exchange_config.map(|c| c.credential_fields),
    }
}

async fn decrypt_provider_client_id(
    provider: &ProviderConfig,
    encryption_keys: &EncryptionKeys,
) -> AppResult<Option<String>> {
    let Some(encrypted) = provider.client_id_encrypted.as_ref() else {
        return Ok(None);
    };

    let decrypted = encryption_keys.decrypt(encrypted).await?;
    let client_id = String::from_utf8(decrypted)
        .map_err(|_| AppError::Internal("Failed to decode provider client_id".to_string()))?;
    if client_id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(client_id))
    }
}

/// MongoDB filter for visibility that hides private services from non-owners.
/// Public services and legacy documents without a visibility field are visible to all.
fn visibility_filter(user_id: &str) -> mongodb::bson::Document {
    doc! {
        "$or": [
            { "visibility": { "$ne": "private" } },
            { "visibility": { "$exists": false } },
            { "visibility": "private", "created_by": user_id },
        ],
    }
}

/// MongoDB filter for service_category that handles legacy documents
/// created before the field was added (defaults to "connection").
fn legacy_service_category_filter(categories: &[&str]) -> mongodb::bson::Document {
    doc! {
        "$or": categories.iter().map(|c| doc! { "service_category": c }).chain(
            std::iter::once(doc! { "service_category": { "$exists": false } })
        ).collect::<Vec<_>>(),
    }
}

/// List catalog entries available for user key creation.
/// Filters to connection-category + provider-linked services.
pub async fn list_catalog(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
) -> AppResult<Vec<CatalogEntry>> {
    // Legacy documents may lack requires_user_credential (defaults to true)
    // and service_category (defaults to "connection").
    list_catalog_filtered(
        db,
        encryption_keys,
        doc! {
            "service_type": "http",
            "is_active": true,
            "$and": [
                {
                    "$or": [
                        { "requires_user_credential": true },
                        { "requires_user_credential": { "$exists": false } },
                        { "provider_config_id": { "$ne": null } },
                    ],
                },
                legacy_service_category_filter(&["connection", "internal"]),
            ],
        },
    )
    .await
}

/// List ALL active catalog entries for discovery (includes system services without auth).
/// Enforces visibility: private services only visible to their creator.
pub async fn list_catalog_all(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
) -> AppResult<Vec<CatalogEntry>> {
    let filter = doc! {
        "is_active": true,
        "$and": [
            legacy_service_category_filter(&["connection", "internal"]),
            visibility_filter(user_id),
        ],
    };
    list_catalog_filtered(db, encryption_keys, filter).await
}

async fn list_catalog_filtered(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    filter: mongodb::bson::Document,
) -> AppResult<Vec<CatalogEntry>> {
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    // Batch-load all referenced provider configs
    let provider_ids: Vec<&str> = services
        .iter()
        .filter_map(|s| s.provider_config_id.as_deref())
        .collect();

    let providers: Vec<ProviderConfig> = if provider_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find(doc! { "_id": { "$in": &provider_ids } })
            .await?
            .try_collect()
            .await?
    };

    // Batch-load service provider requirements to get actual auth injection config
    let svc_ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    let sprs: Vec<ServiceProviderRequirement> = if svc_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
            .find(doc! { "service_id": { "$in": &svc_ids } })
            .await?
            .try_collect()
            .await?
    };

    let mut resolved_entries = Vec::with_capacity(services.len());
    for svc in services {
        let provider = svc
            .provider_config_id
            .as_ref()
            .and_then(|pid| providers.iter().find(|p| &p.id == pid));

        let spr = sprs.iter().find(|r| r.service_id == svc.id);

        let oauth_client_id = match provider {
            Some(provider) if provider.credential_mode != "user" => {
                decrypt_provider_client_id(provider, encryption_keys).await?
            }
            _ => None,
        };

        resolved_entries.push(build_catalog_entry(svc, provider, spr, oauth_client_id));
    }

    Ok(resolved_entries)
}

/// Get the raw DownstreamService by slug (lightweight, no provider/encryption lookup).
/// Enforces visibility: private services only visible to their creator.
pub async fn get_downstream_service_by_slug(
    db: &mongodb::Database,
    slug: &str,
    user_id: &str,
) -> AppResult<DownstreamService> {
    let mut filter = doc! { "slug": slug, "is_active": true };
    filter.extend(visibility_filter(user_id));
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::NotFound("Catalog entry not found".to_string()))
}

/// Get single catalog entry by slug.
pub async fn get_catalog_entry(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    slug: &str,
) -> AppResult<CatalogEntry> {
    let svc = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Catalog entry not found".to_string()))?;

    let provider = if let Some(ref pid) = svc.provider_config_id {
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find_one(doc! { "_id": pid })
            .await?
    } else {
        None
    };

    let spr = db
        .collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
        .find_one(doc! { "service_id": &svc.id })
        .await?;

    let oauth_client_id = match provider.as_ref() {
        Some(provider) if provider.credential_mode != "user" => {
            decrypt_provider_client_id(provider, encryption_keys).await?
        }
        _ => None,
    };

    Ok(build_catalog_entry(
        svc,
        provider.as_ref(),
        spr.as_ref(),
        oauth_client_id,
    ))
}
