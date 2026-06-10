use std::collections::HashMap;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::COLLECTION_NAME as AGENT_SERVICE_BINDINGS;
use crate::models::default_request_header::DefaultRequestHeader;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, ServiceCapabilities,
};
use crate::models::provider_config::{COLLECTION_NAME, ProviderConfig};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
};
use crate::models::user_api_key::COLLECTION_NAME as USER_API_KEYS;
use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
use crate::models::user_provider_credentials::COLLECTION_NAME as USER_PROVIDER_CREDENTIALS;
use crate::models::user_provider_token::COLLECTION_NAME as USER_PROVIDER_TOKENS;
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

const SEEDED_USER_CREDENTIAL_OAUTH_PROVIDER_SLUGS: &[&str] = &[
    "google",
    "github",
    "facebook",
    "discord",
    "spotify",
    "linkedin",
    "slack",
    "microsoft",
    "tiktok",
    "twitch",
    "reddit",
    "lark",
];

/// Seed default AI provider configurations at startup (idempotent).
///
/// Checks for each provider by slug; if it does not exist, inserts it.
/// The OpenAI Codex `client_id` is encrypted before storage.
pub async fn seed_default_providers(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
) -> AppResult<()> {
    let collection = db.collection::<ProviderConfig>(COLLECTION_NAME);
    let now = Utc::now();

    let mut seeded_count: u32 = 0;

    // Helper: check if a provider with this slug already exists
    macro_rules! slug_exists {
        ($slug:expr) => {{ collection.find_one(doc! { "slug": $slug }).await?.is_some() }};
    }

    // 1. OpenAI (API Key)
    if !slug_exists!("openai") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "openai".to_string(),
            name: "OpenAI".to_string(),
            description: Some("OpenAI API access using API keys (pay-per-use billing)".to_string()),
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
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "openai", "Seeded default provider: OpenAI");
        seeded_count += 1;
    }

    // Upgrade existing openai-codex providers with corrected device code URLs
    if let Some(existing) = collection.find_one(doc! { "slug": "openai-codex" }).await? {
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

    // Migration: set device_code_format to "openai" on existing openai-codex providers
    if let Some(existing_codex) = collection
        .find_one(doc! { "slug": "openai-codex", "device_code_format": { "$ne": "openai" } })
        .await?
    {
        collection
            .update_one(
                doc! { "_id": &existing_codex.id },
                doc! { "$set": {
                    "device_code_format": "openai",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .await?;
        tracing::info!(
            slug = "openai-codex",
            "Migrated existing provider to device_code_format=openai"
        );
    }

    // 2. OpenAI Codex (Device Code - ChatGPT subscription)
    if !slug_exists!("openai-codex") {
        let client_id_enc = encryption_keys
            .encrypt(b"app_EMoamEEZ73f0CkXaXp7hrann")
            .await?;

        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "openai-codex".to_string(),
            name: "OpenAI Codex".to_string(),
            description: Some(
                "Connect your ChatGPT subscription (Plus/Pro/Team) for AI model access".to_string(),
            ),
            provider_type: "device_code".to_string(),
            authorization_url: Some("https://auth.openai.com/oauth/authorize".to_string()),
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
            device_verification_url: Some("https://auth.openai.com/codex/device".to_string()),
            hosted_callback_url: Some("https://auth.openai.com/deviceauth/callback".to_string()),
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: Some("https://developers.openai.com/codex/auth/".to_string()),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "openai".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(
            slug = "openai-codex",
            "Seeded default provider: OpenAI Codex"
        );
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
            api_key_url: Some("https://console.anthropic.com/settings/keys".to_string()),
            icon_url: None,
            documentation_url: Some("https://docs.anthropic.com".to_string()),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
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
            description: Some("Google Gemini API access via AI Studio".to_string()),
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
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(
            slug = "google-ai",
            "Seeded default provider: Google AI Studio"
        );
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
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
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
            api_key_url: Some("https://dashboard.cohere.com/api-keys".to_string()),
            icon_url: None,
            documentation_url: Some("https://docs.cohere.com".to_string()),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "cohere", "Seeded default provider: Cohere");
        seeded_count += 1;
    }

    // 7. DeepSeek (API Key)
    if !slug_exists!("deepseek") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "deepseek".to_string(),
            name: "DeepSeek".to_string(),
            description: Some("DeepSeek AI API access".to_string()),
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
                "Get your API key from https://platform.deepseek.com/api_keys".to_string(),
            ),
            api_key_url: Some("https://platform.deepseek.com/api_keys".to_string()),
            icon_url: None,
            documentation_url: Some("https://api-docs.deepseek.com".to_string()),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "deepseek", "Seeded default provider: DeepSeek");
        seeded_count += 1;
    }

    // 8. Twitter / X (OAuth 2.0 with PKCE)
    if !slug_exists!("twitter") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "twitter".to_string(),
            name: "Twitter / X".to_string(),
            description: Some("Twitter/X API access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://x.com/i/oauth2/authorize".to_string()),
            token_url: Some("https://api.x.com/2/oauth2/token".to_string()),
            revocation_url: Some("https://api.x.com/2/oauth2/revoke".to_string()),
            // Write access is intentional: NyxID is a credential broker, so delegated
            // clients commonly need to post on behalf of users. Admins can customise
            // scopes per deployment.
            default_scopes: Some(vec![
                "tweet.read".to_string(),
                "tweet.write".to_string(),
                "users.read".to_string(),
                "offline.access".to_string(),
            ]),
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
            documentation_url: Some("https://developer.x.com/en/docs/x-api".to_string()),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_basic".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "twitter", "Seeded default provider: Twitter / X");
        seeded_count += 1;
    }

    // Migration: set credential_mode and token_endpoint_auth_method on existing Twitter providers.
    // The $ne filter means this is a no-op after migration completes.
    if let Some(existing_twitter) = collection
        .find_one(doc! { "slug": "twitter", "$or": [
            { "credential_mode": { "$ne": "user" } },
            { "token_endpoint_auth_method": { "$ne": "client_secret_basic" } },
        ]})
        .await?
    {
        collection
            .update_one(
                doc! { "_id": &existing_twitter.id },
                doc! { "$set": {
                    "credential_mode": "user",
                    "token_endpoint_auth_method": "client_secret_basic",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .await?;
        tracing::info!(
            slug = "twitter",
            "Migrated existing Twitter provider to credential_mode=user, token_endpoint_auth_method=client_secret_basic"
        );
    }

    let social_user_mode_migration = collection
        .update_many(
            doc! {
                "slug": { "$in": SEEDED_USER_CREDENTIAL_OAUTH_PROVIDER_SLUGS },
                "credential_mode": { "$ne": "user" }
            },
            doc! { "$set": {
                "credential_mode": "user",
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .await?;
    if social_user_mode_migration.modified_count > 0 {
        tracing::info!(
            count = social_user_mode_migration.modified_count,
            "Migrated existing seeded social providers to credential_mode=user"
        );
    }

    // 9. Google (OAuth2)
    if !slug_exists!("google") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "google".to_string(),
            name: "Google".to_string(),
            description: Some("Google account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://accounts.google.com/o/oauth2/v2/auth".to_string()),
            token_url: Some("https://oauth2.googleapis.com/token".to_string()),
            revocation_url: Some("https://oauth2.googleapis.com/revoke".to_string()),
            default_scopes: Some(vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ]),
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
            documentation_url: Some(
                "https://developers.google.com/identity/protocols/oauth2".to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: Some(HashMap::from([
                ("access_type".to_string(), "offline".to_string()),
                ("prompt".to_string(), "consent".to_string()),
            ])),
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "google", "Seeded default provider: Google");
        seeded_count += 1;
    }

    // 10. GitHub (OAuth2)
    if !slug_exists!("github") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "github".to_string(),
            name: "GitHub".to_string(),
            description: Some("GitHub account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://github.com/login/oauth/authorize".to_string()),
            token_url: Some("https://github.com/login/oauth/access_token".to_string()),
            revocation_url: None,
            default_scopes: Some(vec!["read:user".to_string(), "user:email".to_string()]),
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: Some("https://docs.github.com/en/apps/oauth-apps".to_string()),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "github", "Seeded default provider: GitHub");
        seeded_count += 1;
    }

    // 10b. GitHub (Personal Access Token)
    if !slug_exists!("github-pat") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "github-pat".to_string(),
            name: "GitHub (API Key)".to_string(),
            description: Some(
                "GitHub REST API access using a Personal Access Token \
                 (classic or fine-grained) instead of OAuth."
                    .to_string(),
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
                "Create a classic token at https://github.com/settings/tokens \
                 or a fine-grained token at \
                 https://github.com/settings/personal-access-tokens. \
                 Grant only the scopes you need."
                    .to_string(),
            ),
            api_key_url: Some("https://github.com/settings/tokens".to_string()),
            icon_url: None,
            documentation_url: Some(
                "https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens"
                    .to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "github-pat", "Seeded default provider: GitHub PAT");
        seeded_count += 1;
    }

    // 11. Facebook (OAuth2)
    if !slug_exists!("facebook") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "facebook".to_string(),
            name: "Facebook".to_string(),
            description: Some("Facebook account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://www.facebook.com/v21.0/dialog/oauth".to_string()),
            token_url: Some("https://graph.facebook.com/v21.0/oauth/access_token".to_string()),
            revocation_url: None,
            default_scopes: Some(vec!["email".to_string(), "public_profile".to_string()]),
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
            documentation_url: Some(
                "https://developers.facebook.com/docs/facebook-login/".to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "facebook", "Seeded default provider: Facebook");
        seeded_count += 1;
    }

    // 12. Discord (OAuth2)
    if !slug_exists!("discord") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "discord".to_string(),
            name: "Discord".to_string(),
            description: Some("Discord account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://discord.com/oauth2/authorize".to_string()),
            token_url: Some("https://discord.com/api/oauth2/token".to_string()),
            revocation_url: Some("https://discord.com/api/oauth2/token/revoke".to_string()),
            default_scopes: Some(vec!["identify".to_string(), "email".to_string()]),
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
            documentation_url: Some(
                "https://discord.com/developers/docs/topics/oauth2".to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "discord", "Seeded default provider: Discord");
        seeded_count += 1;
    }

    // 13. Spotify (OAuth2)
    if !slug_exists!("spotify") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "spotify".to_string(),
            name: "Spotify".to_string(),
            description: Some("Spotify account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://accounts.spotify.com/authorize".to_string()),
            token_url: Some("https://accounts.spotify.com/api/token".to_string()),
            revocation_url: None,
            default_scopes: Some(vec![
                "user-read-email".to_string(),
                "user-read-private".to_string(),
            ]),
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
            documentation_url: Some(
                "https://developer.spotify.com/documentation/web-api".to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "spotify", "Seeded default provider: Spotify");
        seeded_count += 1;
    }

    // 14. LinkedIn (OAuth2)
    if !slug_exists!("linkedin") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "linkedin".to_string(),
            name: "LinkedIn".to_string(),
            description: Some("LinkedIn account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some(
                "https://www.linkedin.com/oauth/v2/authorization".to_string(),
            ),
            token_url: Some("https://www.linkedin.com/oauth/v2/accessToken".to_string()),
            revocation_url: None,
            default_scopes: Some(vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ]),
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
            documentation_url: Some(
                "https://learn.microsoft.com/en-us/linkedin/shared/authentication/authorization-code-flow".to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "linkedin", "Seeded default provider: LinkedIn");
        seeded_count += 1;
    }

    // 15. Slack (OAuth2)
    if !slug_exists!("slack") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "slack".to_string(),
            name: "Slack".to_string(),
            description: Some("Slack workspace access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://slack.com/oauth/v2/authorize".to_string()),
            token_url: Some("https://slack.com/api/oauth.v2.access".to_string()),
            revocation_url: Some("https://slack.com/api/auth.revoke".to_string()),
            default_scopes: Some(vec![
                "users:read".to_string(),
                "users:read.email".to_string(),
            ]),
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: Some("https://api.slack.com/authentication/oauth-v2".to_string()),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "slack", "Seeded default provider: Slack");
        seeded_count += 1;
    }

    // 16. Microsoft (OAuth2)
    if !slug_exists!("microsoft") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "microsoft".to_string(),
            name: "Microsoft".to_string(),
            description: Some("Microsoft account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some(
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".to_string(),
            ),
            token_url: Some(
                "https://login.microsoftonline.com/common/oauth2/v2.0/token".to_string(),
            ),
            revocation_url: None,
            default_scopes: Some(vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
                "offline_access".to_string(),
            ]),
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
            documentation_url: Some(
                "https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow".to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "microsoft", "Seeded default provider: Microsoft");
        seeded_count += 1;
    }

    // 17. TikTok (OAuth2)
    if !slug_exists!("tiktok") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "tiktok".to_string(),
            name: "TikTok".to_string(),
            description: Some("TikTok account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://www.tiktok.com/v2/auth/authorize/".to_string()),
            token_url: Some("https://open.tiktokapis.com/v2/oauth/token/".to_string()),
            revocation_url: Some("https://open.tiktokapis.com/v2/oauth/revoke/".to_string()),
            default_scopes: Some(vec!["user.info.basic".to_string()]),
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
            documentation_url: Some(
                "https://developers.tiktok.com/doc/oauth-user-access-token-management/".to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: Some("client_key".to_string()),
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "tiktok", "Seeded default provider: TikTok");
        seeded_count += 1;
    }

    // 18. Twitch (OAuth2)
    if !slug_exists!("twitch") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "twitch".to_string(),
            name: "Twitch".to_string(),
            description: Some("Twitch account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://id.twitch.tv/oauth2/authorize".to_string()),
            token_url: Some("https://id.twitch.tv/oauth2/token".to_string()),
            revocation_url: Some("https://id.twitch.tv/oauth2/revoke".to_string()),
            default_scopes: Some(vec!["user:read:email".to_string()]),
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
            documentation_url: Some("https://dev.twitch.tv/docs/authentication/".to_string()),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "twitch", "Seeded default provider: Twitch");
        seeded_count += 1;
    }

    // 19. Reddit (OAuth2)
    if !slug_exists!("reddit") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "reddit".to_string(),
            name: "Reddit".to_string(),
            description: Some("Reddit account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://www.reddit.com/api/v1/authorize".to_string()),
            token_url: Some("https://www.reddit.com/api/v1/access_token".to_string()),
            revocation_url: Some("https://www.reddit.com/api/v1/revoke_token".to_string()),
            default_scopes: Some(vec!["identity".to_string()]),
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: Some("https://www.reddit.com/dev/api/oauth".to_string()),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_basic".to_string(),
            extra_auth_params: Some(HashMap::from([(
                "duration".to_string(),
                "permanent".to_string(),
            )])),
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "reddit", "Seeded default provider: Reddit");
        seeded_count += 1;
    }

    // Upgrade existing lark/feishu providers with corrected documentation URLs
    let old_lark_doc_url = "https://open.larksuite.com/document/server-docs/authentication-management/access-token/authorize-user-access-token";
    if let Some(existing) = collection
        .find_one(doc! { "slug": "lark", "documentation_url": old_lark_doc_url })
        .await?
    {
        collection
            .update_one(
                doc! { "_id": &existing.id },
                doc! { "$set": {
                    "documentation_url": "https://open.larksuite.com/document/common-capabilities/sso/api/obtain-oauth-code",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .await?;
        tracing::info!(
            slug = "lark",
            "Updated existing provider with corrected documentation URL"
        );
    }
    let old_feishu_doc_url = "https://open.feishu.cn/document/server-docs/authentication-management/access-token/authorize-user-access-token";
    if let Some(existing) = collection
        .find_one(doc! { "slug": "feishu", "documentation_url": old_feishu_doc_url })
        .await?
    {
        collection
            .update_one(
                doc! { "_id": &existing.id },
                doc! { "$set": {
                    "documentation_url": "https://open.feishu.cn/document/common-capabilities/sso/api/obtain-oauth-code",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }},
            )
            .await?;
        tracing::info!(
            slug = "feishu",
            "Updated existing provider with corrected documentation URL"
        );
    }

    // 20. Lark / Larksuite (OAuth2)
    if !slug_exists!("lark") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "lark".to_string(),
            name: "Lark".to_string(),
            description: Some("Lark (Larksuite) account access via OAuth 2.0".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some(
                "https://open.larksuite.com/open-apis/authen/v1/index".to_string(),
            ),
            token_url: Some(
                "https://open.larksuite.com/open-apis/authen/v2/oauth/token".to_string(),
            ),
            revocation_url: None,
            default_scopes: Some(vec![
                "contact:user.base:readonly".to_string(),
                "offline_access".to_string(),
            ]),
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
            documentation_url: Some(
                "https://open.larksuite.com/document/common-capabilities/sso/api/obtain-oauth-code"
                    .to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "lark", "Seeded default provider: Lark");
        seeded_count += 1;
    }

    // 20b. Feishu (China variant of Lark, OAuth2)
    if !slug_exists!("feishu") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "feishu".to_string(),
            name: "Feishu".to_string(),
            description: Some(
                "Feishu (飞书) account access via OAuth 2.0 (China region)".to_string(),
            ),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://open.feishu.cn/open-apis/authen/v1/index".to_string()),
            token_url: Some("https://open.feishu.cn/open-apis/authen/v2/oauth/token".to_string()),
            revocation_url: None,
            default_scopes: Some(vec![
                "contact:user.base:readonly".to_string(),
                "offline_access".to_string(),
            ]),
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
            documentation_url: Some(
                "https://open.feishu.cn/document/common-capabilities/sso/api/obtain-oauth-code"
                    .to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "feishu", "Seeded default provider: Feishu");
        seeded_count += 1;
    }

    // 21. Telegram Login Widget (telegram_widget)
    if !slug_exists!("telegram") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "telegram".to_string(),
            name: "Telegram".to_string(),
            description: Some(
                "Telegram identity verification via Login Widget (HMAC-SHA256)".to_string(),
            ),
            provider_type: "telegram_widget".to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            // client_secret_encrypted is reused to store the encrypted bot token
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: Some("https://core.telegram.org/widgets/login".to_string()),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "telegram", "Seeded default provider: Telegram Login");
        seeded_count += 1;
    }

    // 22. Telegram Bot API (API Key)
    if !slug_exists!("telegram-bot") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "telegram-bot".to_string(),
            name: "Telegram Bot API".to_string(),
            description: Some(
                "Access the Telegram Bot API to send messages and manage bots".to_string(),
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
                "Create a bot via @BotFather on Telegram, then copy the bot token (e.g. 123456789:ABCdefGHI_jklMNOpqrSTUvwx)."
                    .to_string(),
            ),
            api_key_url: Some("https://t.me/BotFather".to_string()),
            icon_url: None,
            documentation_url: Some("https://core.telegram.org/bots/api".to_string()),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(
            slug = "telegram-bot",
            "Seeded default provider: Telegram Bot API"
        );
        seeded_count += 1;
    }

    // 22b. Lark Bot API (API Key — app credentials for tenant token exchange)
    if !slug_exists!("lark-bot") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "lark-bot".to_string(),
            name: "Lark Bot API".to_string(),
            description: Some(
                "Lark bot tenant credentials. NyxID stores your app_secret and injects \
                 it into tenant_access_token exchange requests so the secret never leaves \
                 the server. Used to authenticate as a Lark bot rather than as a user."
                    .to_string(),
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
                "Create a custom app at https://open.larksuite.com/app, then copy the \
                 App Secret. The App ID is sent in your request body when calling \
                 /auth/v3/tenant_access_token/internal."
                    .to_string(),
            ),
            api_key_url: Some("https://open.larksuite.com/app".to_string()),
            icon_url: None,
            documentation_url: Some(
                "https://open.larksuite.com/document/server-docs/getting-started/api-access-token/auth-v3/tenant_access_token_internal"
                    .to_string(),
            ),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "lark-bot", "Seeded default provider: Lark Bot API");
        seeded_count += 1;
    }

    // 22c. Feishu Bot API (China region — same as Lark Bot)
    if !slug_exists!("feishu-bot") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "feishu-bot".to_string(),
            name: "Feishu Bot API".to_string(),
            description: Some(
                "Feishu bot tenant credentials (China region). NyxID stores your app_secret \
                 and injects it into tenant_access_token exchange requests so the secret \
                 never leaves the server. Same as Lark Bot but for the China-region domain."
                    .to_string(),
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
                "Create a custom app at https://open.feishu.cn/app, then copy the \
                 App Secret. The App ID is sent in your request body when calling \
                 /auth/v3/tenant_access_token/internal."
                    .to_string(),
            ),
            api_key_url: Some("https://open.feishu.cn/app".to_string()),
            icon_url: None,
            documentation_url: Some(
                "https://open.feishu.cn/document/server-docs/authentication-management/access-token/tenant_access_token_internal"
                    .to_string(),
            ),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(
            slug = "feishu-bot",
            "Seeded default provider: Feishu Bot API"
        );
        seeded_count += 1;
    }

    // 22d. Discord Bot API (API Key — persistent bot token)
    if !slug_exists!("discord-bot") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "discord-bot".to_string(),
            name: "Discord Bot API".to_string(),
            description: Some(
                "Discord bot token credentials. NyxID stores your bot token and injects \
                 it as `Authorization: Bot <token>` on outbound calls. Tokens are \
                 persistent and never expire."
                    .to_string(),
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
                "Create an application at https://discord.com/developers/applications, \
                 add a Bot, then copy the Bot Token. Do not include the 'Bot ' prefix -- \
                 NyxID adds it automatically."
                    .to_string(),
            ),
            api_key_url: Some("https://discord.com/developers/applications".to_string()),
            icon_url: None,
            documentation_url: Some(
                "https://docs.discord.com/developers/reference#authentication".to_string(),
            ),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(
            slug = "discord-bot",
            "Seeded default provider: Discord Bot API"
        );
        seeded_count += 1;
    }

    // 22e. Slack Bot API (API Key — bot user OAuth token `xoxb-`)
    if !slug_exists!("slack-bot") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "slack-bot".to_string(),
            name: "Slack Bot API".to_string(),
            description: Some(
                "Slack bot user (`xoxb-`) token credentials. NyxID stores your bot \
                 token and injects it as `Authorization: Bearer <token>` on outbound \
                 calls. Tokens are long-lived; rotate them in the Slack app UI."
                    .to_string(),
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
                "Create a Slack app at https://api.slack.com/apps, add the bot scopes \
                 you need (e.g. `chat:write`, `channels:read`, `users:read`), install \
                 the app to your workspace, then copy the **Bot User OAuth Token** \
                 (starts with `xoxb-`) from OAuth & Permissions."
                    .to_string(),
            ),
            api_key_url: Some("https://api.slack.com/apps".to_string()),
            icon_url: None,
            documentation_url: Some(
                "https://api.slack.com/authentication/token-types#bot".to_string(),
            ),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "slack-bot", "Seeded default provider: Slack Bot API");
        seeded_count += 1;
    }

    // 23. OpenClaw (API Key + self-hosted gateway URL)
    if !slug_exists!("openclaw") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "openclaw".to_string(),
            name: "OpenClaw".to_string(),
            description: Some(
                "Connect your self-hosted OpenClaw AI gateway for multi-channel agent access"
                    .to_string(),
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
                "Enter your OpenClaw gateway bearer token (OPENCLAW_GATEWAY_TOKEN). \
                 You also need to provide your gateway URL (e.g., http://localhost:18789)."
                    .to_string(),
            ),
            api_key_url: None,
            icon_url: None,
            documentation_url: Some("https://docs.openclaw.ai/gateway/authentication".to_string()),
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "openclaw", "Seeded default provider: OpenClaw");
        seeded_count += 1;
    }

    if !slug_exists!("aevatar") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "aevatar".to_string(),
            name: "aevatar".to_string(),
            description: Some(
                "Connect your self-hosted aevatar runtime for Codex, Cursor, and channel inbound routing"
                    .to_string(),
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
                "Enter the bearer token accepted by your aevatar gateway. You must also provide your aevatar gateway URL."
                    .to_string(),
            ),
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: true,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "aevatar", "Seeded default provider: aevatar");
        seeded_count += 1;
    }

    // Cloud billing providers (NyxID#716, #778). AWS uses direct sigv4
    // injection (non-delegated). Google Cloud uses the standard OAuth2
    // delegated flow via the new `google-cloud` provider + `api-google-cloud`
    // service (user supplies --endpoint-url per add).
    if !slug_exists!("aws-billing") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "aws-billing".to_string(),
            name: "AWS Billing".to_string(),
            description: Some(
                "AWS Cost Explorer and related billing APIs from the management \
                 (payer) account in an AWS Organization. Credentials must be from \
                 the management account — linked-account keys cannot read \
                 consolidated billing."
                    .to_string(),
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
                "Create an IAM user in the AWS Organization management account with \
                 a policy granting `ce:GetCostAndUsage` and \
                 `ce:GetCostAndUsageWithResources`. Generate an access key for \
                 that user and supply it as JSON: \
                 {\"access_key_id\":\"AKIA...\",\"secret_access_key\":\"...\",\
                 \"region\":\"us-east-1\",\"service\":\"ce\"}. \
                 The IAM policy itself enforces read-only — NyxID does not \
                 mediate that boundary."
                    .to_string(),
            ),
            api_key_url: Some(
                "https://console.aws.amazon.com/iam/home#/users".to_string(),
            ),
            icon_url: None,
            documentation_url: Some(
                "https://docs.aws.amazon.com/aws-cost-management/latest/APIReference/API_GetCostAndUsage.html"
                    .to_string(),
            ),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "aws-billing", "Seeded default provider: AWS Billing");
        seeded_count += 1;
    }

    // Google Cloud (OAuth2) — replaces the old gcp-billing / gcp-bigquery
    // service-account seeds (NyxID#778). One provider entry; users pick the
    // actual Google API host per `nyxid service add` via --endpoint-url.
    // Default scope is the read-only umbrella; --scope can add others.
    if !slug_exists!("google-cloud") {
        let provider = ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: "google-cloud".to_string(),
            name: "Google Cloud".to_string(),
            description: Some(
                "Google Cloud APIs (Cloud Billing, BigQuery, Compute, etc.) via \
                 OAuth 2.0 user accounts. Supply the concrete API host via \
                 --endpoint-url when adding the service (e.g. \
                 https://cloudbilling.googleapis.com). A single OAuth token \
                 (from this provider) is reused across any number of Google API \
                 hosts. Default scope: cloud-platform.read-only."
                    .to_string(),
            ),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://accounts.google.com/o/oauth2/v2/auth".to_string()),
            token_url: Some("https://oauth2.googleapis.com/token".to_string()),
            revocation_url: Some("https://oauth2.googleapis.com/revoke".to_string()),
            default_scopes: Some(vec![
                "https://www.googleapis.com/auth/cloud-platform.read-only".to_string(),
            ]),
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
            documentation_url: Some("https://cloud.google.com/docs/authentication".to_string()),
            is_active: true,
            credential_mode: "user".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            // Google only returns a refresh_token when the auth request carries
            // access_type=offline + prompt=consent. Without these the user gets
            // a 1-hour access token and broker calls start failing the moment
            // it expires. Matches the seeded `google` provider above.
            extra_auth_params: Some(HashMap::from([
                ("access_type".to_string(), "offline".to_string()),
                ("prompt".to_string(), "consent".to_string()),
            ])),
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(
            slug = "google-cloud",
            "Seeded default provider: Google Cloud"
        );
        seeded_count += 1;
    }

    // Note once per startup that the google-cloud provider has no
    // platform-level OAuth client configured. This is the default fresh-deploy
    // state. It is NOT a hard error: with credential_mode=user (the seeded
    // default), users can still connect by passing --oauth-client-id /
    // --oauth-client-secret-env on `nyxid service add api-google-cloud
    // --oauth`. Admins who want a platform default can PATCH
    // /api/v1/providers/google-cloud with a client_id/secret and switch
    // credential_mode to "admin" or "both".
    if let Some(gc) = collection.find_one(doc! { "slug": "google-cloud" }).await?
        && gc.client_id_encrypted.is_none()
    {
        tracing::info!(
            slug = "google-cloud",
            "google-cloud provider has no platform OAuth client configured. \
             Users can still connect via BYO credentials \
             (--oauth-client-id / --oauth-client-secret-env on \
             `nyxid service add api-google-cloud --oauth`). To set a platform \
             default, PATCH /api/v1/providers/google-cloud with a \
             client_id/secret and flip credential_mode to \"admin\" or \"both\"."
        );
    }

    if seeded_count > 0 {
        tracing::info!(count = seeded_count, "Default provider seeding complete");
    }

    // Migration (NyxID#238): normalize `credential_mode` on `api_key` providers
    // to "admin". The field only meaningfully gates OAuth client-credential
    // setup for oauth2/device_code providers; on api_key it's inert, and the
    // API now rejects non-"admin" values on create/update. Earlier seeds
    // (telegram-bot, lark-bot, feishu-bot, discord-bot) inserted "user",
    // which would block admin edits of those rows. Idempotent via $ne filter.
    let api_key_mode_migration = collection
        .update_many(
            doc! {
                "provider_type": "api_key",
                "credential_mode": { "$ne": "admin" },
            },
            doc! { "$set": {
                "credential_mode": "admin",
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .await?;
    if api_key_mode_migration.modified_count > 0 {
        tracing::info!(
            count = api_key_mode_migration.modified_count,
            "Normalized credential_mode=admin on existing api_key providers (NyxID#238)"
        );
    }

    Ok(())
}

/// Whether a catalog slug belongs to the Lark / Feishu bot family, which
/// shares the declarative `token_exchange` config defined in the channel
/// adapter. Kept as a function so the matching logic has one home; new
/// token_exchange catalog entries should add their slug here (or grow a
/// proper per-seed config lookup if the list gets long).
fn is_lark_family_slug(slug: &str) -> bool {
    matches!(slug, "api-lark-bot" | "api-feishu-bot")
}

struct DefaultServiceSeed {
    provider_slug: &'static str,
    service_slug: &'static str,
    service_name: &'static str,
    base_url: &'static str,
    injection_method: &'static str,
    injection_key: &'static str,
    /// Optional override for the seeded `DownstreamService.auth_method`.
    /// When `None`, defaults to `"none"` (delegated/provider-managed).
    /// Set this for services where the user's static credential should be
    /// injected directly via the proxy `body`/`bot_bearer`/etc. methods.
    service_auth_method: Option<&'static str>,
    /// Optional override for `DownstreamService.auth_key_name`. Required
    /// when `service_auth_method` is `body`, `header`, `query`, or `path`.
    service_auth_key_name: Option<&'static str>,
    /// Optional rich description for the catalog entry. When `None`, a
    /// generic description is generated from the service name.
    description: Option<&'static str>,
    /// Required / recommended HTTP headers to inject on every proxy
    /// request for this service (e.g. `anthropic-version` on Anthropic).
    /// Applied at insert time and reconciled on every restart -- missing
    /// names are added back so a removed system header cannot
    /// permanently break proxy calls. Admin edits to the *value* of a
    /// seeded name are preserved (see
    /// `ensure_seeded_default_request_headers`).
    default_request_headers: Option<&'static [SeededHeader]>,
}

/// Static, serializable form of `DefaultRequestHeader` for seed tables.
/// Kept separate so the seed can live in a `const` without allocating.
struct SeededHeader {
    name: &'static str,
    value: &'static str,
    /// When `true`, a caller-supplied value (or any lower-precedence
    /// layer) wins over this default. Use `true` for headers the client
    /// is expected to set themselves (e.g. versioned API headers).
    overridable: bool,
    sensitive: bool,
}

struct SeededServiceContract {
    description: String,
    streaming_supported: bool,
    capabilities: Option<ServiceCapabilities>,
    identity_propagation_mode: &'static str,
    identity_include_user_id: bool,
    identity_include_email: bool,
    identity_include_name: bool,
    identity_jwt_audience: Option<String>,
    forward_access_token: bool,
    inject_delegation_token: bool,
    delegation_token_scope: &'static str,
    homepage_url: Option<&'static str>,
    repository_url: Option<&'static str>,
    issues_url: Option<&'static str>,
    auth_notes: Option<&'static str>,
    known_limitations: Option<&'static str>,
    required_permissions: Option<&'static [&'static str]>,
    examples_url: Option<&'static str>,
    recommended_skills: Option<&'static [&'static str]>,
}

/// Per-slug capability overrides for seeded services.
///
/// Returns explicit `ServiceCapabilities` and `streaming_supported` values
/// for services whose WebSocket / streaming behavior is not
/// auto-discoverable from an OpenAPI or AsyncAPI spec. Keeps the seed
/// table declarative and lets the discovery endpoint surface accurate
/// capability flags to clients.
fn seed_capability_override(slug: &str) -> Option<(ServiceCapabilities, bool)> {
    match slug {
        // OpenClaw Gateway speaks native WebSocket to its CLI/TUI
        // clients. The HTTP proxy path is supported but not all
        // downstream instances expose OpenAI-compatible HTTP endpoints,
        // so advertise WS + streaming so clients pick the right
        // transport. See ChronoAIProject/NyxID#160.
        "llm-openclaw" => Some((
            ServiceCapabilities {
                supports_proxy_read: true,
                supports_proxy_write: true,
                supports_proxy_binary_upload: false,
                supports_direct_downstream_auth: true,
                supports_authoring_via_nyx: false,
                supports_websocket: true,
                supports_streaming: true,
            },
            true,
        )),
        "aevatar" => Some((
            ServiceCapabilities {
                supports_proxy_read: true,
                supports_proxy_write: true,
                supports_proxy_binary_upload: false,
                supports_direct_downstream_auth: true,
                supports_authoring_via_nyx: false,
                supports_websocket: false,
                supports_streaming: true,
            },
            true,
        )),
        _ => None,
    }
}

fn seeded_service_contract(seed: &DefaultServiceSeed) -> SeededServiceContract {
    let is_llm_service = seed.service_slug.starts_with("llm-");
    let description = seed.description.map(String::from).unwrap_or_else(|| {
        if is_llm_service {
            format!("{} proxied via NyxID LLM gateway", seed.service_name)
        } else {
            format!("{} proxied via NyxID proxy", seed.service_name)
        }
    });
    let (capabilities, streaming_supported) = match seed_capability_override(seed.service_slug) {
        Some((caps, streaming)) => (Some(caps), streaming),
        None => (None, false),
    };

    let mut contract = SeededServiceContract {
        description,
        streaming_supported,
        capabilities,
        identity_propagation_mode: "none",
        identity_include_user_id: false,
        identity_include_email: false,
        identity_include_name: false,
        identity_jwt_audience: None,
        forward_access_token: false,
        inject_delegation_token: false,
        delegation_token_scope: if is_llm_service {
            "llm:proxy"
        } else {
            "proxy:*"
        },
        homepage_url: None,
        repository_url: None,
        issues_url: None,
        auth_notes: None,
        known_limitations: None,
        required_permissions: None,
        examples_url: None,
        recommended_skills: None,
    };

    if seed.service_slug == "aevatar" {
        contract.identity_propagation_mode = "jwt";
        contract.identity_include_user_id = true;
        contract.identity_include_email = true;
        contract.delegation_token_scope = "llm:proxy";
        contract.auth_notes = Some(
            "NyxID injects the user's aevatar bearer credential as Authorization: Bearer <token> and sends a short-lived X-NyxID-Identity-Token JWT. The raw caller access token is not forwarded.",
        );
        contract.known_limitations = Some(
            "The aevatar instance URL is user supplied. NyxID does not provide a dedicated /llm/aevatar namespace or translate request formats; clients call the aevatar runtime through the existing slug proxy.",
        );
        contract.required_permissions = Some(&[
            "aevatar:responses",
            "aevatar:streaming",
            "nyxid:identity-jwt",
        ]);
        contract.recommended_skills = Some(&["nyxid"]);
    }

    contract
}

/// Required Anthropic API headers. `anthropic-version` is mandatory on every
/// request (rejected with 400 if missing); `content-type` is required for
/// the JSON-bodied endpoints that make up the vast majority of the API.
/// Both are `overridable: true` so SDKs that send their own
/// `anthropic-version` (every official SDK) and clients that already set
/// `content-type` continue to win -- the defaults only kick in when the
/// caller omits them.
const ANTHROPIC_DEFAULT_HEADERS: &[SeededHeader] = &[
    SeededHeader {
        name: "anthropic-version",
        value: "2023-06-01",
        overridable: true,
        sensitive: false,
    },
    SeededHeader {
        name: "content-type",
        value: "application/json",
        overridable: true,
        sensitive: false,
    },
];

const DEFAULT_SERVICE_SEEDS: &[DefaultServiceSeed] = &[
    DefaultServiceSeed {
        provider_slug: "openai",
        service_slug: "llm-openai",
        service_name: "OpenAI API",
        base_url: "https://api.openai.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "openai-codex",
        service_slug: "llm-openai-codex",
        service_name: "OpenAI Codex API",
        base_url: "https://chatgpt.com/backend-api/codex",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "anthropic",
        service_slug: "llm-anthropic",
        service_name: "Anthropic API",
        base_url: "https://api.anthropic.com/v1",
        injection_method: "header",
        injection_key: "x-api-key",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: Some(ANTHROPIC_DEFAULT_HEADERS),
    },
    DefaultServiceSeed {
        provider_slug: "google-ai",
        service_slug: "llm-google-ai",
        service_name: "Google AI API",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        injection_method: "header",
        injection_key: "x-goog-api-key",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "mistral",
        service_slug: "llm-mistral",
        service_name: "Mistral AI API",
        base_url: "https://api.mistral.ai/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "cohere",
        service_slug: "llm-cohere",
        service_name: "Cohere API",
        base_url: "https://api.cohere.com/v2",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "deepseek",
        service_slug: "llm-deepseek",
        service_name: "DeepSeek API",
        base_url: "https://api.deepseek.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "twitter",
        service_slug: "api-twitter",
        service_name: "Twitter / X API",
        base_url: "https://api.x.com/2",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "google",
        service_slug: "api-google",
        service_name: "Google API",
        base_url: "https://www.googleapis.com",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "github",
        service_slug: "api-github",
        service_name: "GitHub API",
        base_url: "https://api.github.com",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "github-pat",
        service_slug: "api-github-pat",
        service_name: "GitHub API (Personal Access Token)",
        base_url: "https://api.github.com",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: Some("bearer"),
        service_auth_key_name: Some("Authorization"),
        description: Some(
            "GitHub REST API access using a Personal Access Token (classic or fine-grained). \
             Use this when you want a long-lived static credential instead of OAuth.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "facebook",
        service_slug: "api-facebook",
        service_name: "Facebook Graph API",
        base_url: "https://graph.facebook.com/v21.0",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "discord",
        service_slug: "api-discord",
        service_name: "Discord API",
        base_url: "https://discord.com/api/v10",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "spotify",
        service_slug: "api-spotify",
        service_name: "Spotify API",
        base_url: "https://api.spotify.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "slack",
        service_slug: "api-slack",
        service_name: "Slack API (User OAuth)",
        base_url: "https://slack.com/api",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: Some(
            "Slack Web API authenticated as the installing user via OAuth 2.0. \
             Use this when the agent should act on behalf of a real Slack user. \
             For bot-level access with a long-lived `xoxb-` token, use \
             `api-slack-bot` instead.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "microsoft",
        service_slug: "api-microsoft",
        service_name: "Microsoft Graph API",
        base_url: "https://graph.microsoft.com/v1.0",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "tiktok",
        service_slug: "api-tiktok",
        service_name: "TikTok API",
        base_url: "https://open.tiktokapis.com/v2",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "twitch",
        service_slug: "api-twitch",
        service_name: "Twitch API",
        base_url: "https://api.twitch.tv/helix",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "reddit",
        service_slug: "api-reddit",
        service_name: "Reddit API",
        base_url: "https://oauth.reddit.com",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "lark",
        service_slug: "api-lark",
        service_name: "Lark API (User OAuth)",
        base_url: "https://open.larksuite.com/open-apis",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: Some(
            "Lark API authenticated as a logged-in user via OAuth 2.0. \
             Use this when the bot should act on behalf of a real Lark user. \
             For bot/tenant-level access, use `api-lark-bot` instead.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "lark-bot",
        service_slug: "api-lark-bot",
        service_name: "Lark Bot API (Transparent Tenant Token)",
        base_url: "https://open.larksuite.com",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: Some("token_exchange"),
        service_auth_key_name: None,
        description: Some(
            "Lark bot tenant APIs. NyxID handles token exchange transparently: store your \
             `app_id` and `app_secret` once, and NyxID POSTs them to `/auth/v3/tenant_access_token/internal`, \
             caches the resulting `tenant_access_token` (~2h TTL), and injects it as \
             `Authorization: Bearer <token>` on every outbound request. You never refresh tokens, \
             never touch the exchange endpoint, and your `app_secret` never leaves NyxID. \
             Call any Lark open API path (`/open-apis/im/v1/chats`, `/open-apis/contact/v3/users`, etc.) \
             directly through the proxy.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "telegram-bot",
        service_slug: "api-telegram-bot",
        service_name: "Telegram Bot API",
        base_url: "https://api.telegram.org",
        injection_method: "path",
        injection_key: "bot",
        service_auth_method: Some("path"),
        service_auth_key_name: Some("bot"),
        description: Some(
            "Telegram Bot API. Get a token from @BotFather and store it once. Pass only the \
             Bot API method name in the proxy path (e.g. `sendMessage`, `setWebhook`, \
             `getWebhookInfo`) -- NyxID automatically prepends `bot<token>/` so the request \
             goes to `https://api.telegram.org/bot<token>/<method>`. Do NOT include `bot/` \
             yourself; that would double-prefix the path. Works for messages, files, \
             webhooks, payments, mini apps -- all Bot API methods use the same shape.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "feishu",
        service_slug: "api-feishu",
        service_name: "Feishu API (User OAuth)",
        base_url: "https://open.feishu.cn/open-apis",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: Some(
            "Feishu API authenticated as a logged-in user via OAuth 2.0 (China region). \
             Use this when the bot should act on behalf of a real Feishu user. \
             For bot/tenant-level access, use `api-feishu-bot` instead.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "feishu-bot",
        service_slug: "api-feishu-bot",
        service_name: "Feishu Bot API (Transparent Tenant Token)",
        base_url: "https://open.feishu.cn",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: Some("token_exchange"),
        service_auth_key_name: None,
        description: Some(
            "Feishu bot tenant APIs (China region). Same as `api-lark-bot` but for the Feishu domain. \
             NyxID handles token exchange transparently: store your `app_id` and `app_secret` once, \
             NyxID caches the `tenant_access_token` and injects it as a Bearer header on every \
             outbound request. Call any Feishu open API path directly through the proxy.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "discord-bot",
        service_slug: "api-discord-bot",
        service_name: "Discord Bot API",
        base_url: "https://discord.com/api/v10",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: Some("bot_bearer"),
        service_auth_key_name: Some("Authorization"),
        description: Some(
            "Discord Bot API. NyxID stores your bot token and injects it as \
             `Authorization: Bot <token>` (note: `Bot`, not `Bearer`) on outbound calls. \
             Bot tokens are persistent and never expire. Use for sending channel messages, \
             managing guilds, and any other Discord bot operations.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "slack-bot",
        service_slug: "api-slack-bot",
        service_name: "Slack Bot API",
        base_url: "https://slack.com/api",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: Some(
            "Slack Web API authenticated as the app's bot user. Paste a `xoxb-` \
             bot token once (from your Slack app's OAuth & Permissions page), and \
             NyxID injects it as `Authorization: Bearer <token>` on every outbound \
             call. Bot tokens are long-lived and only rotate when you reinstall \
             the app or rotate in the Slack UI. Use for `chat.postMessage`, \
             `conversations.history`, `users.info`, and any other Web API method \
             outside the channel-relay flow.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "openclaw",
        service_slug: "llm-openclaw",
        service_name: "OpenClaw Gateway",
        // Placeholder: each user provides their own gateway URL when connecting.
        // resolve_gateway_url_override() replaces this at proxy time.
        base_url: "https://openclaw-gateway.invalid",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: None,
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "aevatar",
        service_slug: "aevatar",
        service_name: "aevatar",
        base_url: "https://aevatar-gateway.invalid",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: Some(
            "Self-hosted aevatar runtime exposed through NyxID's slug proxy. Use this \
             catalog entry for Codex, Cursor, and channel inbound routing that speaks \
             OpenAI Responses-compatible HTTP and SSE streaming. Users provide their \
             own gateway URL and bearer token.",
        ),
        default_request_headers: None,
    },
    // Cloud-billing catalog entries (NyxID#716, #778). AWS uses direct
    // (non-delegated) sigv4 injection. Google Cloud uses delegated OAuth
    // via the `google-cloud` provider (service_auth_method=None so a
    // ServiceProviderRequirement is created; user supplies host via
    // --endpoint-url on `nyxid service add`, matching llm-openclaw pattern).
    DefaultServiceSeed {
        provider_slug: "aws-billing",
        service_slug: "aws-cost-explorer",
        service_name: "AWS Cost Explorer",
        // Cost Explorer is a single-region service — every request
        // goes to us-east-1 regardless of where the workload runs.
        base_url: "https://ce.us-east-1.amazonaws.com",
        injection_method: "header",
        injection_key: "Authorization",
        service_auth_method: Some("aws_sigv4"),
        service_auth_key_name: None,
        description: Some(
            "AWS Cost Explorer (GetCostAndUsage / GetCostAndUsageWithResources) for \
             consolidated billing visibility across the AWS Organization. Must be \
             called with management/payer account credentials — linked-account \
             keys return AccessDenied. Costs $0.01 per paginated request, so \
             cache aggressively.",
        ),
        default_request_headers: None,
    },
    DefaultServiceSeed {
        provider_slug: "google-cloud",
        service_slug: "api-google-cloud",
        service_name: "Google Cloud API",
        // Placeholder: each user provides their own Google API host when
        // connecting (via --endpoint-url). Matches the llm-openclaw pattern.
        // The OAuth token from the `google-cloud` provider is reused across
        // any number of hosts (Cloud Billing, BigQuery, Compute, ...).
        base_url: "https://googleapis.invalid",
        injection_method: "bearer",
        injection_key: "Authorization",
        service_auth_method: None,
        service_auth_key_name: None,
        description: Some(
            "Generic Google Cloud API endpoint. Supply the real host via \
             --endpoint-url at add time (e.g. https://cloudbilling.googleapis.com, \
             https://bigquery.googleapis.com, https://compute.googleapis.com). \
             One OAuth user token (google-cloud provider) works for any Google \
             API host. Default scope cloud-platform.read-only; extend with \
             --scope for write APIs.",
        ),
        default_request_headers: None,
    },
];

/// Apply per-slug capability / streaming overrides to pre-existing seeded
/// downstream services. Designed to be a one-shot migration that runs on
/// every startup but only mutates rows that still carry the legacy
/// "no capabilities declared" shape from before these overrides existed.
///
/// The filter is intentionally narrow to avoid two classes of bug:
///
///   1. **Thrashing `updated_at` on every boot.** We'd otherwise rewrite
///      the row every startup (MongoDB treats `$set: {updated_at: ...}`
///      as a modification even if the other fields are unchanged).
///   2. **Overwriting admin customizations.** If an admin has explicitly
///      edited `capabilities` via the frontend/API, the row already has
///      a non-null `capabilities` document and we leave it alone.
///
/// No upsert, no `insert_one`. If the row doesn't exist yet, the seed
/// loop below creates it with the correct capabilities -- this function
/// is purely a forward-migration for already-seeded deployments. No
/// duplicate rows are possible because `update_one` never inserts.
///
/// See ChronoAIProject/NyxID#160.
async fn backfill_seeded_capability_overrides(
    service_col: &mongodb::Collection<DownstreamService>,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    // Slugs we upgrade in-place. Keep this short -- if the list grows,
    // iterate DEFAULT_SERVICE_SEEDS instead.
    const BACKFILL_SLUGS: &[&str] = &["llm-openclaw"];

    for slug in BACKFILL_SLUGS {
        let Some((caps, streaming)) = seed_capability_override(slug) else {
            continue;
        };

        // Serialize ServiceCapabilities into a BSON document so MongoDB
        // stores the nested object as-is instead of a serde wire format.
        let caps_bson = match bson::to_bson(&caps) {
            Ok(bson::Bson::Document(doc)) => doc,
            Ok(other) => {
                tracing::warn!(
                    slug = %slug,
                    actual = ?other,
                    "Unexpected BSON shape for ServiceCapabilities; skipping backfill"
                );
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    slug = %slug,
                    error = %error,
                    "Failed to encode ServiceCapabilities; skipping backfill"
                );
                continue;
            }
        };

        // Only touch rows whose capabilities are effectively unset.
        //
        // Three legacy shapes exist in production:
        //   1. `capabilities` missing entirely (pre-feature seed row).
        //   2. `capabilities: null` (explicit null from early writers).
        //   3. `capabilities: { all flags false }` -- the admin service
        //      editor serializes `ServiceCapabilities::default()` into
        //      the document whenever an admin saves an unrelated field
        //      (description, base_url, ...) on a row that never had
        //      capabilities authored. Skipping these would leave the
        //      OpenClaw discovery fix unapplied for any deployment
        //      where an admin ever touched the row.
        //
        // The third clause uses `$ne: true` on every flag, which matches
        // both "field missing" and "field explicitly false". If any flag
        // is `true`, the row is a deliberate admin customization and we
        // leave it alone.
        //
        // `slug` has a partial unique index on `is_active: true`, so at
        // most one active row per slug exists -- but soft-deleted or
        // deactivated rows with the same slug can coexist. We use
        // update_many (not update_one) so that both the active row and
        // any inactive historical rows get backfilled; otherwise a
        // single update_one could match an inactive row first, leaving
        // the active row (the one `/api/v1/proxy/services` serves) with
        // stale metadata.
        let filter = doc! {
            "slug": *slug,
            "created_by": "system",
            "$or": [
                { "capabilities": { "$exists": false } },
                { "capabilities": null },
                { "$and": [
                    { "capabilities.supports_proxy_read": { "$ne": true } },
                    { "capabilities.supports_proxy_write": { "$ne": true } },
                    { "capabilities.supports_proxy_binary_upload": { "$ne": true } },
                    { "capabilities.supports_direct_downstream_auth": { "$ne": true } },
                    { "capabilities.supports_authoring_via_nyx": { "$ne": true } },
                    { "capabilities.supports_websocket": { "$ne": true } },
                    { "capabilities.supports_streaming": { "$ne": true } },
                ]},
            ],
        };

        let result = service_col
            .update_many(
                filter,
                doc! { "$set": {
                    "capabilities": caps_bson,
                    "streaming_supported": streaming,
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;

        if result.matched_count > 0 {
            tracing::info!(
                slug = %slug,
                matched = result.matched_count,
                modified = result.modified_count,
                "Backfilled catalog capability overrides"
            );
        }
    }

    Ok(())
}

/// Backfill missing `ServiceProviderRequirement` rows for provider-delegated
/// seeded services. Idempotent: inserts only when no SPR exists for the
/// DownstreamService's id.
///
/// The catalog builder reads `injection_method` / `injection_key` off the SPR
/// to fill in `auth_method` on `auth_method = "none"` services; without the
/// SPR the frontend renders the entry as no-auth and users cannot attach a
/// credential. The main seed loop only creates the SPR when inserting a new
/// service, so deployments whose service row predates or has drifted from
/// the SPR never recover on their own.
async fn backfill_missing_service_provider_requirements(
    provider_col: &mongodb::Collection<ProviderConfig>,
    service_col: &mongodb::Collection<DownstreamService>,
    req_col: &mongodb::Collection<ServiceProviderRequirement>,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let mut backfilled: u32 = 0;

    for seed in DEFAULT_SERVICE_SEEDS {
        // Direct-auth seeds (body / bot_bearer / path / ...) intentionally
        // have no SPR -- credentials live on the user's UserApiKey and are
        // injected via the static `auth_method` on the DownstreamService.
        if seed.service_auth_method.is_some() {
            continue;
        }

        let Some(service) = service_col
            .find_one(doc! { "slug": seed.service_slug })
            .await?
        else {
            continue;
        };
        if seed.service_slug == "aevatar" && service.created_by != "system" {
            continue;
        }

        let existing = req_col.find_one(doc! { "service_id": &service.id }).await?;
        if existing.is_some() {
            continue;
        }

        // Prefer the service's own provider_config_id so we never link the
        // SPR to a different provider than the service itself points at.
        // Fall back to a slug lookup for legacy rows missing the reference.
        let provider_id = match service.provider_config_id.clone() {
            Some(id) => id,
            None => match provider_col
                .find_one(doc! { "slug": seed.provider_slug })
                .await?
            {
                Some(p) => p.id,
                None => continue,
            },
        };

        let requirement = ServiceProviderRequirement {
            id: Uuid::new_v4().to_string(),
            service_id: service.id.clone(),
            provider_config_id: provider_id,
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
            injection_method = seed.injection_method,
            injection_key = seed.injection_key,
            "Backfilled missing ServiceProviderRequirement"
        );
        backfilled += 1;
    }

    if backfilled > 0 {
        tracing::info!(
            count = backfilled,
            "ServiceProviderRequirement backfill complete"
        );
    }

    Ok(())
}

fn seeded_header_to_model(seed: &SeededHeader) -> DefaultRequestHeader {
    DefaultRequestHeader {
        name: seed.name.to_string(),
        value: seed.value.to_string(),
        overridable: seed.overridable,
        sensitive: seed.sensitive,
    }
}

/// Pure merge step for the seeded-header reconciler. Returns `Some(new_list)`
/// when the stored list must be updated, or `None` when no change is needed.
///
/// Missing / empty stored lists receive every seeded entry. Non-empty stored
/// lists keep all existing entries (including admin-edited values) and only
/// gain seeded entries whose names are absent (case-insensitive).
fn reconcile_seeded_headers(
    stored: Option<&[DefaultRequestHeader]>,
    seeded: &[SeededHeader],
) -> Option<Vec<DefaultRequestHeader>> {
    if seeded.is_empty() {
        return None;
    }
    let existing = stored.unwrap_or(&[]);

    let mut merged: Vec<DefaultRequestHeader> = existing.to_vec();
    let mut added = false;
    for entry in seeded {
        let already_present = merged
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case(entry.name));
        if !already_present {
            merged.push(seeded_header_to_model(entry));
            added = true;
        }
    }

    if !added {
        return None;
    }
    Some(merged)
}

/// Reconcile seeded `default_request_headers` onto every seeded
/// `DownstreamService` whose seed declares required headers. Runs on every
/// restart so admins cannot permanently drop a system-required header.
///
/// Policy:
///   - Missing / null `default_request_headers` OR empty list on the stored
///     row -> write the full seeded list.
///   - Non-empty stored list -> append any seeded header whose name (case-
///     insensitive) is not already present. Existing entries (including
///     admin-edited values, overridable flags, added custom headers) are
///     left untouched.
///   - Only issues a Mongo write when the resulting list actually differs
///     from what is stored.
///
/// Idempotent: a second run on a reconciled row is a no-op.
async fn ensure_seeded_default_request_headers(
    service_col: &mongodb::Collection<DownstreamService>,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let mut updated: u32 = 0;

    for seed in DEFAULT_SERVICE_SEEDS {
        let Some(seeded) = seed.default_request_headers else {
            continue;
        };
        let Some(service) = service_col
            .find_one(doc! { "slug": seed.service_slug })
            .await?
        else {
            continue;
        };

        let existing_len = service
            .default_request_headers
            .as_ref()
            .map(|h| h.len())
            .unwrap_or(0);
        let Some(merged) =
            reconcile_seeded_headers(service.default_request_headers.as_deref(), seeded)
        else {
            continue;
        };

        let headers_bson = bson::to_bson(&merged).map_err(|e| {
            AppError::Internal(format!("Failed to serialize default_request_headers: {e}"))
        })?;

        service_col
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$set": {
                    "default_request_headers": headers_bson,
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;

        tracing::info!(
            slug = seed.service_slug,
            added = merged.len().saturating_sub(existing_len),
            had_existing = existing_len > 0,
            "Reconciled seeded default_request_headers"
        );
        updated += 1;
    }

    if updated > 0 {
        tracing::info!(
            count = updated,
            "Seeded default_request_headers reconcile complete"
        );
    }

    Ok(())
}

/// Seed downstream services for each default provider (idempotent).
///
/// Creates a `DownstreamService` and a `ServiceProviderRequirement` for each
/// seeded provider that does not yet have a corresponding downstream service.
pub async fn seed_default_services(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
) -> AppResult<()> {
    let provider_col = db.collection::<ProviderConfig>(COLLECTION_NAME);
    let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    let req_col = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);
    let now = Utc::now();
    let mut seeded_count: u32 = 0;

    // Upgrade existing openai-codex downstream service base_url
    // (was api.openai.com/v1, now chatgpt.com/backend-api/codex)
    let old_codex_url = "https://api.openai.com/v1";
    let new_codex_url = "https://chatgpt.com/backend-api/codex";
    if let Ok(Some(_)) = service_col
        .find_one(doc! {
            "slug": "llm-openai-codex",
            "base_url": old_codex_url,
        })
        .await
    {
        service_col
            .update_one(
                doc! { "slug": "llm-openai-codex", "base_url": old_codex_url },
                doc! { "$set": {
                    "base_url": new_codex_url,
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;
        tracing::info!(
            slug = "llm-openai-codex",
            "Updated existing downstream service base_url to chatgpt.com"
        );
    }

    // Backfill capability + streaming flags for seeded services whose
    // WebSocket / streaming support is known at the proxy layer but was
    // not captured on the original seed row (e.g. llm-openclaw). The
    // idempotent seed loop below skips rows that already exist, so
    // without this step pre-existing deployments keep reporting
    // `streaming_supported: false` even after the seed is upgraded.
    // See ChronoAIProject/NyxID#160.
    backfill_seeded_capability_overrides(&service_col, now).await?;

    // Backfill `ServiceProviderRequirement` rows for provider-delegated
    // seeded services (`service_auth_method: None`) that exist in
    // `downstream_services` but are missing their SPR. Without the SPR,
    // `catalog_service::build_catalog_entry` falls back to
    // `auth_method = "none"`, so the AI Services dialog renders the
    // service as no-auth and the user cannot attach a credential. The
    // main seed loop below only creates the SPR when it inserts a fresh
    // DownstreamService, so pre-existing deployments that lost (or
    // never had) the SPR are otherwise stuck.
    backfill_missing_service_provider_requirements(&provider_col, &service_col, &req_col, now)
        .await?;

    // Reconcile seeded `default_request_headers` on every restart so a
    // system-required header (e.g. `anthropic-version`, required by the
    // Anthropic API on every request) can't be permanently removed by an
    // admin or lost by a pre-existing seed row created before the field
    // was introduced. Missing / empty lists receive the full seed;
    // non-empty lists keep admin edits and only gain entries whose names
    // are absent. See `ensure_seeded_default_request_headers` for the
    // full policy.
    ensure_seeded_default_request_headers(&service_col, now).await?;

    for seed in DEFAULT_SERVICE_SEEDS {
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

        if seed.service_slug == "aevatar"
            && service_col
                .find_one(doc! { "slug": seed.service_slug, "is_active": true })
                .await?
                .is_some()
        {
            tracing::info!(
                slug = seed.service_slug,
                "Skipping default service seed because an active service already uses this slug"
            );
            continue;
        }

        // Create an empty encrypted credential (field is required)
        let empty_credential = encryption_keys.encrypt(b"").await?;

        let service_id = Uuid::new_v4().to_string();
        let contract = seeded_service_contract(seed);

        // For services with a static auth method (e.g. body, bot_bearer),
        // the credential is stored on the user's UserService and injected by
        // the proxy directly. For provider-managed services, auth_method
        // stays "none" and the requirement record drives injection.
        let service_auth_method = seed.service_auth_method.unwrap_or("none").to_string();
        let service_auth_key_name = seed
            .service_auth_key_name
            .map(String::from)
            .unwrap_or_default();

        // Populate the token_exchange_config for catalog services that use
        // the declarative token_exchange auth method. Lark and Feishu share
        // the same config shape (only the base_url differs) and pull their
        // definition from the channel adapter so there is one source of
        // truth in the tree.
        let token_exchange_config =
            if service_auth_method == "token_exchange" && is_lark_family_slug(seed.service_slug) {
                Some(crate::services::channel_adapters::lark::lark_family_token_exchange_config())
            } else {
                None
            };

        let default_request_headers = seed
            .default_request_headers
            .map(|entries| entries.iter().map(seeded_header_to_model).collect());

        let service = DownstreamService {
            id: service_id.clone(),
            name: seed.service_name.to_string(),
            slug: seed.service_slug.to_string(),
            description: Some(contract.description),
            base_url: seed.base_url.to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: service_auth_method,
            auth_key_name: service_auth_key_name,
            credential_encrypted: empty_credential,
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: contract.streaming_supported,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "internal".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: "system".to_string(),
            identity_propagation_mode: contract.identity_propagation_mode.to_string(),
            identity_include_user_id: contract.identity_include_user_id,
            identity_include_email: contract.identity_include_email,
            identity_include_name: contract.identity_include_name,
            identity_jwt_audience: contract.identity_jwt_audience,
            forward_access_token: contract.forward_access_token,
            inject_delegation_token: contract.inject_delegation_token,
            delegation_token_scope: contract.delegation_token_scope.to_string(),
            provider_config_id: Some(provider.id.clone()),
            homepage_url: contract.homepage_url.map(str::to_string),
            repository_url: contract.repository_url.map(str::to_string),
            issues_url: contract.issues_url.map(str::to_string),
            capabilities: contract.capabilities,
            auth_notes: contract.auth_notes.map(str::to_string),
            known_limitations: contract.known_limitations.map(str::to_string),
            required_permissions: contract
                .required_permissions
                .map(|values| values.iter().map(|value| value.to_string()).collect()),
            examples_url: contract.examples_url.map(str::to_string),
            recommended_skills: contract
                .recommended_skills
                .map(|values| values.iter().map(|value| value.to_string()).collect()),
            custom_user_agent: None,
            default_request_headers,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config,
            created_at: now,
            updated_at: now,
        };

        service_col.insert_one(&service).await?;

        // Direct-auth services (body / bot_bearer / etc.) store the user's
        // credential on a UserApiKey and inject it at proxy time via the
        // static `auth_method` on the DownstreamService. They must NOT have
        // a ServiceProviderRequirement, because the proxy's delegated
        // credential resolver would then look for a UserProviderToken that
        // the `nyxid service add` flow never creates, causing a
        // "Provider connection required" error on every request.
        let is_direct_auth = seed.service_auth_method.is_some();

        if !is_direct_auth {
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
        }

        tracing::info!(
            slug = seed.service_slug,
            provider = seed.provider_slug,
            direct_auth = is_direct_auth,
            "Seeded default downstream service"
        );
        seeded_count += 1;
    }

    if seeded_count > 0 {
        tracing::info!(
            count = seeded_count,
            "Default downstream service seeding complete"
        );
    }

    // Migration: update name + description on existing seeded services
    // whose description still matches a known auto-generated or previously
    // seeded value. Lets existing deployments pick up richer catalog
    // descriptions without overwriting any user-customized text.
    //
    // We track three classes of "safe to overwrite":
    //   1. The original auto-generated suffixes from the very first seed.
    //   2. Known prior seed descriptions we have shipped and later corrected
    //      (listed in `STALE_SEED_DESCRIPTIONS`). When a seed description
    //      gets corrected, add its previous text here so the next backend
    //      restart propagates the fix to existing deployments.
    //   3. Empty/missing descriptions (always safe).
    const AUTO_PROXY_SUFFIX: &str = " proxied via NyxID proxy";
    const AUTO_GATEWAY_SUFFIX: &str = " proxied via NyxID LLM gateway";

    // Previously-shipped seed descriptions that have since been corrected.
    // Listed exactly so the migration can recognize and overwrite them.
    const STALE_SEED_DESCRIPTIONS: &[&str] = &[
        // api-telegram-bot, shipped in #205. Replaced in #207 follow-up to
        // remove the misleading `/bot{token}/` example that suggested
        // callers should include `bot/` in their proxy path.
        "Telegram Bot API authenticated by injecting the bot token into the URL path. \
         Get a token from @BotFather and store it once -- the proxy injects it as `/bot{token}/` \
         on every request. Use for sending messages, managing webhooks, and any other \
         Telegram bot operations.",
        // api-lark-bot, shipped in #205 with body auth. Replaced in #220
        // with the `lark_token_exchange` auth method that performs token
        // exchange server-side transparently.
        "Lark bot tenant credentials. NyxID stores your `app_secret` and injects \
         it into the request body when you call `/open-apis/auth/v3/tenant_access_token/internal`. \
         Send `{\"app_id\": \"cli_xxx\"}` and NyxID merges in the secret server-side. \
         Returns a `tenant_access_token` valid for 2 hours that you cache and use as \
         a Bearer token for subsequent Lark API calls. Your `app_secret` never leaves NyxID.",
        // api-feishu-bot, same story.
        "Feishu bot tenant credentials (China region). Same as `api-lark-bot` but for the \
         Feishu domain. NyxID stores your `app_secret` and injects it into the request body \
         when you call `/open-apis/auth/v3/tenant_access_token/internal`. \
         Returns a `tenant_access_token` valid for 2 hours.",
    ];

    // #220 migration: existing api-lark-bot / api-feishu-bot catalog rows
    // need to land on the declarative `token_exchange` auth method with a
    // populated `token_exchange_config`. Two waves are possible depending
    // on when the row was seeded:
    //   - #205: auth_method="body", auth_key_name="app_secret", no config
    //   - #220 pre-refactor: auth_method="lark_token_exchange", no config
    // Both need the same end state. Idempotent: the filter only matches
    // stale shapes and the update wipes the now-unused auth_key_name.
    //
    // Note: this only updates the catalog `DownstreamService` rows. User
    // `UserService` rows pointing at these catalog services still need to
    // be recreated by the user, because the credential format changed
    // from a raw `app_secret` string to a JSON `{app_id, app_secret}` blob.
    let lark_exchange_config =
        crate::services::channel_adapters::lark::lark_family_token_exchange_config();
    let lark_exchange_config_bson = bson::to_bson(&lark_exchange_config)
        .map_err(|e| AppError::Internal(format!("Failed to serialize TokenExchangeConfig: {e}")))?;

    for slug in ["api-lark-bot", "api-feishu-bot"] {
        let res = service_col
            .update_many(
                doc! {
                    "slug": slug,
                    "auth_method": { "$in": ["body", "lark_token_exchange"] },
                },
                doc! {
                    "$set": {
                        "auth_method": "token_exchange",
                        "auth_key_name": "",
                        "token_exchange_config": &lark_exchange_config_bson,
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await?;
        if res.modified_count > 0 {
            tracing::info!(
                slug = slug,
                modified = res.modified_count,
                "Migrated catalog service to declarative token_exchange auth"
            );
        }
    }

    // #274 migration: api-telegram-bot was originally seeded as a
    // provider-managed service (`auth_method = "none"` + a
    // ServiceProviderRequirement with `injection_method = "path"`). The
    // streamlined `/keys` flow stores the user's bot token on `UserApiKey`,
    // not `UserProviderToken`, so the stale requirement made the proxy ask
    // for a provider connection and ignored the direct credential. Move the
    // catalog row and any existing user service rows to direct path auth, and
    // remove the requirement so delegated provider lookup no longer runs.
    let telegram_services: Vec<DownstreamService> = service_col
        .find(doc! { "slug": "api-telegram-bot", "created_by": "system" })
        .await?
        .try_collect()
        .await?;
    let telegram_service_ids: Vec<String> =
        telegram_services.iter().map(|svc| svc.id.clone()).collect();

    if !telegram_service_ids.is_empty() {
        let now = Utc::now();
        let res = service_col
            .update_many(
                doc! {
                    "_id": { "$in": &telegram_service_ids },
                    "$or": [
                        { "auth_method": { "$ne": "path" } },
                        { "auth_key_name": { "$ne": "bot" } },
                    ],
                },
                doc! {
                    "$set": {
                        "auth_method": "path",
                        "auth_key_name": "bot",
                        "updated_at": bson::DateTime::from_chrono(now),
                    }
                },
            )
            .await?;
        if res.modified_count > 0 {
            tracing::info!(
                modified = res.modified_count,
                "Migrated Telegram Bot catalog service to direct path auth"
            );
        }

        let req_res = req_col
            .delete_many(doc! { "service_id": { "$in": &telegram_service_ids } })
            .await?;
        if req_res.deleted_count > 0 {
            tracing::info!(
                deleted = req_res.deleted_count,
                "Removed stale Telegram Bot provider requirements"
            );
        }

        let user_res = db
            .collection::<mongodb::bson::Document>(USER_SERVICES)
            .update_many(
                doc! {
                    "catalog_service_id": { "$in": &telegram_service_ids },
                    "auth_method": { "$ne": "path" },
                },
                doc! {
                    "$set": {
                        "auth_method": "path",
                        "auth_key_name": "bot",
                        "updated_at": bson::DateTime::from_chrono(now),
                    }
                },
            )
            .await?;
        if user_res.modified_count > 0 {
            tracing::info!(
                modified = user_res.modified_count,
                "Migrated Telegram Bot user services to direct path auth"
            );
        }
    }

    // Migration: llm-google-ai was originally seeded with query-param auth
    // (`?key=<api_key>`), which leaks into downstream access logs and
    // Referer headers. Flip the catalog row to Google's recommended
    // `x-goog-api-key` header instead. Strict filter on the original
    // seeded values so any admin customization is left alone. Existing
    // `UserService` rows are snapshots copied from the catalog at
    // provision time and are intentionally not touched -- the proxy reads
    // `auth_method` from those snapshots, so current users keep working
    // exactly as they do today. Only new auto-provisions pick up the
    // header method. The legacy delegated path reads the catalog live, so
    // it flips on the next proxy call; Gemini accepts both methods, so
    // there is no observable change downstream.
    let res = service_col
        .update_one(
            doc! {
                "slug": "llm-google-ai",
                "created_by": "system",
                "auth_method": "query",
                "auth_key_name": "key",
            },
            doc! {
                "$set": {
                    "auth_method": "header",
                    "auth_key_name": "x-goog-api-key",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;
    if res.modified_count > 0 {
        tracing::info!("Migrated llm-google-ai catalog to x-goog-api-key header auth");
    }

    let mut migrated_count: u32 = 0;
    for seed in DEFAULT_SERVICE_SEEDS {
        let Some(new_description) = seed.description else {
            continue;
        };

        // Find the existing service by slug
        let Some(existing) = service_col
            .find_one(doc! { "slug": seed.service_slug })
            .await?
        else {
            continue;
        };
        if seed.service_slug == "aevatar" && existing.created_by != "system" {
            continue;
        }

        // Only overwrite if the description looks auto-generated, empty,
        // or matches a known stale seed value we previously shipped.
        let is_safe_to_overwrite = match existing.description.as_deref() {
            None | Some("") => true,
            Some(d) => {
                d.ends_with(AUTO_PROXY_SUFFIX)
                    || d.ends_with(AUTO_GATEWAY_SUFFIX)
                    || STALE_SEED_DESCRIPTIONS.contains(&d)
            }
        };

        if !is_safe_to_overwrite {
            continue;
        }

        // Skip if both name and description already match (no-op update)
        if existing.name == seed.service_name
            && existing.description.as_deref() == Some(new_description)
        {
            continue;
        }

        service_col
            .update_one(
                doc! { "_id": &existing.id },
                doc! {
                    "$set": {
                        "name": seed.service_name,
                        "description": new_description,
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await?;

        tracing::info!(
            slug = seed.service_slug,
            "Migrated catalog service name + description to current seed"
        );
        migrated_count += 1;
    }

    if migrated_count > 0 {
        tracing::info!(
            count = migrated_count,
            "Catalog service description migration complete"
        );
    }

    // NyxID#778: the gcp-cloud-billing / gcp-bigquery-billing catalog entries
    // (and their gcp-billing / gcp-bigquery providers) were superseded by
    // the generic `api-google-cloud` + `google-cloud` OAuth flow. The seed
    // code for these was removed, but pre-existing rows from earlier
    // deployments remain in MongoDB -- both admin catalog rows AND user-
    // owned rows that were auto-provisioned from those catalog slugs.
    //
    // Auth method `gcp_service_account` is gone from the proxy, so any
    // surviving UserService row is dead: it cannot route a request and
    // its stored SA JSON (on the linked UserApiKey) is meaningless. Keep
    // them around and we leak credentials into perpetuity. Cascade-delete:
    //
    //   AgentServiceBinding ─┐
    //   UserService ─────────┤── (UserEndpoint / UserApiKey if not still
    //                             referenced by another remaining service)
    //   UserProviderToken ─┐
    //   UserProviderCreds ─┤── by provider_config_id IN <removed>
    //   ServiceProviderRequirement (by either side)
    //   DownstreamService / ProviderConfig
    //
    // Affected users re-add via `api-google-cloud --oauth ...`.
    cleanup_legacy_gcp_sa_data(db, &service_col, &provider_col, &req_col).await?;

    Ok(())
}

/// One-shot cascade cleanup for NyxID#778. See the call site above for the
/// dependency tree. After the first run on a deployment with legacy GCP
/// rows, every subsequent invocation is a no-op (every query returns empty).
async fn cleanup_legacy_gcp_sa_data(
    db: &mongodb::Database,
    service_col: &mongodb::Collection<DownstreamService>,
    provider_col: &mongodb::Collection<ProviderConfig>,
    req_col: &mongodb::Collection<ServiceProviderRequirement>,
) -> AppResult<()> {
    const REMOVED_SERVICE_SLUGS: &[&str] = &["gcp-cloud-billing", "gcp-bigquery-billing"];
    const REMOVED_PROVIDER_SLUGS: &[&str] = &["gcp-billing", "gcp-bigquery"];

    let service_ids: Vec<String> = service_col
        .find(doc! { "slug": { "$in": REMOVED_SERVICE_SLUGS } })
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();

    let provider_ids: Vec<String> = provider_col
        .find(doc! { "slug": { "$in": REMOVED_PROVIDER_SLUGS } })
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|p| p.id)
        .collect();

    // Cascade-delete user-owned rows tied to the removed catalog slugs.
    let user_service_col = db.collection::<UserService>(USER_SERVICES);
    let user_services: Vec<UserService> = user_service_col
        .find(doc! { "slug": { "$in": REMOVED_SERVICE_SLUGS } })
        .await?
        .try_collect()
        .await?;

    let user_service_ids: Vec<String> = user_services.iter().map(|s| s.id.clone()).collect();
    let touched_endpoint_ids: Vec<String> = user_services
        .iter()
        .map(|s| s.endpoint_id.clone())
        .collect();
    let touched_api_key_ids: Vec<String> = user_services
        .iter()
        .filter_map(|s| s.api_key_id.clone())
        .collect();

    let agent_bindings_removed = if user_service_ids.is_empty() {
        0
    } else {
        db.collection::<mongodb::bson::Document>(AGENT_SERVICE_BINDINGS)
            .delete_many(doc! { "user_service_id": { "$in": &user_service_ids } })
            .await?
            .deleted_count
    };

    let user_services_removed = if user_service_ids.is_empty() {
        0
    } else {
        user_service_col
            .delete_many(doc! { "_id": { "$in": &user_service_ids } })
            .await?
            .deleted_count
    };

    // Only delete endpoints / api keys that aren't still referenced by
    // some other UserService row. Matches the cascade pattern in
    // `unified_key_service::cleanup_orphan_endpoints_for_user`.
    let user_endpoints_removed =
        delete_unreferenced(db, USER_ENDPOINTS, "endpoint_id", &touched_endpoint_ids).await?;
    let user_api_keys_removed =
        delete_unreferenced(db, USER_API_KEYS, "api_key_id", &touched_api_key_ids).await?;

    let user_provider_tokens_removed = if provider_ids.is_empty() {
        0
    } else {
        db.collection::<mongodb::bson::Document>(USER_PROVIDER_TOKENS)
            .delete_many(doc! { "provider_config_id": { "$in": &provider_ids } })
            .await?
            .deleted_count
    };

    let user_provider_creds_removed = if provider_ids.is_empty() {
        0
    } else {
        db.collection::<mongodb::bson::Document>(USER_PROVIDER_CREDENTIALS)
            .delete_many(doc! { "provider_config_id": { "$in": &provider_ids } })
            .await?
            .deleted_count
    };

    let mut requirement_query = doc! {};
    if !service_ids.is_empty() {
        requirement_query.insert("service_id", doc! { "$in": &service_ids });
    }
    if !provider_ids.is_empty() {
        let provider_clause = doc! { "provider_config_id": { "$in": &provider_ids } };
        if requirement_query.is_empty() {
            requirement_query = provider_clause;
        } else {
            requirement_query = doc! { "$or": [requirement_query, provider_clause] };
        }
    }
    let requirements_removed = if requirement_query.is_empty() {
        0
    } else {
        req_col.delete_many(requirement_query).await?.deleted_count
    };

    let services_removed = if service_ids.is_empty() {
        0
    } else {
        service_col
            .delete_many(doc! { "_id": { "$in": &service_ids } })
            .await?
            .deleted_count
    };

    let providers_removed = if provider_ids.is_empty() {
        0
    } else {
        provider_col
            .delete_many(doc! { "_id": { "$in": &provider_ids } })
            .await?
            .deleted_count
    };

    let nothing_to_do = services_removed == 0
        && providers_removed == 0
        && user_services_removed == 0
        && agent_bindings_removed == 0
        && user_endpoints_removed == 0
        && user_api_keys_removed == 0
        && user_provider_tokens_removed == 0
        && user_provider_creds_removed == 0
        && requirements_removed == 0;
    if nothing_to_do {
        return Ok(());
    }

    tracing::info!(
        services_removed,
        providers_removed,
        requirements_removed,
        user_services_removed,
        agent_bindings_removed,
        user_endpoints_removed,
        user_api_keys_removed,
        user_provider_tokens_removed,
        user_provider_creds_removed,
        service_slugs = ?REMOVED_SERVICE_SLUGS,
        provider_slugs = ?REMOVED_PROVIDER_SLUGS,
        "NyxID#778: cascade-removed legacy GCP service-account data \
         (superseded by api-google-cloud + google-cloud OAuth)"
    );

    Ok(())
}

/// Delete rows from `collection` whose `_id` is in `candidate_ids` *and*
/// no surviving `user_services` row still references that id via the
/// `referencing_field` foreign key. Returns the number deleted.
async fn delete_unreferenced(
    db: &mongodb::Database,
    collection: &str,
    referencing_field: &str,
    candidate_ids: &[String],
) -> AppResult<u64> {
    if candidate_ids.is_empty() {
        return Ok(0);
    }
    let still_referenced: std::collections::HashSet<String> = db
        .collection::<mongodb::bson::Document>(USER_SERVICES)
        .distinct(
            referencing_field,
            doc! { referencing_field: { "$in": candidate_ids } },
        )
        .await?
        .into_iter()
        .filter_map(|b| b.as_str().map(str::to_string))
        .collect();
    let orphaned: Vec<&String> = candidate_ids
        .iter()
        .filter(|id| !still_referenced.contains(*id))
        .collect();
    if orphaned.is_empty() {
        return Ok(0);
    }
    Ok(db
        .collection::<mongodb::bson::Document>(collection)
        .delete_many(doc! { "_id": { "$in": &orphaned } })
        .await?
        .deleted_count)
}

/// Input for OAuth2 provider configuration fields.
pub struct OAuthProviderInput {
    pub authorization_url: String,
    pub token_url: String,
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
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
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub supports_pkce: bool,
}

/// Input for API key provider configuration fields.
pub struct ApiKeyProviderInput {
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
}

/// Input for Telegram Widget provider configuration fields.
pub struct TelegramWidgetProviderInput {
    pub bot_token: String,
    pub bot_username: String,
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
    pub credential_mode: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub extra_auth_params: Option<HashMap<String, String>>,
    pub device_code_format: Option<String>,
    pub client_id_param_name: Option<String>,
}

/// Create a new provider configuration. Admin only.
#[allow(clippy::too_many_arguments)]
pub async fn create_provider(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    name: &str,
    slug: &str,
    provider_type: &str,
    credential_mode: &str,
    token_endpoint_auth_method: &str,
    oauth_config: Option<OAuthProviderInput>,
    api_key_config: Option<ApiKeyProviderInput>,
    device_code_config: Option<DeviceCodeProviderInput>,
    telegram_widget_config: Option<TelegramWidgetProviderInput>,
    description: Option<&str>,
    icon_url: Option<&str>,
    documentation_url: Option<&str>,
    created_by: &str,
    extra_auth_params: Option<HashMap<String, String>>,
    device_code_format: Option<&str>,
    client_id_param_name: Option<&str>,
) -> AppResult<ProviderConfig> {
    let valid_types = ["oauth2", "api_key", "device_code", "telegram_widget"];
    if !valid_types.contains(&provider_type) {
        return Err(AppError::ValidationError(format!(
            "provider_type must be one of: {}",
            valid_types.join(", ")
        )));
    }
    let valid_modes = ["admin", "user", "both"];
    if !valid_modes.contains(&credential_mode) {
        return Err(AppError::ValidationError(format!(
            "credential_mode must be one of: {}",
            valid_modes.join(", ")
        )));
    }
    if provider_type == "telegram_widget" && credential_mode != "admin" {
        return Err(AppError::ValidationError(
            "telegram_widget providers only support credential_mode=admin".to_string(),
        ));
    }
    if provider_type == "api_key" && credential_mode != "admin" {
        return Err(AppError::ValidationError(
            "credential_mode only applies to oauth2/device_code providers; omit it or set \"admin\" for api_key providers".to_string(),
        ));
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
        let cid = match oauth.client_id.as_ref() {
            Some(value) => Some(encryption_keys.encrypt(value.as_bytes()).await?),
            None => None,
        };
        let csec = match oauth.client_secret.as_ref() {
            Some(value) => Some(encryption_keys.encrypt(value.as_bytes()).await?),
            None => None,
        };
        (cid, csec)
    } else if let Some(ref dc) = device_code_config {
        let cid = match dc.client_id.as_ref() {
            Some(value) => Some(encryption_keys.encrypt(value.as_bytes()).await?),
            None => None,
        };
        let csec = match dc.client_secret.as_ref() {
            Some(value) => Some(encryption_keys.encrypt(value.as_bytes()).await?),
            None => None,
        };
        (cid, csec)
    } else if let Some(ref tw) = telegram_widget_config {
        let bot_token = normalize_telegram_bot_token(&tw.bot_token)?;
        let csec = encryption_keys.encrypt(bot_token.as_bytes()).await?;
        (None, Some(csec))
    } else {
        (None, None)
    };
    let normalized_client_id_param_name = if let Some(ref tw) = telegram_widget_config {
        Some(normalize_telegram_bot_username(&tw.bot_username)?)
    } else {
        client_id_param_name.map(String::from)
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
            .or_else(|| {
                device_code_config
                    .as_ref()
                    .map(|d| d.authorization_url.clone())
            }),
        token_url: oauth_config
            .as_ref()
            .map(|o| o.token_url.clone())
            .or_else(|| device_code_config.as_ref().map(|d| d.token_url.clone())),
        revocation_url: oauth_config.as_ref().and_then(|o| o.revocation_url.clone()),
        default_scopes: oauth_config
            .as_ref()
            .and_then(|o| o.default_scopes.clone())
            .or_else(|| {
                device_code_config
                    .as_ref()
                    .and_then(|d| d.default_scopes.clone())
            }),
        client_id_encrypted: client_id_enc,
        client_secret_encrypted: client_secret_enc,
        supports_pkce: oauth_config.as_ref().is_some_and(|o| o.supports_pkce)
            || device_code_config.as_ref().is_some_and(|d| d.supports_pkce),
        device_code_url: device_code_config
            .as_ref()
            .map(|d| d.device_code_url.clone()),
        device_token_url: device_code_config
            .as_ref()
            .map(|d| d.device_token_url.clone()),
        device_verification_url: device_code_config
            .as_ref()
            .and_then(|d| d.device_verification_url.clone()),
        hosted_callback_url: device_code_config
            .as_ref()
            .and_then(|d| d.hosted_callback_url.clone()),
        api_key_instructions: api_key_config
            .as_ref()
            .and_then(|a| a.api_key_instructions.clone()),
        api_key_url: api_key_config.as_ref().and_then(|a| a.api_key_url.clone()),
        icon_url: icon_url.map(String::from),
        documentation_url: documentation_url.map(String::from),
        is_active: true,
        credential_mode: credential_mode.to_string(),
        token_endpoint_auth_method: token_endpoint_auth_method.to_string(),
        extra_auth_params,
        device_code_format: device_code_format.unwrap_or("rfc8628").to_string(),
        client_id_param_name: normalized_client_id_param_name,
        requires_gateway_url: false,
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
pub async fn get_provider(db: &mongodb::Database, provider_id: &str) -> AppResult<ProviderConfig> {
    db.collection::<ProviderConfig>(COLLECTION_NAME)
        .find_one(doc! { "_id": provider_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Provider not found".to_string()))
}

/// Get a single provider by slug.
#[allow(dead_code)]
pub async fn get_provider_by_slug(db: &mongodb::Database, slug: &str) -> AppResult<ProviderConfig> {
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
    encryption_keys: &EncryptionKeys,
    provider_id: &str,
    updates: ProviderUpdateInput,
) -> AppResult<ProviderConfig> {
    // Verify exists
    let existing = get_provider(db, provider_id).await?;
    if existing.provider_type == "telegram_widget"
        && let Some(ref mode) = updates.credential_mode
        && mode != "admin"
    {
        return Err(AppError::ValidationError(
            "telegram_widget providers only support credential_mode=admin".to_string(),
        ));
    }
    if existing.provider_type == "api_key"
        && let Some(ref mode) = updates.credential_mode
        && mode != "admin"
    {
        return Err(AppError::ValidationError(
            "credential_mode only applies to oauth2/device_code providers; omit it or set \"admin\" for api_key providers".to_string(),
        ));
    }

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
        let enc = encryption_keys.encrypt(cid.as_bytes()).await?;
        set_doc.insert(
            "client_id_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: enc,
            },
        );
    }
    if let Some(ref csec) = updates.client_secret {
        let secret_value = if existing.provider_type == "telegram_widget" {
            normalize_telegram_bot_token(csec)?
        } else {
            csec.clone()
        };
        let enc = encryption_keys.encrypt(secret_value.as_bytes()).await?;
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
    if let Some(ref mode) = updates.credential_mode {
        let valid_modes = ["admin", "user", "both"];
        if !valid_modes.contains(&mode.as_str()) {
            return Err(AppError::ValidationError(format!(
                "credential_mode must be one of: {}",
                valid_modes.join(", ")
            )));
        }
        set_doc.insert("credential_mode", mode.as_str());
    }

    if let Some(ref method) = updates.token_endpoint_auth_method {
        let valid_methods: [&str; 2] = ["client_secret_post", "client_secret_basic"];
        if !valid_methods.contains(&method.as_str()) {
            return Err(AppError::ValidationError(format!(
                "token_endpoint_auth_method must be one of: {}",
                valid_methods.join(", ")
            )));
        }
        set_doc.insert("token_endpoint_auth_method", method.as_str());
    }

    if let Some(ref params) = updates.extra_auth_params {
        set_doc.insert(
            "extra_auth_params",
            bson::to_bson(params).unwrap_or(bson::Bson::Null),
        );
    }
    if let Some(ref format) = updates.device_code_format {
        let valid_formats = ["rfc8628", "openai"];
        if !valid_formats.contains(&format.as_str()) {
            return Err(AppError::ValidationError(
                "device_code_format must be 'rfc8628' or 'openai'".to_string(),
            ));
        }
        set_doc.insert("device_code_format", format.as_str());
    }
    if let Some(ref name) = updates.client_id_param_name {
        let value = if existing.provider_type == "telegram_widget" {
            normalize_telegram_bot_username(name)?
        } else {
            name.clone()
        };
        set_doc.insert("client_id_param_name", value);
    }

    use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

    let updated = db
        .collection::<ProviderConfig>(COLLECTION_NAME)
        .find_one_and_update(doc! { "_id": provider_id }, doc! { "$set": set_doc })
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

pub(crate) fn normalize_telegram_bot_username(raw: &str) -> AppResult<String> {
    let normalized = raw.trim().trim_start_matches('@');
    if normalized.is_empty() {
        return Err(AppError::ValidationError(
            "Telegram bot username must not be empty".to_string(),
        ));
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(AppError::ValidationError(
            "Telegram bot username must not contain whitespace".to_string(),
        ));
    }
    if !(5..=32).contains(&normalized.len()) {
        return Err(AppError::ValidationError(
            "Telegram bot username must be 5-32 characters, start with a letter, use only letters, digits, or underscores, and end in 'bot'".to_string(),
        ));
    }

    let mut chars = normalized.chars();
    let Some(first) = chars.next() else {
        return Err(AppError::ValidationError(
            "Telegram bot username must not be empty".to_string(),
        ));
    };

    if !first.is_ascii_alphabetic()
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        || !normalized.to_ascii_lowercase().ends_with("bot")
    {
        return Err(AppError::ValidationError(
            "Telegram bot username must be 5-32 characters, start with a letter, use only letters, digits, or underscores, and end in 'bot'".to_string(),
        ));
    }

    Ok(normalized.to_string())
}

fn normalize_telegram_bot_token(raw: &str) -> AppResult<String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Err(AppError::ValidationError(
            "Telegram bot token must not be empty".to_string(),
        ));
    }
    if normalized.chars().any(char::is_whitespace) {
        return Err(AppError::ValidationError(
            "Telegram bot token must not contain whitespace".to_string(),
        ));
    }

    Ok(normalized.to_string())
}

/// Soft-delete a provider. Also revokes all user tokens for this provider.
pub async fn delete_provider(db: &mongodb::Database, provider_id: &str) -> AppResult<()> {
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

    // Delete all per-user OAuth credentials for this provider
    db.collection::<mongodb::bson::Document>(USER_PROVIDER_CREDENTIALS)
        .delete_many(doc! { "provider_config_id": provider_id })
        .await?;

    tracing::info!(provider_id = %provider_id, "Provider deactivated, user tokens revoked, and user credentials deleted");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ANTHROPIC_DEFAULT_HEADERS, DEFAULT_SERVICE_SEEDS, SeededHeader,
        normalize_telegram_bot_token, normalize_telegram_bot_username, reconcile_seeded_headers,
        seed_capability_override,
    };
    use crate::errors::AppError;
    use crate::models::default_request_header::DefaultRequestHeader;

    fn hdr(name: &str, value: &str) -> DefaultRequestHeader {
        DefaultRequestHeader {
            name: name.to_string(),
            value: value.to_string(),
            overridable: false,
            sensitive: false,
        }
    }

    #[test]
    fn telegram_bot_seed_uses_direct_path_auth() {
        let seed = DEFAULT_SERVICE_SEEDS
            .iter()
            .find(|seed| seed.service_slug == "api-telegram-bot")
            .expect("api-telegram-bot seed should exist");

        assert_eq!(seed.service_auth_method, Some("path"));
        assert_eq!(seed.service_auth_key_name, Some("bot"));
    }

    #[test]
    fn openclaw_seed_advertises_websocket_and_streaming() {
        let (caps, streaming) = seed_capability_override("llm-openclaw")
            .expect("llm-openclaw should have a capability override");

        assert!(
            caps.supports_websocket,
            "llm-openclaw must advertise WebSocket passthrough (NyxID#160)"
        );
        assert!(
            caps.supports_streaming,
            "llm-openclaw must advertise streaming so clients pick the right transport"
        );
        assert!(
            streaming,
            "streaming_supported must be true so discovery stops returning false"
        );
    }

    #[test]
    fn seed_capability_override_aevatar_has_streaming_flags() {
        let (caps, streaming) =
            seed_capability_override("aevatar").expect("aevatar should have a capability override");

        assert!(caps.supports_proxy_read);
        assert!(caps.supports_proxy_write);
        assert!(!caps.supports_proxy_binary_upload);
        assert!(caps.supports_direct_downstream_auth);
        assert!(!caps.supports_authoring_via_nyx);
        assert!(!caps.supports_websocket);
        assert!(caps.supports_streaming);
        assert!(streaming);
    }

    #[test]
    fn seed_capability_override_returns_none_for_unknown_slug() {
        assert!(seed_capability_override("llm-openai").is_none());
        assert!(seed_capability_override("api-github").is_none());
    }

    #[test]
    fn normalize_telegram_bot_username_trims_whitespace_and_at_prefix() {
        let normalized = normalize_telegram_bot_username("  @NyxIdBot  ")
            .expect("username should be normalized");

        assert_eq!(normalized, "NyxIdBot");
    }

    #[test]
    fn normalize_telegram_bot_username_rejects_whitespace() {
        let err = normalize_telegram_bot_username("Nyx Id Bot")
            .expect_err("whitespace should be rejected");

        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message == "Telegram bot username must not contain whitespace"
        ));
    }

    #[test]
    fn normalize_telegram_bot_username_rejects_invalid_format() {
        let too_long = format!("{}bot", "a".repeat(30));
        for username in ["abc", "123bot", "not-a-bot", "bot", too_long.as_str()] {
            let err = normalize_telegram_bot_username(username)
                .expect_err("invalid bot username should be rejected");

            assert!(matches!(
                err,
                AppError::ValidationError(message)
                    if message == "Telegram bot username must be 5-32 characters, start with a letter, use only letters, digits, or underscores, and end in 'bot'"
            ));
        }
    }

    #[test]
    fn normalize_telegram_bot_token_trims_surrounding_whitespace() {
        let normalized = normalize_telegram_bot_token(" 123456:ABC-DEF123 \n")
            .expect("token should be normalized");

        assert_eq!(normalized, "123456:ABC-DEF123");
    }

    #[test]
    fn normalize_telegram_bot_token_rejects_blank_or_embedded_whitespace() {
        let blank =
            normalize_telegram_bot_token("   ").expect_err("blank token should be rejected");
        assert!(matches!(
            blank,
            AppError::ValidationError(message)
                if message == "Telegram bot token must not be empty"
        ));

        let spaced = normalize_telegram_bot_token("123456:ABC DEF")
            .expect_err("tokens with embedded whitespace should be rejected");
        assert!(matches!(
            spaced,
            AppError::ValidationError(message)
                if message == "Telegram bot token must not contain whitespace"
        ));
    }

    #[test]
    fn anthropic_seed_carries_required_headers() {
        let seed = DEFAULT_SERVICE_SEEDS
            .iter()
            .find(|s| s.service_slug == "llm-anthropic")
            .expect("llm-anthropic seed should exist");

        let headers = seed
            .default_request_headers
            .expect("llm-anthropic must carry seeded default headers");
        let names: Vec<&str> = headers.iter().map(|h| h.name).collect();
        assert!(
            names.contains(&"anthropic-version"),
            "anthropic-version must be seeded (required on every Anthropic API call); got {names:?}"
        );
        assert!(
            names.contains(&"content-type"),
            "content-type must be seeded; got {names:?}"
        );

        let version = headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("anthropic-version"))
            .expect("anthropic-version present");
        assert!(
            version.overridable,
            "anthropic-version must be overridable so SDK-supplied versions win"
        );
    }

    #[test]
    fn reconcile_inserts_full_list_when_stored_is_missing() {
        let merged = reconcile_seeded_headers(None, ANTHROPIC_DEFAULT_HEADERS)
            .expect("missing stored list must be filled from seed");
        assert_eq!(merged.len(), ANTHROPIC_DEFAULT_HEADERS.len());
    }

    #[test]
    fn reconcile_inserts_full_list_when_stored_is_empty() {
        let stored: Vec<DefaultRequestHeader> = Vec::new();
        let merged = reconcile_seeded_headers(Some(&stored), ANTHROPIC_DEFAULT_HEADERS)
            .expect("empty stored list counts as 'backfill required'");
        assert_eq!(merged.len(), ANTHROPIC_DEFAULT_HEADERS.len());
    }

    #[test]
    fn reconcile_preserves_admin_values_and_adds_only_missing_seeded() {
        // Admin has kept `anthropic-version` and customized its value; we
        // must not overwrite that. Admin also added an unrelated header.
        // `content-type` is missing, so only that one should be appended.
        let stored = vec![
            DefaultRequestHeader {
                name: "Anthropic-Version".to_string(),
                value: "2024-10-22".to_string(),
                overridable: true,
                sensitive: false,
            },
            hdr("x-admin-custom", "keep"),
        ];
        let merged = reconcile_seeded_headers(Some(&stored), ANTHROPIC_DEFAULT_HEADERS)
            .expect("missing content-type should trigger a write");

        assert_eq!(merged.len(), 3);
        let version = merged
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("anthropic-version"))
            .expect("admin's anthropic-version survives");
        assert_eq!(
            version.value, "2024-10-22",
            "admin-edited value must not be clobbered by the seed default"
        );
        assert_eq!(version.name, "Anthropic-Version", "admin casing preserved");
        assert!(
            merged
                .iter()
                .any(|h| h.name == "x-admin-custom" && h.value == "keep"),
            "unrelated admin headers must be preserved"
        );
        assert!(
            merged
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("content-type")),
            "missing seeded header must be appended"
        );
    }

    #[test]
    fn reconcile_returns_none_when_everything_already_present() {
        let stored = vec![
            hdr("anthropic-version", "2023-06-01"),
            hdr("content-type", "application/json"),
        ];
        assert!(
            reconcile_seeded_headers(Some(&stored), ANTHROPIC_DEFAULT_HEADERS).is_none(),
            "no-op reconcile must not trigger a DB write"
        );
    }

    #[test]
    fn reconcile_case_insensitive_name_match_prevents_duplicates() {
        // Admin has stored the header under a different casing; we must
        // not append a duplicate just because the bytes differ.
        let stored = vec![hdr("ANTHROPIC-VERSION", "2025-01-01")];
        let merged = reconcile_seeded_headers(Some(&stored), ANTHROPIC_DEFAULT_HEADERS)
            .expect("content-type still needs to be added");

        let version_matches: Vec<&DefaultRequestHeader> = merged
            .iter()
            .filter(|h| h.name.eq_ignore_ascii_case("anthropic-version"))
            .collect();
        assert_eq!(
            version_matches.len(),
            1,
            "name match must be case-insensitive; no duplicate rows"
        );
        assert_eq!(version_matches[0].value, "2025-01-01");
    }

    #[test]
    fn reconcile_empty_seed_list_is_noop() {
        let stored = vec![hdr("x-custom", "v")];
        let seeded: &[SeededHeader] = &[];
        assert!(reconcile_seeded_headers(Some(&stored), seeded).is_none());
        assert!(reconcile_seeded_headers(None, seeded).is_none());
    }

    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::provider_config::{COLLECTION_NAME, ProviderConfig};
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
    };
    use crate::test_utils::{connect_test_database, test_encryption_keys};
    use chrono::Utc;
    use mongodb::bson::doc;
    use uuid::Uuid;

    fn make_test_provider(slug: &str, provider_type: &str) -> ProviderConfig {
        let now = Utc::now();
        ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: slug.to_string(),
            name: format!("Test {slug}"),
            description: None,
            provider_type: provider_type.to_string(),
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
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    async fn seed_default_catalog(db_name: &str) -> Option<mongodb::Database> {
        let Some(db) = connect_test_database(db_name).await else {
            eprintln!("skipping: no MongoDB");
            return None;
        };
        let enc = test_encryption_keys();
        super::seed_default_providers(&db, &enc)
            .await
            .expect("seed providers");
        super::seed_default_services(&db, &enc)
            .await
            .expect("seed services");
        Some(db)
    }

    #[tokio::test]
    async fn seed_default_services_populates_catalog_from_seed_table() {
        let Some(db) = seed_default_catalog("prov_seed_catalog").await else {
            return;
        };
        let provider_col = db.collection::<ProviderConfig>(COLLECTION_NAME);
        let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);

        for seed in DEFAULT_SERVICE_SEEDS {
            let provider = provider_col
                .find_one(doc! { "slug": seed.provider_slug })
                .await
                .expect("query provider")
                .unwrap_or_else(|| panic!("provider '{}' should be seeded", seed.provider_slug));
            assert!(
                provider.is_active,
                "provider '{}' active",
                seed.provider_slug
            );

            let service = service_col
                .find_one(doc! { "slug": seed.service_slug })
                .await
                .expect("query service")
                .unwrap_or_else(|| panic!("service '{}' should be seeded", seed.service_slug));
            assert_eq!(
                service.provider_config_id.as_deref(),
                Some(provider.id.as_str())
            );
            assert_eq!(service.base_url, seed.base_url);
            assert_eq!(
                service.auth_method,
                seed.service_auth_method.unwrap_or("none")
            );
            assert_eq!(
                service.auth_key_name,
                seed.service_auth_key_name.unwrap_or("")
            );
        }

        let service_count = service_col
            .count_documents(doc! {})
            .await
            .expect("count seeded services");
        assert_eq!(service_count, DEFAULT_SERVICE_SEEDS.len() as u64);
    }

    #[tokio::test]
    async fn seed_default_services_seeds_anthropic_default_headers() {
        let Some(db) = seed_default_catalog("prov_seed_anthropic_headers").await else {
            return;
        };
        let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        let anthropic = service_col
            .find_one(doc! { "slug": "llm-anthropic" })
            .await
            .expect("query anthropic")
            .expect("anthropic service");
        let headers = anthropic
            .default_request_headers
            .expect("anthropic seeded headers");
        assert_eq!(headers.len(), ANTHROPIC_DEFAULT_HEADERS.len());
        assert!(headers.iter().any(|h| h.name == "anthropic-version"));
        assert!(headers.iter().any(|h| h.name == "content-type"));
    }

    #[tokio::test]
    async fn seed_default_services_seeds_openclaw_capabilities() {
        let Some(db) = seed_default_catalog("prov_seed_openclaw_caps").await else {
            return;
        };
        let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        let openclaw = service_col
            .find_one(doc! { "slug": "llm-openclaw" })
            .await
            .expect("query openclaw")
            .expect("openclaw service");
        assert!(openclaw.streaming_supported);
        let caps = openclaw.capabilities.expect("openclaw capabilities");
        assert!(caps.supports_websocket);
        assert!(caps.supports_streaming);
    }

    #[tokio::test]
    async fn seed_default_services_seeds_aevatar_contract() {
        let Some(db) = seed_default_catalog("prov_seed_aevatar_contract").await else {
            return;
        };
        let provider_col = db.collection::<ProviderConfig>(COLLECTION_NAME);
        let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        let req_col = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);

        let provider = provider_col
            .find_one(doc! { "slug": "aevatar" })
            .await
            .expect("query aevatar provider")
            .expect("aevatar provider");
        assert_eq!(provider.provider_type, "api_key");
        assert!(provider.requires_gateway_url);

        let service = service_col
            .find_one(doc! { "slug": "aevatar" })
            .await
            .expect("query aevatar service")
            .expect("aevatar service");
        assert_eq!(service.name, "aevatar");
        assert_eq!(service.base_url, "https://aevatar-gateway.invalid");
        assert_eq!(
            service.provider_config_id.as_deref(),
            Some(provider.id.as_str())
        );
        assert_eq!(service.auth_method, "none");
        assert_eq!(service.auth_key_name, "");
        assert!(service.streaming_supported);
        let caps = service.capabilities.expect("aevatar capabilities");
        assert!(caps.supports_proxy_read);
        assert!(caps.supports_proxy_write);
        assert!(caps.supports_streaming);
        assert!(!caps.supports_websocket);
        assert_eq!(service.identity_propagation_mode, "jwt");
        assert!(service.identity_include_user_id);
        assert!(service.identity_include_email);
        assert!(!service.identity_include_name);
        assert!(!service.forward_access_token);
        assert!(!service.inject_delegation_token);
        assert_eq!(service.delegation_token_scope, "llm:proxy");
        assert!(
            service
                .required_permissions
                .as_ref()
                .is_some_and(|permissions| permissions.contains(&"nyxid:identity-jwt".to_string()))
        );

        let requirement = req_col
            .find_one(doc! { "service_id": &service.id })
            .await
            .expect("query aevatar requirement")
            .expect("aevatar provider requirement");
        assert_eq!(requirement.provider_config_id, provider.id);
        assert_eq!(requirement.injection_method, "bearer");
        assert_eq!(requirement.injection_key.as_deref(), Some("Authorization"));
    }

    #[tokio::test]
    async fn seed_default_services_preserves_active_non_system_aevatar() {
        let Some(db) = connect_test_database("prov_seed_aevatar_custom_preserved").await else {
            return;
        };
        let enc = test_encryption_keys();
        let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        let req_col = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);

        let mut custom_service = crate::models::downstream_service::test_helpers::dummy_service();
        custom_service.id = Uuid::new_v4().to_string();
        custom_service.name = "Custom aevatar".to_string();
        custom_service.slug = "aevatar".to_string();
        custom_service.description = Some("operator-owned description".to_string());
        custom_service.base_url = "https://operator-aevatar.example.com".to_string();
        custom_service.created_by = "operator-admin".to_string();
        custom_service.provider_config_id = None;
        custom_service.auth_method = "bearer".to_string();
        custom_service.auth_key_name = "Authorization".to_string();
        custom_service.streaming_supported = false;
        custom_service.capabilities = None;
        custom_service.identity_propagation_mode = "none".to_string();
        custom_service.identity_include_user_id = false;
        custom_service.identity_include_email = false;
        custom_service.delegation_token_scope = "proxy:*".to_string();
        let custom_service_id = custom_service.id.clone();

        service_col
            .insert_one(custom_service)
            .await
            .expect("insert custom aevatar service");

        super::seed_default_providers(&db, &enc)
            .await
            .expect("seed providers");
        super::seed_default_services(&db, &enc)
            .await
            .expect("seed services");

        assert_eq!(
            service_col
                .count_documents(doc! { "slug": "aevatar", "is_active": true })
                .await
                .expect("count active aevatar services"),
            1,
            "seeding must not insert a second active aevatar row over an operator-owned slug"
        );

        let preserved = service_col
            .find_one(doc! { "_id": &custom_service_id })
            .await
            .expect("query preserved custom service")
            .expect("custom aevatar service should remain");
        assert_eq!(preserved.name, "Custom aevatar");
        assert_eq!(
            preserved.description.as_deref(),
            Some("operator-owned description")
        );
        assert_eq!(preserved.base_url, "https://operator-aevatar.example.com");
        assert_eq!(preserved.created_by, "operator-admin");
        assert_eq!(preserved.provider_config_id, None);
        assert_eq!(preserved.auth_method, "bearer");
        assert_eq!(preserved.auth_key_name, "Authorization");
        assert!(!preserved.streaming_supported);
        assert!(preserved.capabilities.is_none());
        assert_eq!(preserved.identity_propagation_mode, "none");
        assert!(!preserved.identity_include_user_id);
        assert!(!preserved.identity_include_email);
        assert_eq!(preserved.delegation_token_scope, "proxy:*");

        assert_eq!(
            req_col
                .count_documents(doc! { "service_id": &custom_service_id })
                .await
                .expect("count custom aevatar requirements"),
            0,
            "SPR backfill must not attach the seeded provider requirement to an operator-owned aevatar row"
        );
    }

    #[tokio::test]
    async fn seed_default_services_seeds_telegram_bot_as_direct_path_auth() {
        let Some(db) = seed_default_catalog("prov_seed_telegram_bot").await else {
            return;
        };
        let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        let req_col = db.collection::<ServiceProviderRequirement>(REQUIREMENTS);
        let telegram = service_col
            .find_one(doc! { "slug": "api-telegram-bot" })
            .await
            .expect("query telegram bot")
            .expect("telegram bot service");
        assert_eq!(telegram.auth_method, "path");
        assert_eq!(telegram.auth_key_name, "bot");
        assert_eq!(
            req_col
                .count_documents(doc! { "service_id": &telegram.id })
                .await
                .expect("count telegram requirements"),
            0
        );
    }

    #[tokio::test]
    async fn seed_default_services_seeds_lark_bot_token_exchange() {
        let Some(db) = seed_default_catalog("prov_seed_lark_bot").await else {
            return;
        };
        let service_col = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
        let lark_bot = service_col
            .find_one(doc! { "slug": "api-lark-bot" })
            .await
            .expect("query lark bot")
            .expect("lark bot service");
        assert_eq!(lark_bot.auth_method, "token_exchange");
        assert_eq!(lark_bot.auth_key_name, "");
        assert!(lark_bot.token_exchange_config.is_some());
    }

    #[tokio::test]
    async fn create_provider_validates_invalid_provider_type() {
        let Some(db) = connect_test_database("provider_ext_invalid_type").await else {
            return;
        };
        let enc = test_encryption_keys();
        let err = super::create_provider(
            &db,
            &enc,
            "Bad",
            "bad-type",
            "invalid_type",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .expect_err("invalid type");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_provider_validates_invalid_credential_mode() {
        let Some(db) = connect_test_database("provider_ext_invalid_mode").await else {
            return;
        };
        let enc = test_encryption_keys();
        let err = super::create_provider(
            &db,
            &enc,
            "Bad",
            "bad-mode",
            "api_key",
            "invalid_mode",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .expect_err("invalid mode");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_provider_rejects_non_admin_mode_for_api_key() {
        let Some(db) = connect_test_database("provider_ext_apikey_mode").await else {
            return;
        };
        let enc = test_encryption_keys();
        let err = super::create_provider(
            &db,
            &enc,
            "Bad",
            "apikey-user",
            "api_key",
            "user",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .expect_err("api_key must be admin");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_provider_rejects_non_admin_mode_for_telegram_widget() {
        let Some(db) = connect_test_database("provider_ext_tg_mode").await else {
            return;
        };
        let enc = test_encryption_keys();
        let err = super::create_provider(
            &db,
            &enc,
            "TG",
            "tg-user",
            "telegram_widget",
            "user",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .expect_err("telegram must be admin");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_provider_rejects_duplicate_slug() {
        let Some(db) = connect_test_database("provider_ext_dup_slug").await else {
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("dup-{}", Uuid::new_v4());
        super::create_provider(
            &db,
            &enc,
            "First",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let err = super::create_provider(
            &db,
            &enc,
            "Second",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .expect_err("duplicate slug");
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn create_and_get_provider_round_trips() {
        let Some(db) = connect_test_database("provider_ext_create_get").await else {
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("rt-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Round Trip",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            Some("A test provider"),
            None,
            Some("https://example.com/docs"),
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(created.slug, slug);
        assert_eq!(created.description.as_deref(), Some("A test provider"));
        assert!(created.is_active);

        let fetched = super::get_provider(&db, &created.id).await.unwrap();
        assert_eq!(fetched.slug, slug);

        let by_slug = super::get_provider_by_slug(&db, &slug).await.unwrap();
        assert_eq!(by_slug.id, created.id);
    }

    #[tokio::test]
    async fn list_providers_only_returns_active() {
        let Some(db) = connect_test_database("provider_ext_list").await else {
            return;
        };
        let enc = test_encryption_keys();
        let slug_active = format!("active-{}", Uuid::new_v4());
        let slug_inactive = format!("inactive-{}", Uuid::new_v4());

        super::create_provider(
            &db,
            &enc,
            "Active",
            &slug_active,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let mut inactive = make_test_provider(&slug_inactive, "api_key");
        inactive.is_active = false;
        db.collection::<ProviderConfig>(COLLECTION_NAME)
            .insert_one(&inactive)
            .await
            .unwrap();

        let all = super::list_providers(&db).await.unwrap();
        let slugs: Vec<&str> = all.iter().map(|p| p.slug.as_str()).collect();
        assert!(slugs.contains(&slug_active.as_str()));
        assert!(!slugs.contains(&slug_inactive.as_str()));
    }

    #[tokio::test]
    async fn get_provider_returns_not_found_for_missing_id() {
        let Some(db) = connect_test_database("provider_ext_notfound").await else {
            return;
        };
        let err = super::get_provider(&db, "nonexistent-id")
            .await
            .expect_err("should not find");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_provider_by_slug_returns_not_found_for_missing_slug() {
        let Some(db) = connect_test_database("provider_ext_slug_nf").await else {
            return;
        };
        let err = super::get_provider_by_slug(&db, "no-such-slug")
            .await
            .expect_err("should not find");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_provider_deactivates_and_revokes_tokens() {
        let Some(db) = connect_test_database("provider_ext_delete").await else {
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("del-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Delete Me",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        super::delete_provider(&db, &created.id).await.unwrap();
        let after = db
            .collection::<ProviderConfig>(COLLECTION_NAME)
            .find_one(doc! { "_id": &created.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!after.is_active);
    }

    #[tokio::test]
    async fn delete_provider_returns_not_found_for_missing() {
        let Some(db) = connect_test_database("provider_ext_del_nf").await else {
            return;
        };
        let err = super::delete_provider(&db, "nonexistent")
            .await
            .expect_err("should not find");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn create_provider_api_key_stores_correctly() {
        let Some(db) = connect_test_database("provider_ext_ak_create").await else {
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("ak-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "API Key Provider",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            Some(super::ApiKeyProviderInput {
                api_key_instructions: Some("Get key from console".to_string()),
                api_key_url: Some("https://example.com/keys".to_string()),
            }),
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            created.api_key_instructions.as_deref(),
            Some("Get key from console")
        );
        assert_eq!(
            created.api_key_url.as_deref(),
            Some("https://example.com/keys")
        );
        assert_eq!(created.provider_type, "api_key");
    }

    #[tokio::test]
    async fn create_provider_oauth2_stores_urls_and_scopes() {
        let Some(db) = connect_test_database("provider_ext_oauth_create").await else {
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("oauth-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "OAuth",
            &slug,
            "oauth2",
            "user",
            "client_secret_post",
            Some(super::OAuthProviderInput {
                authorization_url: "https://auth.example.com/authorize".to_string(),
                token_url: "https://auth.example.com/token".to_string(),
                revocation_url: Some("https://auth.example.com/revoke".to_string()),
                default_scopes: Some(vec!["openid".to_string(), "profile".to_string()]),
                client_id: Some("test-client".to_string()),
                client_secret: Some("test-secret".to_string()),
                supports_pkce: true,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            created.authorization_url.as_deref(),
            Some("https://auth.example.com/authorize")
        );
        assert!(created.supports_pkce);
        assert!(created.client_id_encrypted.is_some());
        assert!(created.client_secret_encrypted.is_some());
        assert_eq!(created.credential_mode, "user");
    }

    #[test]
    fn normalize_telegram_bot_token_rejects_tab_whitespace() {
        let err = normalize_telegram_bot_token("123:ABC\tDEF").expect_err("tab should be rejected");
        assert!(err.to_string().contains("whitespace"));
    }

    #[test]
    fn normalize_telegram_bot_username_accepts_valid_five_char() {
        let result = normalize_telegram_bot_username("AaBot").expect("5-char valid bot username");
        assert_eq!(result, "AaBot");
    }

    #[test]
    fn normalize_telegram_bot_username_rejects_empty() {
        let err = normalize_telegram_bot_username("").expect_err("empty should be rejected");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn is_lark_family_slug_matches_known_slugs() {
        assert!(super::is_lark_family_slug("api-lark-bot"));
        assert!(super::is_lark_family_slug("api-feishu-bot"));
        assert!(!super::is_lark_family_slug("api-slack-bot"));
        assert!(!super::is_lark_family_slug("api-lark"));
    }

    #[test]
    fn default_service_seeds_have_unique_slugs() {
        let slugs: Vec<&str> = DEFAULT_SERVICE_SEEDS
            .iter()
            .map(|s| s.service_slug)
            .collect();
        let mut unique = slugs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(slugs.len(), unique.len());
    }

    // ── is_lark_family_slug additional coverage ─────────────────────

    #[test]
    fn is_lark_family_slug_case_sensitive() {
        assert!(!super::is_lark_family_slug("API-LARK-BOT"));
        assert!(!super::is_lark_family_slug("Api-Lark-Bot"));
    }

    #[test]
    fn is_lark_family_slug_empty_and_partial() {
        assert!(!super::is_lark_family_slug(""));
        assert!(!super::is_lark_family_slug("api-lark-bo"));
        assert!(!super::is_lark_family_slug("api-feishu-bott"));
    }

    // ── seeded_header_to_model tests ────────────────────────────────

    #[test]
    fn seeded_header_to_model_copies_all_fields() {
        let seed = SeededHeader {
            name: "x-custom",
            value: "test-value",
            overridable: true,
            sensitive: true,
        };
        let model = super::seeded_header_to_model(&seed);
        assert_eq!(model.name, "x-custom");
        assert_eq!(model.value, "test-value");
        assert!(model.overridable);
        assert!(model.sensitive);
    }

    #[test]
    fn seeded_header_to_model_non_overridable_non_sensitive() {
        let seed = SeededHeader {
            name: "content-type",
            value: "application/json",
            overridable: false,
            sensitive: false,
        };
        let model = super::seeded_header_to_model(&seed);
        assert_eq!(model.name, "content-type");
        assert_eq!(model.value, "application/json");
        assert!(!model.overridable);
        assert!(!model.sensitive);
    }

    // ── normalize_telegram_bot_token additional coverage ─────────────

    #[test]
    fn normalize_telegram_bot_token_accepts_minimal_valid_token() {
        assert_eq!(normalize_telegram_bot_token("abc").unwrap(), "abc");
    }

    #[test]
    fn normalize_telegram_bot_token_rejects_embedded_tab() {
        let err =
            normalize_telegram_bot_token("abc\tdef").expect_err("embedded tab should be rejected");
        assert!(matches!(err, AppError::ValidationError(m) if m.contains("whitespace")));
    }

    #[test]
    fn normalize_telegram_bot_token_rejects_embedded_newline() {
        let err = normalize_telegram_bot_token("abc\ndef")
            .expect_err("embedded newline should be rejected");
        assert!(matches!(err, AppError::ValidationError(m) if m.contains("whitespace")));
    }

    // ── seed_capability_override additional coverage ─────────────────

    #[test]
    fn seed_capability_override_openclaw_has_correct_flags() {
        let (caps, streaming) = seed_capability_override("llm-openclaw").unwrap();
        assert!(caps.supports_proxy_read);
        assert!(caps.supports_proxy_write);
        assert!(!caps.supports_proxy_binary_upload);
        assert!(caps.supports_direct_downstream_auth);
        assert!(!caps.supports_authoring_via_nyx);
        assert!(streaming);
    }

    #[test]
    fn seed_capability_override_returns_none_for_empty_slug() {
        assert!(seed_capability_override("").is_none());
    }

    // ── reconcile_seeded_headers additional coverage ─────────────────

    #[test]
    fn reconcile_seeded_headers_single_seeded_entry() {
        let single_seed: &[SeededHeader] = &[SeededHeader {
            name: "x-only",
            value: "val",
            overridable: false,
            sensitive: false,
        }];
        let stored = vec![hdr("x-other", "v")];
        let merged = reconcile_seeded_headers(Some(&stored), single_seed)
            .expect("missing seeded entry should trigger merge");
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|h| h.name == "x-only"));
        assert!(merged.iter().any(|h| h.name == "x-other"));
    }

    #[test]
    fn reconcile_seeded_headers_all_present_returns_none() {
        let single_seed: &[SeededHeader] = &[SeededHeader {
            name: "x-present",
            value: "val",
            overridable: false,
            sensitive: false,
        }];
        let stored = vec![hdr("x-present", "custom-val")];
        assert!(reconcile_seeded_headers(Some(&stored), single_seed).is_none());
    }

    // ── DEFAULT_SERVICE_SEEDS content checks ────────────────────────

    #[test]
    fn default_service_seeds_all_have_provider_slugs() {
        for seed in DEFAULT_SERVICE_SEEDS {
            assert!(
                !seed.provider_slug.is_empty(),
                "seed '{}' has empty provider_slug",
                seed.service_slug
            );
        }
    }

    #[test]
    fn default_service_seeds_all_have_base_urls() {
        for seed in DEFAULT_SERVICE_SEEDS {
            assert!(
                !seed.base_url.is_empty(),
                "seed '{}' has empty base_url",
                seed.service_slug
            );
        }
    }

    // ── update_provider integration tests ──────────────────────────

    #[tokio::test]
    async fn update_provider_name_and_description() {
        let Some(db) = connect_test_database("prov_svc_upd_name").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-name-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Original",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            Some("old desc"),
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let updated = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: Some("Renamed".to_string()),
                description: Some("new desc".to_string()),
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.description.as_deref(), Some("new desc"));
        assert_eq!(updated.slug, slug, "slug must not change");
        assert!(updated.updated_at > created.updated_at);
    }

    #[tokio::test]
    async fn update_provider_deactivates() {
        let Some(db) = connect_test_database("prov_svc_upd_deact").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("deact-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Active",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(created.is_active);

        let updated = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: Some(false),
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();
        assert!(!updated.is_active);
    }

    #[tokio::test]
    async fn update_provider_returns_not_found_for_missing() {
        let Some(db) = connect_test_database("prov_svc_upd_nf").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let err = super::update_provider(
            &db,
            &enc,
            "nonexistent-id",
            super::ProviderUpdateInput {
                name: Some("X".to_string()),
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .expect_err("should not find");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_provider_rejects_invalid_credential_mode() {
        let Some(db) = connect_test_database("prov_svc_upd_bad_mode").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-bm-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Mode",
            &slug,
            "oauth2",
            "admin",
            "client_secret_post",
            Some(super::OAuthProviderInput {
                authorization_url: "https://auth.example.com/auth".to_string(),
                token_url: "https://auth.example.com/token".to_string(),
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: false,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: Some("invalid_mode".to_string()),
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .expect_err("invalid credential_mode");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn update_provider_rejects_non_admin_mode_for_api_key_type() {
        let Some(db) = connect_test_database("prov_svc_upd_ak_mode").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-ak-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "AK",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: Some("user".to_string()),
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .expect_err("api_key must remain admin");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn update_provider_rejects_non_admin_mode_for_telegram_widget_type() {
        let Some(db) = connect_test_database("prov_svc_upd_tw_mode").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        // Insert a telegram_widget provider directly since create_provider needs
        // a TelegramWidgetProviderInput with bot credentials.
        let mut tw = make_test_provider(&format!("tw-{}", Uuid::new_v4()), "telegram_widget");
        tw.credential_mode = "admin".to_string();
        db.collection::<ProviderConfig>(COLLECTION_NAME)
            .insert_one(&tw)
            .await
            .unwrap();

        let err = super::update_provider(
            &db,
            &enc,
            &tw.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: Some("user".to_string()),
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .expect_err("telegram_widget must be admin");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn update_provider_rejects_invalid_token_endpoint_auth_method() {
        let Some(db) = connect_test_database("prov_svc_upd_bad_auth").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-ba-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "AuthM",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: Some("bearer_magic".to_string()),
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .expect_err("invalid auth method");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn update_provider_rejects_invalid_device_code_format() {
        let Some(db) = connect_test_database("prov_svc_upd_bad_dcf").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-dcf-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "DCF",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: Some("magic".to_string()),
                client_id_param_name: None,
            },
        )
        .await
        .expect_err("invalid device_code_format");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn update_provider_encrypts_client_id_and_secret() {
        let Some(db) = connect_test_database("prov_svc_upd_enc").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-enc-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Enc",
            &slug,
            "oauth2",
            "admin",
            "client_secret_post",
            Some(super::OAuthProviderInput {
                authorization_url: "https://auth.example.com/auth".to_string(),
                token_url: "https://auth.example.com/token".to_string(),
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: false,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(created.client_id_encrypted.is_none());

        let updated = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: Some("my-client-id".to_string()),
                client_secret: Some("my-secret".to_string()),
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();
        assert!(
            updated.client_id_encrypted.is_some(),
            "client_id should be encrypted after update"
        );
        assert!(
            updated.client_secret_encrypted.is_some(),
            "client_secret should be encrypted after update"
        );
    }

    #[tokio::test]
    async fn update_provider_updates_oauth_urls() {
        let Some(db) = connect_test_database("prov_svc_upd_urls").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-urls-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "URLs",
            &slug,
            "oauth2",
            "user",
            "client_secret_post",
            Some(super::OAuthProviderInput {
                authorization_url: "https://old.example.com/auth".to_string(),
                token_url: "https://old.example.com/token".to_string(),
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: false,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let updated = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: Some("https://new.example.com/auth".to_string()),
                token_url: Some("https://new.example.com/token".to_string()),
                revocation_url: Some("https://new.example.com/revoke".to_string()),
                default_scopes: Some(vec!["email".to_string()]),
                client_id: None,
                client_secret: None,
                supports_pkce: Some(true),
                device_code_url: Some("https://new.example.com/device".to_string()),
                device_token_url: Some("https://new.example.com/devtoken".to_string()),
                device_verification_url: Some("https://new.example.com/verify".to_string()),
                hosted_callback_url: Some("https://new.example.com/callback".to_string()),
                api_key_instructions: Some("new instructions".to_string()),
                api_key_url: Some("https://new.example.com/keys".to_string()),
                icon_url: Some("https://new.example.com/icon.png".to_string()),
                documentation_url: Some("https://new.example.com/docs".to_string()),
                credential_mode: Some("both".to_string()),
                token_endpoint_auth_method: Some("client_secret_basic".to_string()),
                extra_auth_params: Some(std::collections::HashMap::from([(
                    "prompt".to_string(),
                    "consent".to_string(),
                )])),
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            updated.authorization_url.as_deref(),
            Some("https://new.example.com/auth")
        );
        assert_eq!(
            updated.token_url.as_deref(),
            Some("https://new.example.com/token")
        );
        assert_eq!(
            updated.revocation_url.as_deref(),
            Some("https://new.example.com/revoke")
        );
        assert_eq!(updated.default_scopes, Some(vec!["email".to_string()]));
        assert!(updated.supports_pkce);
        assert_eq!(
            updated.device_code_url.as_deref(),
            Some("https://new.example.com/device")
        );
        assert_eq!(
            updated.device_token_url.as_deref(),
            Some("https://new.example.com/devtoken")
        );
        assert_eq!(
            updated.device_verification_url.as_deref(),
            Some("https://new.example.com/verify")
        );
        assert_eq!(
            updated.hosted_callback_url.as_deref(),
            Some("https://new.example.com/callback")
        );
        assert_eq!(
            updated.api_key_instructions.as_deref(),
            Some("new instructions")
        );
        assert_eq!(
            updated.api_key_url.as_deref(),
            Some("https://new.example.com/keys")
        );
        assert_eq!(
            updated.icon_url.as_deref(),
            Some("https://new.example.com/icon.png")
        );
        assert_eq!(
            updated.documentation_url.as_deref(),
            Some("https://new.example.com/docs")
        );
        assert_eq!(updated.credential_mode, "both");
        assert_eq!(updated.token_endpoint_auth_method, "client_secret_basic");
        assert!(updated.extra_auth_params.is_some());
    }

    // ── create_provider with device_code config ────────────────────

    #[tokio::test]
    async fn create_provider_device_code_stores_fields() {
        let Some(db) = connect_test_database("prov_svc_dc_create").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("dc-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "DevCode",
            &slug,
            "device_code",
            "admin",
            "client_secret_post",
            None,
            None,
            Some(super::DeviceCodeProviderInput {
                authorization_url: "https://auth.example.com/authorize".to_string(),
                token_url: "https://auth.example.com/token".to_string(),
                device_code_url: "https://auth.example.com/device/code".to_string(),
                device_token_url: "https://auth.example.com/device/token".to_string(),
                device_verification_url: Some("https://auth.example.com/device/verify".to_string()),
                hosted_callback_url: Some("https://auth.example.com/callback".to_string()),
                default_scopes: Some(vec!["openid".to_string()]),
                client_id: Some("dc-client-id".to_string()),
                client_secret: Some("dc-secret".to_string()),
                supports_pkce: true,
            }),
            None,
            Some("A device code provider"),
            None,
            None,
            "test",
            None,
            Some("openai"),
            None,
        )
        .await
        .unwrap();

        assert_eq!(created.provider_type, "device_code");
        assert_eq!(
            created.authorization_url.as_deref(),
            Some("https://auth.example.com/authorize")
        );
        assert_eq!(
            created.token_url.as_deref(),
            Some("https://auth.example.com/token")
        );
        assert_eq!(
            created.device_code_url.as_deref(),
            Some("https://auth.example.com/device/code")
        );
        assert_eq!(
            created.device_token_url.as_deref(),
            Some("https://auth.example.com/device/token")
        );
        assert_eq!(
            created.device_verification_url.as_deref(),
            Some("https://auth.example.com/device/verify")
        );
        assert_eq!(
            created.hosted_callback_url.as_deref(),
            Some("https://auth.example.com/callback")
        );
        assert_eq!(created.default_scopes, Some(vec!["openid".to_string()]));
        assert!(created.client_id_encrypted.is_some());
        assert!(created.client_secret_encrypted.is_some());
        assert!(created.supports_pkce);
        assert_eq!(created.device_code_format, "openai");
    }

    // ── create_provider with telegram_widget config ────────────────

    #[tokio::test]
    async fn create_provider_telegram_widget_encrypts_bot_token() {
        let Some(db) = connect_test_database("prov_svc_tw_create").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("tw-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Telegram",
            &slug,
            "telegram_widget",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            Some(super::TelegramWidgetProviderInput {
                bot_token: "123456789:ABCdefGHI_jklMNOpqrSTUvwx".to_string(),
                bot_username: "@NyxTestBot".to_string(),
            }),
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(created.provider_type, "telegram_widget");
        assert!(
            created.client_secret_encrypted.is_some(),
            "bot token should be encrypted"
        );
        assert_eq!(
            created.client_id_param_name.as_deref(),
            Some("NyxTestBot"),
            "bot username should be normalized (@ stripped)"
        );
    }

    // ── create_provider with extra_auth_params ─────────────────────

    #[tokio::test]
    async fn create_provider_stores_extra_auth_params() {
        let Some(db) = connect_test_database("prov_svc_extra_params").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("eap-{}", Uuid::new_v4());
        let extra = std::collections::HashMap::from([
            ("access_type".to_string(), "offline".to_string()),
            ("prompt".to_string(), "consent".to_string()),
        ]);
        let created = super::create_provider(
            &db,
            &enc,
            "Extra",
            &slug,
            "oauth2",
            "user",
            "client_secret_post",
            Some(super::OAuthProviderInput {
                authorization_url: "https://auth.example.com/auth".to_string(),
                token_url: "https://auth.example.com/token".to_string(),
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: false,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            Some(extra.clone()),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(created.extra_auth_params, Some(extra));
    }

    // ── create_provider with client_id_param_name ──────────────────

    #[tokio::test]
    async fn create_provider_stores_client_id_param_name() {
        let Some(db) = connect_test_database("prov_svc_cidpn").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("cidpn-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "TikTok-ish",
            &slug,
            "oauth2",
            "user",
            "client_secret_post",
            Some(super::OAuthProviderInput {
                authorization_url: "https://tiktok.example.com/auth".to_string(),
                token_url: "https://tiktok.example.com/token".to_string(),
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: false,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            Some("client_key"),
        )
        .await
        .unwrap();
        assert_eq!(created.client_id_param_name.as_deref(), Some("client_key"));
    }

    // ── seed_default_providers idempotency ─────────────────────────

    #[tokio::test]
    async fn seed_default_providers_is_idempotent() {
        let Some(db) = connect_test_database("prov_svc_seed_idem").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        // First call seeds providers
        super::seed_default_providers(&db, &enc).await.unwrap();
        let count_1 = db
            .collection::<ProviderConfig>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
        assert!(count_1 > 0, "should have seeded at least one provider");

        // Second call should be a no-op
        super::seed_default_providers(&db, &enc).await.unwrap();
        let count_2 = db
            .collection::<ProviderConfig>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
        assert_eq!(count_1, count_2, "idempotent: count must not change");
    }

    #[tokio::test]
    async fn seed_default_providers_creates_known_slugs() {
        let Some(db) = connect_test_database("prov_svc_seed_slugs").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        super::seed_default_providers(&db, &enc).await.unwrap();

        let expected_slugs = [
            "openai",
            "openai-codex",
            "anthropic",
            "google-ai",
            "mistral",
            "cohere",
            "deepseek",
            "twitter",
            "google",
            "github",
            "github-pat",
        ];
        for slug in expected_slugs {
            let found = db
                .collection::<ProviderConfig>(COLLECTION_NAME)
                .find_one(doc! { "slug": slug })
                .await
                .unwrap();
            assert!(found.is_some(), "expected seeded provider slug: {slug}");
        }
    }

    // ── seed_default_services idempotency ──────────────────────────

    #[tokio::test]
    async fn seed_default_services_is_idempotent() {
        let Some(db) = connect_test_database("prov_svc_svc_idem").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        // Seed providers first (services depend on them)
        super::seed_default_providers(&db, &enc).await.unwrap();

        super::seed_default_services(&db, &enc).await.unwrap();
        let service_col = db.collection::<super::DownstreamService>(super::DOWNSTREAM_SERVICES);
        let req_col = db.collection::<super::ServiceProviderRequirement>(super::REQUIREMENTS);
        let count_1 = service_col.count_documents(doc! {}).await.unwrap();
        let req_count_1 = req_col.count_documents(doc! {}).await.unwrap();
        assert!(count_1 > 0, "should have seeded at least one service");
        assert!(
            req_count_1 > 0,
            "should have seeded at least one provider requirement"
        );

        super::seed_default_services(&db, &enc).await.unwrap();
        let count_2 = service_col.count_documents(doc! {}).await.unwrap();
        let req_count_2 = req_col.count_documents(doc! {}).await.unwrap();
        assert_eq!(count_1, count_2, "idempotent: count must not change");
        assert_eq!(
            req_count_1, req_count_2,
            "idempotent: requirement count must not change"
        );
    }

    #[tokio::test]
    async fn seed_default_services_creates_known_slugs() {
        let Some(db) = connect_test_database("prov_svc_svc_slugs").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        super::seed_default_providers(&db, &enc).await.unwrap();
        super::seed_default_services(&db, &enc).await.unwrap();

        let service_col = db.collection::<super::DownstreamService>(super::DOWNSTREAM_SERVICES);
        let expected = ["llm-openai", "llm-anthropic", "llm-google-ai", "api-github"];
        for slug in expected {
            let found = service_col.find_one(doc! { "slug": slug }).await.unwrap();
            assert!(found.is_some(), "expected seeded service slug: {slug}");
        }
    }

    // ── delete_provider cascade tests ──────────────────────────────

    #[tokio::test]
    async fn delete_provider_revokes_user_tokens_and_deletes_credentials() {
        let Some(db) = connect_test_database("prov_svc_del_cascade").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("del-casc-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "Cascade",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Insert a mock user_provider_token and user_provider_credentials
        let token_col = db.collection::<mongodb::bson::Document>("user_provider_tokens");
        token_col
            .insert_one(doc! {
                "_id": Uuid::new_v4().to_string(),
                "user_id": "test-user",
                "provider_config_id": &created.id,
                "status": "active",
            })
            .await
            .unwrap();

        let cred_col = db.collection::<mongodb::bson::Document>("user_provider_credentials");
        cred_col
            .insert_one(doc! {
                "_id": Uuid::new_v4().to_string(),
                "user_id": "test-user",
                "provider_config_id": &created.id,
            })
            .await
            .unwrap();

        super::delete_provider(&db, &created.id).await.unwrap();

        // Check provider is deactivated
        let provider = db
            .collection::<ProviderConfig>(COLLECTION_NAME)
            .find_one(doc! { "_id": &created.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!provider.is_active);

        // Check token was revoked
        let token = token_col
            .find_one(doc! { "provider_config_id": &created.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(token.get_str("status").unwrap(), "revoked");

        // Check credentials were deleted
        let cred_count = cred_col
            .count_documents(doc! { "provider_config_id": &created.id })
            .await
            .unwrap();
        assert_eq!(cred_count, 0, "credentials should be deleted");
    }

    // ── list_providers sorting ─────────────────────────────────────

    #[tokio::test]
    async fn list_providers_returns_sorted_by_name() {
        let Some(db) = connect_test_database("prov_svc_list_sort").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();

        // Create providers with names that sort in a known order
        for name in ["Zebra", "Apple", "Mango"] {
            let slug = format!("sort-{}-{}", name.to_lowercase(), Uuid::new_v4());
            super::create_provider(
                &db,
                &enc,
                name,
                &slug,
                "api_key",
                "admin",
                "client_secret_post",
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                "test",
                None,
                None,
                None,
            )
            .await
            .unwrap();
        }

        let all = super::list_providers(&db).await.unwrap();
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        let mut sorted_names = names.clone();
        sorted_names.sort();
        assert_eq!(
            names, sorted_names,
            "list_providers must return sorted by name"
        );
    }

    // ── update_provider with extra_auth_params ─────────────────────

    #[tokio::test]
    async fn update_provider_sets_extra_auth_params() {
        let Some(db) = connect_test_database("prov_svc_upd_eap").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("upd-eap-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "EAP",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(created.extra_auth_params.is_none());

        let params = std::collections::HashMap::from([("nonce".to_string(), "abc".to_string())]);
        let updated = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: None,
                extra_auth_params: Some(params.clone()),
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.extra_auth_params, Some(params));
    }

    // ── update_provider valid credential_mode transitions ──────────

    #[tokio::test]
    async fn update_provider_allows_valid_credential_mode_changes() {
        let Some(db) = connect_test_database("prov_svc_upd_valid_cm").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("vcm-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "CM",
            &slug,
            "oauth2",
            "admin",
            "client_secret_post",
            Some(super::OAuthProviderInput {
                authorization_url: "https://auth.example.com/auth".to_string(),
                token_url: "https://auth.example.com/token".to_string(),
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: false,
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(created.credential_mode, "admin");

        // admin -> user
        let updated = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: Some("user".to_string()),
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.credential_mode, "user");

        // user -> both
        let updated2 = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: Some("both".to_string()),
                token_endpoint_auth_method: None,
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated2.credential_mode, "both");
    }

    // ── update_provider valid device_code_format ───────────────────

    #[tokio::test]
    async fn update_provider_accepts_valid_device_code_formats() {
        let Some(db) = connect_test_database("prov_svc_upd_dcf_ok").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("dcf-ok-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "DCF",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(created.device_code_format, "rfc8628");

        for format in ["openai", "rfc8628"] {
            let updated = super::update_provider(
                &db,
                &enc,
                &created.id,
                super::ProviderUpdateInput {
                    name: None,
                    description: None,
                    is_active: None,
                    authorization_url: None,
                    token_url: None,
                    revocation_url: None,
                    default_scopes: None,
                    client_id: None,
                    client_secret: None,
                    supports_pkce: None,
                    device_code_url: None,
                    device_token_url: None,
                    device_verification_url: None,
                    hosted_callback_url: None,
                    api_key_instructions: None,
                    api_key_url: None,
                    icon_url: None,
                    documentation_url: None,
                    credential_mode: None,
                    token_endpoint_auth_method: None,
                    extra_auth_params: None,
                    device_code_format: Some(format.to_string()),
                    client_id_param_name: None,
                },
            )
            .await
            .unwrap();
            assert_eq!(updated.device_code_format, format);
        }
    }

    // ── normalize_telegram_bot_username edge cases ──────────────────

    #[test]
    fn normalize_telegram_bot_username_rejects_at_only() {
        let err = normalize_telegram_bot_username("@")
            .expect_err("@ only should be rejected (empty after trim)");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn normalize_telegram_bot_username_accepts_max_length() {
        // 32 chars: start with letter, end with bot
        let username = format!("A{}bot", "a".repeat(28));
        assert_eq!(username.len(), 32);
        let result = normalize_telegram_bot_username(&username).unwrap();
        assert_eq!(result, username);
    }

    #[test]
    fn normalize_telegram_bot_username_accepts_underscores() {
        let result = normalize_telegram_bot_username("my_cool_bot").unwrap();
        assert_eq!(result, "my_cool_bot");
    }

    #[test]
    fn normalize_telegram_bot_username_rejects_hyphen() {
        let err =
            normalize_telegram_bot_username("my-cool-bot").expect_err("hyphens should be rejected");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    // ── make_test_provider helper checks ───────────────────────────

    #[test]
    fn make_test_provider_has_expected_defaults() {
        let p = make_test_provider("test-slug", "api_key");
        assert_eq!(p.slug, "test-slug");
        assert_eq!(p.provider_type, "api_key");
        assert!(p.is_active);
        assert_eq!(p.credential_mode, "admin");
        assert_eq!(p.created_by, "system");
    }

    // ── seed_default_services creates requirements ─────────────────

    #[tokio::test]
    async fn seed_default_services_creates_provider_requirements() {
        let Some(db) = connect_test_database("prov_svc_svc_reqs").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        super::seed_default_providers(&db, &enc).await.unwrap();
        super::seed_default_services(&db, &enc).await.unwrap();

        let req_col = db.collection::<super::ServiceProviderRequirement>(super::REQUIREMENTS);
        let count = req_col.count_documents(doc! {}).await.unwrap();
        assert!(count > 0, "should have created at least one requirement");
        let delegated_seed_count = DEFAULT_SERVICE_SEEDS
            .iter()
            .filter(|seed| seed.service_auth_method.is_none())
            .count() as u64;
        assert_eq!(count, delegated_seed_count);

        // Verify a specific requirement (openai -> llm-openai)
        let service_col = db.collection::<super::DownstreamService>(super::DOWNSTREAM_SERVICES);
        for seed in DEFAULT_SERVICE_SEEDS {
            let service = service_col
                .find_one(doc! { "slug": seed.service_slug })
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("service '{}' should be seeded", seed.service_slug));
            let req_count = req_col
                .count_documents(doc! { "service_id": &service.id })
                .await
                .unwrap();
            if seed.service_auth_method.is_some() {
                assert_eq!(
                    req_count, 0,
                    "direct-auth service '{}' must not get a provider requirement",
                    seed.service_slug
                );
            } else {
                assert_eq!(
                    req_count, 1,
                    "delegated service '{}' must get exactly one provider requirement",
                    seed.service_slug
                );
            }
        }
        if let Some(openai_svc) = service_col
            .find_one(doc! { "slug": "llm-openai" })
            .await
            .unwrap()
        {
            let req = req_col
                .find_one(doc! { "service_id": &openai_svc.id })
                .await
                .unwrap();
            assert!(
                req.is_some(),
                "llm-openai should have a provider requirement"
            );
            let req = req.unwrap();
            assert_eq!(req.injection_method, "bearer");
        }
    }

    // ── update_provider valid token_endpoint_auth_method ───────────

    #[tokio::test]
    async fn update_provider_accepts_valid_token_auth_methods() {
        let Some(db) = connect_test_database("prov_svc_upd_tam").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let slug = format!("tam-{}", Uuid::new_v4());
        let created = super::create_provider(
            &db,
            &enc,
            "TAM",
            &slug,
            "api_key",
            "admin",
            "client_secret_post",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(created.token_endpoint_auth_method, "client_secret_post");

        let updated = super::update_provider(
            &db,
            &enc,
            &created.id,
            super::ProviderUpdateInput {
                name: None,
                description: None,
                is_active: None,
                authorization_url: None,
                token_url: None,
                revocation_url: None,
                default_scopes: None,
                client_id: None,
                client_secret: None,
                supports_pkce: None,
                device_code_url: None,
                device_token_url: None,
                device_verification_url: None,
                hosted_callback_url: None,
                api_key_instructions: None,
                api_key_url: None,
                icon_url: None,
                documentation_url: None,
                credential_mode: None,
                token_endpoint_auth_method: Some("client_secret_basic".to_string()),
                extra_auth_params: None,
                device_code_format: None,
                client_id_param_name: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.token_endpoint_auth_method, "client_secret_basic");
    }

    // ── Direct-auth seeds skip provider requirements ───────────────

    #[test]
    fn direct_auth_seeds_have_service_auth_method() {
        let direct_auth_slugs = [
            "api-github-pat",
            "api-telegram-bot",
            "api-discord-bot",
            "api-lark-bot",
            "api-feishu-bot",
            "aws-cost-explorer",
        ];
        for slug in direct_auth_slugs {
            let seed = DEFAULT_SERVICE_SEEDS
                .iter()
                .find(|s| s.service_slug == slug);
            if let Some(seed) = seed {
                assert!(
                    seed.service_auth_method.is_some(),
                    "direct-auth seed '{slug}' must have a service_auth_method"
                );
            }
        }
    }
}
