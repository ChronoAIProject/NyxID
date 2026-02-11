use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES,
};
use crate::models::provider_config::{ProviderConfig, COLLECTION_NAME};
use crate::models::service_provider_requirement::{
    ServiceProviderRequirement, COLLECTION_NAME as REQUIREMENTS,
};
use crate::models::user_provider_token::COLLECTION_NAME as USER_PROVIDER_TOKENS;

/// Seed default AI provider configurations at startup (idempotent).
///
/// Checks for each provider by slug; if it does not exist, inserts it.
/// The OpenAI Codex `client_id` is encrypted before storage.
pub async fn seed_default_providers(
    db: &mongodb::Database,
    encryption_key_hex: &str,
) -> AppResult<()> {
    let collection = db.collection::<ProviderConfig>(COLLECTION_NAME);
    let encryption_key = aes::parse_hex_key(encryption_key_hex)?;
    let now = Utc::now();

    let mut seeded_count: u32 = 0;

    // Helper: check if a provider with this slug already exists
    macro_rules! slug_exists {
        ($slug:expr) => {{
            collection
                .find_one(doc! { "slug": $slug })
                .await?
                .is_some()
        }};
    }

    // 1. OpenAI (API Key)
    if !slug_exists!("openai") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "openai".to_string(),
            name: "OpenAI".to_string(),
            description: Some(
                "OpenAI API access using API keys (pay-per-use billing)".to_string(),
            ),
            provider_type: "api_key".to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: Some(
                "Get your API key from https://platform.openai.com/api-keys".to_string(),
            ),
            api_key_url: Some("https://platform.openai.com/api-keys".to_string()),
            icon_url: None,
            documentation_url: Some("https://platform.openai.com/docs".to_string()),
            is_active: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "openai", "Seeded default provider: OpenAI");
        seeded_count += 1;
    }

    // Upgrade existing openai-codex providers with corrected device code URLs
    if let Some(existing) = collection
        .find_one(doc! { "slug": "openai-codex" })
        .await?
    {
        let needs_update = existing.device_code_url.as_deref()
            != Some("https://auth.openai.com/api/accounts/deviceauth/usercode")
            || existing.device_verification_url.is_none();
        if needs_update {
            collection
                .update_one(
                    doc! { "_id": &existing.id },
                    doc! { "$set": {
                        "device_code_url": "https://auth.openai.com/api/accounts/deviceauth/usercode",
                        "device_token_url": "https://auth.openai.com/api/accounts/deviceauth/token",
                        "device_verification_url": "https://auth.openai.com/codex/device",
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }},
                )
                .await?;
            tracing::info!(
                slug = "openai-codex",
                "Updated existing provider with corrected device code URLs"
            );
        }
    }

    // 2. OpenAI Codex (Device Code - ChatGPT subscription)
    if !slug_exists!("openai-codex") {
        let client_id_enc =
            aes::encrypt(b"app_EMoamEEZ73f0CkXaXp7hrann", &encryption_key)?;

        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "openai-codex".to_string(),
            name: "OpenAI Codex".to_string(),
            description: Some(
                "Connect your ChatGPT subscription (Plus/Pro/Team) for AI model access".to_string(),
            ),
            provider_type: "device_code".to_string(),
            authorization_url: Some(
                "https://auth.openai.com/oauth/authorize".to_string(),
            ),
            token_url: Some("https://auth.openai.com/oauth/token".to_string()),
            revocation_url: None,
            default_scopes: Some(vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ]),
            client_id_encrypted: Some(client_id_enc),
            client_secret_encrypted: None,
            supports_pkce: true,
            device_code_url: Some(
                "https://auth.openai.com/api/accounts/deviceauth/usercode".to_string(),
            ),
            device_token_url: Some(
                "https://auth.openai.com/api/accounts/deviceauth/token".to_string(),
            ),
            device_verification_url: Some(
                "https://auth.openai.com/codex/device".to_string(),
            ),
            hosted_callback_url: Some(
                "https://auth.openai.com/deviceauth/callback".to_string(),
            ),
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: Some(
                "https://developers.openai.com/codex/auth/".to_string(),
            ),
            is_active: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "openai-codex", "Seeded default provider: OpenAI Codex");
        seeded_count += 1;
    }

    // 3. Anthropic (API Key)
    if !slug_exists!("anthropic") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            description: Some("Anthropic Claude API access".to_string()),
            provider_type: "api_key".to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: Some(
                "Get your API key from https://console.anthropic.com/settings/keys".to_string(),
            ),
            api_key_url: Some(
                "https://console.anthropic.com/settings/keys".to_string(),
            ),
            icon_url: None,
            documentation_url: Some("https://docs.anthropic.com".to_string()),
            is_active: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "anthropic", "Seeded default provider: Anthropic");
        seeded_count += 1;
    }

    // 4. Google AI Studio (API Key)
    if !slug_exists!("google-ai") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "google-ai".to_string(),
            name: "Google AI Studio".to_string(),
            description: Some(
                "Google Gemini API access via AI Studio".to_string(),
            ),
            provider_type: "api_key".to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: Some(
                "Get your API key from https://aistudio.google.com/apikey".to_string(),
            ),
            api_key_url: Some("https://aistudio.google.com/apikey".to_string()),
            icon_url: None,
            documentation_url: Some("https://ai.google.dev/docs".to_string()),
            is_active: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "google-ai", "Seeded default provider: Google AI Studio");
        seeded_count += 1;
    }

    // 5. Mistral AI (API Key)
    if !slug_exists!("mistral") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "mistral".to_string(),
            name: "Mistral AI".to_string(),
            description: Some("Mistral AI API access".to_string()),
            provider_type: "api_key".to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: Some(
                "Get your API key from https://console.mistral.ai/api-keys".to_string(),
            ),
            api_key_url: Some("https://console.mistral.ai/api-keys".to_string()),
            icon_url: None,
            documentation_url: Some("https://docs.mistral.ai".to_string()),
            is_active: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "mistral", "Seeded default provider: Mistral AI");
        seeded_count += 1;
    }

    // 6. Cohere (API Key)
    if !slug_exists!("cohere") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "cohere".to_string(),
            name: "Cohere".to_string(),
            description: Some("Cohere API access".to_string()),
            provider_type: "api_key".to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: Some(
                "Get your API key from https://dashboard.cohere.com/api-keys".to_string(),
            ),
            api_key_url: Some(
                "https://dashboard.cohere.com/api-keys".to_string(),
            ),
            icon_url: None,
            documentation_url: Some("https://docs.cohere.com".to_string()),
            is_active: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "cohere", "Seeded default provider: Cohere");
        seeded_count += 1;
    }

    if seeded_count > 0 {
        tracing::info!(count = seeded_count, "Default provider seeding complete");
    }

    Ok(())
}

struct LlmServiceSeed {
    provider_slug: &'static str,
    service_slug: &'static str,
    service_name: &'static str,
    base_url: &'static str,
    injection_method: &'static str,
    injection_key: &'static str,
}

const LLM_SERVICE_SEEDS: &[LlmServiceSeed] = &[
    LlmServiceSeed {
        provider_slug: "openai",
        service_slug: "llm-openai",
        service_name: "OpenAI API",
        base_url: "https://api.openai.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    LlmServiceSeed {
        provider_slug: "openai-codex",
        service_slug: "llm-openai-codex",
        service_name: "OpenAI Codex API",
        base_url: "https://api.openai.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    LlmServiceSeed {
        provider_slug: "anthropic",
        service_slug: "llm-anthropic",
        service_name: "Anthropic API",
        base_url: "https://api.anthropic.com/v1",
        injection_method: "header",
        injection_key: "x-api-key",
    },
    LlmServiceSeed {
        provider_slug: "google-ai",
        service_slug: "llm-google-ai",
        service_name: "Google AI API",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        injection_method: "query",
        injection_key: "key",
    },
    LlmServiceSeed {
        provider_slug: "mistral",
        service_slug: "llm-mistral",
        service_name: "Mistral AI API",
        base_url: "https://api.mistral.ai/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    LlmServiceSeed {
        provider_slug: "cohere",
        service_slug: "llm-cohere",
        service_name: "Cohere API",
        base_url: "https://api.cohere.com/v2",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
];

/// Seed downstream services for each LLM provider (idempotent).
///
/// Creates a `DownstreamService` and a `ServiceProviderRequirement` for each
/// seeded provider that does not yet have a corresponding downstream service.
pub async fn seed_default_llm_services(
    db: &mongodb::Database,
    encryption_key_hex: &str,
) -> AppResult<()> {
    let encryption_key = aes::parse_hex_key(encryption_key_hex)?;
    let provider_col = db.collection::<ProviderConfig>(COLLECTION_NAME);
    let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    let req_col = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);
    let now = Utc::now();
    let mut seeded_count: u32 = 0;

    for seed in LLM_SERVICE_SEEDS {
        // Find the provider by slug
        let provider = match provider_col
            .find_one(doc! { "slug": seed.provider_slug })
            .await?
        {
            Some(p) => p,
            None => continue, // Provider not seeded yet, skip
        };

        // Check if a downstream service already exists for this provider
        let existing = service_col
            .find_one(doc! { "provider_config_id": &provider.id })
            .await?;

        if existing.is_some() {
            continue; // Already seeded
        }

        // Create an empty encrypted credential (field is required)
        let empty_credential = aes::encrypt(b"", &encryption_key)?;

        let service_id = Uuid::new_v4().to_string();

        let service = DownstreamService {
            id: service_id.clone(),
            name: seed.service_name.to_string(),
            slug: seed.service_slug.to_string(),
            description: Some(format!("{} proxied via NyxID LLM gateway", seed.service_name)),
            base_url: seed.base_url.to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential_encrypted: empty_credential,
            auth_type: None,
            api_spec_url: None,
            oauth_client_id: None,
            service_category: "internal".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: "system".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            provider_config_id: Some(provider.id.clone()),
            created_at: now,
            updated_at: now,
        };

        service_col.insert_one(&service).await?;

        // Create the ServiceProviderRequirement linking this service to its provider
        let requirement = ServiceProviderRequirement {
            id: Uuid::new_v4().to_string(),
            service_id: service_id.clone(),
            provider_config_id: provider.id.clone(),
            required: true,
            scopes: None,
            injection_method: seed.injection_method.to_string(),
            injection_key: Some(seed.injection_key.to_string()),
            created_at: now,
            updated_at: now,
        };

        req_col.insert_one(&requirement).await?;

        tracing::info!(
            slug = seed.service_slug,
            provider = seed.provider_slug,
            "Seeded LLM downstream service"
        );
        seeded_count += 1;
    }

    if seeded_count > 0 {
        tracing::info!(count = seeded_count, "LLM downstream service seeding complete");
    }

    Ok(())
}

/// Input for OAuth2 provider configuration fields.
pub struct OAuthProviderInput {
    pub authorization_url: String,
    pub token_url: String,
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub client_id: String,
    pub client_secret: String,
    pub supports_pkce: bool,
}

/// Input for device_code provider configuration fields (RFC 8628 Device Authorization Grant).
pub struct DeviceCodeProviderInput {
    pub authorization_url: String,
    pub token_url: String,
    pub device_code_url: String,
    pub device_token_url: String,
    pub device_verification_url: Option<String>,
    pub hosted_callback_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub supports_pkce: bool,
}

/// Input for API key provider configuration fields.
pub struct ApiKeyProviderInput {
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
}

/// Fields that can be updated on a provider config.
pub struct ProviderUpdateInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub is_active: Option<bool>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub supports_pkce: Option<bool>,
    pub device_code_url: Option<String>,
    pub device_token_url: Option<String>,
    pub device_verification_url: Option<String>,
    pub hosted_callback_url: Option<String>,
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
}

/// Create a new provider configuration. Admin only.
#[allow(clippy::too_many_arguments)]
pub async fn create_provider(
    db: &mongodb::Database,
    encryption_key: &[u8],
    name: &str,
    slug: &str,
    provider_type: &str,
    oauth_config: Option<OAuthProviderInput>,
    api_key_config: Option<ApiKeyProviderInput>,
    device_code_config: Option<DeviceCodeProviderInput>,
    description: Option<&str>,
    icon_url: Option<&str>,
    documentation_url: Option<&str>,
    created_by: &str,
) -> AppResult<ProviderConfig> {
    let valid_types = ["oauth2", "api_key", "device_code"];
    if !valid_types.contains(&provider_type) {
        return Err(AppError::ValidationError(format!(
            "provider_type must be one of: {}",
            valid_types.join(", ")
        )));
    }

    // Check slug uniqueness
    let existing = db
        .collection::<ProviderConfig>(COLLECTION_NAME)
        .find_one(doc! { "slug": slug })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "A provider with this slug already exists".to_string(),
        ));
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    // Encrypt OAuth credentials if provided
    let (client_id_enc, client_secret_enc) = if let Some(ref oauth) = oauth_config {
        let cid = aes::encrypt(oauth.client_id.as_bytes(), encryption_key)?;
        let csec = aes::encrypt(oauth.client_secret.as_bytes(), encryption_key)?;
        (Some(cid), Some(csec))
    } else if let Some(ref dc) = device_code_config {
        let cid = aes::encrypt(dc.client_id.as_bytes(), encryption_key)?;
        let csec = match dc.client_secret.as_ref() {
            Some(s) => Some(aes::encrypt(s.as_bytes(), encryption_key)?),
            None => None,
        };
        (Some(cid), csec)
    } else {
        (None, None)
    };

    let provider = ProviderConfig {
        id: id.clone(),
        slug: slug.to_string(),
        name: name.to_string(),
        description: description.map(String::from),
        provider_type: provider_type.to_string(),
        authorization_url: oauth_config
            .as_ref()
            .map(|o| o.authorization_url.clone())
            .or_else(|| device_code_config.as_ref().map(|d| d.authorization_url.clone())),
        token_url: oauth_config
            .as_ref()
            .map(|o| o.token_url.clone())
            .or_else(|| device_code_config.as_ref().map(|d| d.token_url.clone())),
        revocation_url: oauth_config.as_ref().and_then(|o| o.revocation_url.clone()),
        default_scopes: oauth_config
            .as_ref()
            .and_then(|o| o.default_scopes.clone())
            .or_else(|| device_code_config.as_ref().and_then(|d| d.default_scopes.clone())),
        client_id_encrypted: client_id_enc,
        client_secret_encrypted: client_secret_enc,
        supports_pkce: oauth_config
            .as_ref()
            .is_some_and(|o| o.supports_pkce)
            || device_code_config.as_ref().is_some_and(|d| d.supports_pkce),
        device_code_url: device_code_config.as_ref().map(|d| d.device_code_url.clone()),
        device_token_url: device_code_config.as_ref().map(|d| d.device_token_url.clone()),
        device_verification_url: device_code_config
            .as_ref()
            .and_then(|d| d.device_verification_url.clone()),
        hosted_callback_url: device_code_config.as_ref().and_then(|d| d.hosted_callback_url.clone()),
        api_key_instructions: api_key_config.as_ref().and_then(|a| a.api_key_instructions.clone()),
        api_key_url: api_key_config.as_ref().and_then(|a| a.api_key_url.clone()),
        icon_url: icon_url.map(String::from),
        documentation_url: documentation_url.map(String::from),
        is_active: true,
        created_by: created_by.to_string(),
        created_at: now,
        updated_at: now,
    };

    db.collection::<ProviderConfig>(COLLECTION_NAME)
        .insert_one(&provider)
        .await?;

    tracing::info!(provider_id = %id, slug = %slug, "Provider config created");

    Ok(provider)
}

/// List all active providers (visible to all authenticated users).
pub async fn list_providers(db: &mongodb::Database) -> AppResult<Vec<ProviderConfig>> {
    let providers: Vec<ProviderConfig> = db
        .collection::<ProviderConfig>(COLLECTION_NAME)
        .find(doc! { "is_active": true })
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    Ok(providers)
}

/// Get a single provider by ID.
pub async fn get_provider(
    db: &mongodb::Database,
    provider_id: &str,
) -> AppResult<ProviderConfig> {
    db.collection::<ProviderConfig>(COLLECTION_NAME)
        .find_one(doc! { "_id": provider_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))
}

/// Get a single provider by slug.
#[allow(dead_code)]
pub async fn get_provider_by_slug(
    db: &mongodb::Database,
    slug: &str,
) -> AppResult<ProviderConfig> {
    db.collection::<ProviderConfig>(COLLECTION_NAME)
        .find_one(doc! { "slug": slug })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))
}

/// Update provider configuration. Admin only.
///
/// Uses `find_one_and_update` with `ReturnDocument::After` to avoid an
/// extra read query (CR-14).
pub async fn update_provider(
    db: &mongodb::Database,
    encryption_key: &[u8],
    provider_id: &str,
    updates: ProviderUpdateInput,
) -> AppResult<ProviderConfig> {
    // Verify exists
    let _existing = get_provider(db, provider_id).await?;

    let now = Utc::now();
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(now),
    };

    if let Some(ref name) = updates.name {
        set_doc.insert("name", name.as_str());
    }
    if let Some(ref desc) = updates.description {
        set_doc.insert("description", desc.as_str());
    }
    if let Some(active) = updates.is_active {
        set_doc.insert("is_active", active);
    }
    if let Some(ref url) = updates.authorization_url {
        set_doc.insert("authorization_url", url.as_str());
    }
    if let Some(ref url) = updates.token_url {
        set_doc.insert("token_url", url.as_str());
    }
    if let Some(ref url) = updates.revocation_url {
        set_doc.insert("revocation_url", url.as_str());
    }
    if let Some(ref scopes) = updates.default_scopes {
        set_doc.insert("default_scopes", scopes);
    }
    if let Some(ref cid) = updates.client_id {
        let enc = aes::encrypt(cid.as_bytes(), encryption_key)?;
        set_doc.insert(
            "client_id_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: enc,
            },
        );
    }
    if let Some(ref csec) = updates.client_secret {
        let enc = aes::encrypt(csec.as_bytes(), encryption_key)?;
        set_doc.insert(
            "client_secret_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: enc,
            },
        );
    }
    if let Some(pkce) = updates.supports_pkce {
        set_doc.insert("supports_pkce", pkce);
    }
    if let Some(ref url) = updates.device_code_url {
        set_doc.insert("device_code_url", url.as_str());
    }
    if let Some(ref url) = updates.device_token_url {
        set_doc.insert("device_token_url", url.as_str());
    }
    if let Some(ref url) = updates.device_verification_url {
        set_doc.insert("device_verification_url", url.as_str());
    }
    if let Some(ref url) = updates.hosted_callback_url {
        set_doc.insert("hosted_callback_url", url.as_str());
    }
    if let Some(ref instr) = updates.api_key_instructions {
        set_doc.insert("api_key_instructions", instr.as_str());
    }
    if let Some(ref url) = updates.api_key_url {
        set_doc.insert("api_key_url", url.as_str());
    }
    if let Some(ref url) = updates.icon_url {
        set_doc.insert("icon_url", url.as_str());
    }
    if let Some(ref url) = updates.documentation_url {
        set_doc.insert("documentation_url", url.as_str());
    }

    use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

    let updated = db
        .collection::<ProviderConfig>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": provider_id },
            doc! { "$set": set_doc },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))?;

    tracing::info!(provider_id = %provider_id, "Provider config updated");

    Ok(updated)
}

/// Soft-delete a provider. Also revokes all user tokens for this provider.
pub async fn delete_provider(
    db: &mongodb::Database,
    provider_id: &str,
) -> AppResult<()> {
    let _existing = get_provider(db, provider_id).await?;

    let now = Utc::now();

    // Deactivate the provider
    db.collection::<ProviderConfig>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": provider_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // Revoke all user tokens for this provider
    db.collection::<mongodb::bson::Document>(USER_PROVIDER_TOKENS)
        .update_many(
            doc! { "provider_config_id": provider_id, "status": "active" },
            doc! { "$set": {
                "status": "revoked",
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(provider_id = %provider_id, "Provider deactivated and user tokens revoked");

    Ok(())
}
