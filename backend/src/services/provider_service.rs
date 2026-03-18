use std::collections::HashMap;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::provider_config::{COLLECTION_NAME, ProviderConfig};
use crate::models::service_provider_requirement::{
    COLLECTION_NAME as REQUIREMENTS, ServiceProviderRequirement,
};
use crate::models::user_provider_credentials::COLLECTION_NAME as USER_PROVIDER_CREDENTIALS;
use crate::models::user_provider_token::COLLECTION_NAME as USER_PROVIDER_TOKENS;

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
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "github", "Seeded default provider: GitHub");
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
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        };
        collection.insert_one(&provider).await?;
        tracing::info!(slug = "reddit", "Seeded default provider: Reddit");
        seeded_count += 1;
    }

    if seeded_count > 0 {
        tracing::info!(count = seeded_count, "Default provider seeding complete");
    }

    Ok(())
}

struct DefaultServiceSeed {
    provider_slug: &'static str,
    service_slug: &'static str,
    service_name: &'static str,
    base_url: &'static str,
    injection_method: &'static str,
    injection_key: &'static str,
}

const DEFAULT_SERVICE_SEEDS: &[DefaultServiceSeed] = &[
    DefaultServiceSeed {
        provider_slug: "openai",
        service_slug: "llm-openai",
        service_name: "OpenAI API",
        base_url: "https://api.openai.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "openai-codex",
        service_slug: "llm-openai-codex",
        service_name: "OpenAI Codex API",
        base_url: "https://chatgpt.com/backend-api/codex",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "anthropic",
        service_slug: "llm-anthropic",
        service_name: "Anthropic API",
        base_url: "https://api.anthropic.com/v1",
        injection_method: "header",
        injection_key: "x-api-key",
    },
    DefaultServiceSeed {
        provider_slug: "google-ai",
        service_slug: "llm-google-ai",
        service_name: "Google AI API",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        injection_method: "query",
        injection_key: "key",
    },
    DefaultServiceSeed {
        provider_slug: "mistral",
        service_slug: "llm-mistral",
        service_name: "Mistral AI API",
        base_url: "https://api.mistral.ai/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "cohere",
        service_slug: "llm-cohere",
        service_name: "Cohere API",
        base_url: "https://api.cohere.com/v2",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "deepseek",
        service_slug: "llm-deepseek",
        service_name: "DeepSeek API",
        base_url: "https://api.deepseek.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "twitter",
        service_slug: "api-twitter",
        service_name: "Twitter / X API",
        base_url: "https://api.x.com/2",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "google",
        service_slug: "api-google",
        service_name: "Google API",
        base_url: "https://www.googleapis.com",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "github",
        service_slug: "api-github",
        service_name: "GitHub API",
        base_url: "https://api.github.com",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "facebook",
        service_slug: "api-facebook",
        service_name: "Facebook Graph API",
        base_url: "https://graph.facebook.com/v21.0",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "discord",
        service_slug: "api-discord",
        service_name: "Discord API",
        base_url: "https://discord.com/api/v10",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "spotify",
        service_slug: "api-spotify",
        service_name: "Spotify API",
        base_url: "https://api.spotify.com/v1",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "slack",
        service_slug: "api-slack",
        service_name: "Slack API",
        base_url: "https://slack.com/api",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "microsoft",
        service_slug: "api-microsoft",
        service_name: "Microsoft Graph API",
        base_url: "https://graph.microsoft.com/v1.0",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "tiktok",
        service_slug: "api-tiktok",
        service_name: "TikTok API",
        base_url: "https://open.tiktokapis.com/v2",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "twitch",
        service_slug: "api-twitch",
        service_name: "Twitch API",
        base_url: "https://api.twitch.tv/helix",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
    DefaultServiceSeed {
        provider_slug: "reddit",
        service_slug: "api-reddit",
        service_name: "Reddit API",
        base_url: "https://oauth.reddit.com",
        injection_method: "bearer",
        injection_key: "Authorization",
    },
];

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

        // Create an empty encrypted credential (field is required)
        let empty_credential = encryption_keys.encrypt(b"").await?;

        let service_id = Uuid::new_v4().to_string();
        let is_llm_service = seed.service_slug.starts_with("llm-");
        let description = if is_llm_service {
            format!("{} proxied via NyxID LLM gateway", seed.service_name)
        } else {
            format!("{} proxied via NyxID proxy", seed.service_name)
        };
        let delegation_scope = if is_llm_service {
            "llm:proxy"
        } else {
            "proxy:*"
        };

        let service = DownstreamService {
            id: service_id.clone(),
            name: seed.service_name.to_string(),
            slug: seed.service_slug.to_string(),
            description: Some(description),
            base_url: seed.base_url.to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential_encrypted: empty_credential,
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
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
            inject_delegation_token: false,
            delegation_token_scope: delegation_scope.to_string(),
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

    Ok(())
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
    description: Option<&str>,
    icon_url: Option<&str>,
    documentation_url: Option<&str>,
    created_by: &str,
    extra_auth_params: Option<HashMap<String, String>>,
    device_code_format: Option<&str>,
    client_id_param_name: Option<&str>,
) -> AppResult<ProviderConfig> {
    let valid_types = ["oauth2", "api_key", "device_code"];
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
        client_id_param_name: client_id_param_name.map(String::from),
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
        let enc = encryption_keys.encrypt(csec.as_bytes()).await?;
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
        set_doc.insert("client_id_param_name", name.as_str());
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
