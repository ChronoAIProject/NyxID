use std::collections::HashMap;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use rand::Rng;
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::user_api_key::UserApiKey;
use crate::models::user_endpoint::UserEndpoint;
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service::UserService;
use crate::services::{
    node_service, ssh_service, user_api_key_service, user_endpoint_service, user_service_service,
};

const AUTO_PROVISION_SOURCE: &str = "auto_provision";

/// Generate a slug from a label: lowercase, replace non-alphanumeric with
/// hyphens, collapse runs, then append a 4-char random alphanumeric suffix.
fn generate_slug_from_label(label: &str) -> String {
    let base: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    let base = if base.is_empty() {
        "service".to_string()
    } else if base.len() > 80 {
        base[..80].to_string()
    } else {
        base
    };

    let mut rng = rand::thread_rng();
    let suffix: String = (0..4)
        .map(|_| {
            let idx: u8 = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect();

    format!("{base}-{suffix}")
}

/// Find a unique slug for a user by appending `-2`, `-3`, etc. if the base
/// slug already exists.
async fn resolve_unique_slug(
    db: &mongodb::Database,
    user_id: &str,
    base_slug: &str,
) -> AppResult<String> {
    if user_service_service::find_by_slug(db, user_id, base_slug)
        .await?
        .is_none()
    {
        return Ok(base_slug.to_string());
    }
    for n in 2..=100 {
        let candidate = format!("{base_slug}-{n}");
        if user_service_service::find_by_slug(db, user_id, &candidate)
            .await?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(AppError::Conflict(
        "Too many services with the same slug".to_string(),
    ))
}

fn auto_provision_source_id(user_id: &str, catalog_service_id: &str) -> String {
    format!("{user_id}:{catalog_service_id}")
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we)) =
        error.kind.as_ref()
    {
        return we.code == 11000;
    }
    false
}

fn identity_config_from_downstream_service(
    service: &DownstreamService,
) -> user_service_service::IdentityConfig {
    // When the catalog entry enables identity propagation but has all include
    // flags off (a common misconfiguration when seeding services), default to
    // including user_id and email so the mode is not a silent no-op.
    let has_active_mode = matches!(
        service.identity_propagation_mode.as_str(),
        "headers" | "jwt" | "both"
    );
    let all_flags_off = !service.identity_include_user_id
        && !service.identity_include_email
        && !service.identity_include_name;
    let apply_defaults = has_active_mode && all_flags_off;

    user_service_service::IdentityConfig {
        identity_propagation_mode: service.identity_propagation_mode.clone(),
        identity_include_user_id: service.identity_include_user_id || apply_defaults,
        identity_include_email: service.identity_include_email || apply_defaults,
        identity_include_name: service.identity_include_name || apply_defaults,
        identity_jwt_audience: service.identity_jwt_audience.clone(),
        forward_access_token: service.forward_access_token,
        inject_delegation_token: service.inject_delegation_token,
        delegation_token_scope: service.delegation_token_scope.clone(),
    }
}

/// SSH-specific parameters for custom SSH service creation.
pub struct SshCreateParams<'a> {
    pub host: &'a str,
    pub port: u16,
    pub certificate_auth: bool,
    pub principals: Vec<String>,
    pub certificate_ttl_minutes: u32,
}

/// Result of creating a key (all 3 records).
pub struct CreateKeyResult {
    pub endpoint: UserEndpoint,
    pub api_key: Option<UserApiKey>,
    pub service: UserService,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub ssh_ca_public_key: Option<String>,
    pub ssh_allowed_principals: Option<Vec<String>>,
    pub ssh_certificate_ttl_minutes: Option<u32>,
}

/// Combined view for GET /keys and GET /keys/:id.
pub struct KeyView {
    pub id: String,
    pub label: String,
    pub slug: String,
    pub endpoint_url: String,
    pub endpoint_id: String,
    pub api_key_id: Option<String>,
    pub credential_type: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub status: String,
    pub catalog_service_id: Option<String>,
    pub catalog_service_slug: Option<String>,
    pub catalog_service_name: Option<String>,
    pub node_id: Option<String>,
    pub node_priority: i32,
    pub service_type: String,
    pub is_active: bool,
    pub identity_propagation_mode: String,
    pub identity_include_user_id: bool,
    pub identity_include_email: bool,
    pub identity_include_name: bool,
    pub identity_jwt_audience: Option<String>,
    pub forward_access_token: bool,
    pub inject_delegation_token: bool,
    pub delegation_token_scope: String,
    pub auto_connected: bool,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    // SSH fields
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub ssh_ca_public_key: Option<String>,
    pub ssh_allowed_principals: Option<Vec<String>>,
    pub ssh_certificate_ttl_minutes: Option<u32>,
}

/// Validate that a catalog `token_exchange` service gets a properly
/// shaped credential from the caller. Older CLIs (pre-#220) and raw
/// HTTP clients that haven't learned the new credential format will
/// POST `{"credential": "<single_secret_string>"}` to `/api/v1/keys`.
/// Under the new `token_exchange` auth method, that single string can't
/// be parsed into the declared `{app_id, app_secret}` fields and every
/// subsequent proxy call would fail at request time with a misleading
/// error.
///
/// Fail loudly at registration time instead. The error message tells
/// the caller exactly how to fix it -- run `nyxid update` for a newer
/// CLI, or send the credential as a JSON object matching the declared
/// fields.
///
/// Returns `Ok(())` for auth methods other than `token_exchange` (the
/// helper short-circuits so it's cheap to call unconditionally).
pub(crate) fn validate_token_exchange_catalog_credential(
    svc: &DownstreamService,
    credential: &str,
) -> AppResult<()> {
    if svc.auth_method != "token_exchange" {
        return Ok(());
    }
    let exchange_config = svc.token_exchange_config.as_ref().ok_or_else(|| {
        AppError::Internal(format!(
            "Catalog service '{}' has auth_method=token_exchange but no \
             token_exchange_config. Contact an admin to fix the catalog entry.",
            svc.slug
        ))
    })?;
    if let Err(err) = crate::services::provider_token_exchange_service::parse_credential(
        credential,
        &exchange_config.credential_fields,
    ) {
        let field_list = exchange_config
            .credential_fields
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let first_field = exchange_config
            .credential_fields
            .first()
            .map(|f| f.name.as_str())
            .unwrap_or("field");
        return Err(AppError::BadRequest(format!(
            "'{}' requires the credential to be a JSON object with fields [{}]. \
             Older CLIs may only prompt for a single secret -- run `nyxid update` \
             to get the multi-field prompt. If you're calling /api/v1/keys directly, \
             send `credential` as a JSON string like '{{\"{}\":\"...\"}}'. \
             Underlying error: {err}",
            svc.slug, field_list, first_field
        )));
    }
    Ok(())
}

fn normalized_provider_credential_type(provider_type: &str) -> &'static str {
    match provider_type {
        "oauth2" | "device_code" => "oauth2",
        _ => "api_key",
    }
}

fn direct_credential_type_from_auth_method(auth_method: &str) -> Option<&'static str> {
    match auth_method {
        "none" => None,
        "bearer" => Some("bearer"),
        "basic" => Some("basic"),
        _ => Some("api_key"),
    }
}

fn direct_credential_type_for_service(
    api_key: &UserApiKey,
    service: &UserService,
    provider: Option<&ProviderConfig>,
) -> Option<&'static str> {
    if service.service_type == "ssh" || api_key.credential_type == "ssh_certificate" {
        return None;
    }

    if let Some(provider) = provider {
        return Some(normalized_provider_credential_type(&provider.provider_type));
    }

    match api_key.credential_type.as_str() {
        "oauth2" => Some("oauth2"),
        "bearer" => Some("bearer"),
        "basic" => Some("basic"),
        "node_managed" => direct_credential_type_from_auth_method(&service.auth_method),
        _ => Some("api_key"),
    }
}

async fn find_existing_provider_token(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: &str,
) -> AppResult<Option<UserProviderToken>> {
    db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "status": { "$in": ["active", "expired", "refresh_failed"] },
        })
        .await
        .map_err(Into::into)
}

/// POST /api/v1/keys -- auto-provision endpoint + api_key + service from catalog or custom.
#[allow(clippy::too_many_arguments)]
pub async fn create_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service_slug: Option<&str>,
    endpoint_url: Option<&str>,
    credential: &str,
    label: &str,
    slug_override: Option<&str>,
    auth_method: Option<&str>,
    auth_key_name: Option<&str>,
    node_id: Option<&str>,
    ssh_params: Option<SshCreateParams<'_>>,
    identity: Option<user_service_service::IdentityConfig>,
) -> AppResult<CreateKeyResult> {
    let node_id = node_id.filter(|nid| !nid.is_empty());

    if let Some(slug) = service_slug {
        // -- Catalog path --
        use crate::models::service_provider_requirement::{
            COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
        };

        let svc = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "slug": slug, "is_active": true })
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Catalog service '{slug}' not found")))?;

        let is_ssh = svc.service_type == "ssh";
        let provider = if let Some(ref pid) = svc.provider_config_id {
            db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
                .find_one(doc! { "_id": pid })
                .await?
        } else {
            None
        };
        let provider_type = provider.as_ref().map(|p| p.provider_type.as_str());
        let provider_requirement = db
            .collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
            .find_one(doc! { "service_id": &svc.id })
            .await?;
        let existing_provider_token =
            if let Some(provider_config_id) = svc.provider_config_id.as_deref() {
                find_existing_provider_token(db, user_id, provider_config_id).await?
            } else {
                None
            };
        let is_truly_no_auth =
            !is_ssh && svc.auth_method == "none" && provider_requirement.is_none();

        // SSH services must be node-routed
        if is_ssh && node_id.is_none() {
            return Err(AppError::BadRequest(
                "SSH services must be routed through a node agent".to_string(),
            ));
        }

        // Determine endpoint URL
        let ep_url = if let Some(url) = endpoint_url {
            url.to_string()
        } else if is_ssh {
            // SSH: derive from SshServiceConfig
            svc.ssh_config
                .as_ref()
                .map(|c| format!("ssh://{}:{}", c.host, c.port))
                .unwrap_or_default()
        } else if node_id.is_some() {
            // Node-routed: endpoint URL stored on node, not on NyxID
            String::new()
        } else if provider.as_ref().is_some_and(|p| p.requires_gateway_url) {
            return Err(AppError::BadRequest(
                "This service requires an endpoint URL".to_string(),
            ));
        } else {
            svc.base_url.clone()
        };

        // Determine credential type
        let node_managed_credential = node_id.is_some() && credential.is_empty();

        if node_id.is_some() && svc.provider_config_id.is_some() && !credential.is_empty() {
            return Err(AppError::BadRequest(
                "Node-routed provider services must be authorized on the node agent. Do not send the credential to NyxID."
                    .to_string(),
            ));
        }

        // Validate: credential required for direct routing (non-SSH, non-node-managed)
        let can_defer_direct_credential = existing_provider_token.is_some()
            || matches!(provider_type, Some("oauth2" | "device_code"));
        if credential.is_empty()
            && node_id.is_none()
            && !is_ssh
            && !can_defer_direct_credential
            && !is_truly_no_auth
        {
            return Err(AppError::BadRequest(
                "Credential is required for direct routing (or select a node)".to_string(),
            ));
        }

        // Validate: `token_exchange` services require the credential to be
        // a JSON object matching the catalog's declared credential fields.
        // See `validate_token_exchange_catalog_credential` for the full
        // rationale and the upgrade message old clients get.
        if !credential.is_empty() && !node_managed_credential {
            validate_token_exchange_catalog_credential(&svc, credential)?;
        }

        // Determine provider_config_id for the api key
        let provider_config_id = svc.provider_config_id.as_deref();

        // Create all three records
        let endpoint =
            user_endpoint_service::create_endpoint(db, user_id, &svc.name, &ep_url, Some(&svc.id))
                .await?;

        let api_key = if is_truly_no_auth {
            None
        } else if !node_managed_credential {
            let credential_type = if is_ssh {
                "ssh_certificate".to_string()
            } else if let Some(ref token) = existing_provider_token {
                match token.token_type.as_str() {
                    "oauth2" => "oauth2".to_string(),
                    _ => "api_key".to_string(),
                }
            } else if matches!(provider_type, Some("oauth2" | "device_code")) {
                "oauth2".to_string()
            } else if let Some(kind) = provider_type {
                normalized_provider_credential_type(kind).to_string()
            } else {
                svc.auth_type.as_deref().unwrap_or("api_key").to_string()
            };

            if let Some(ref provider_token) = existing_provider_token {
                Some(
                    user_api_key_service::create_api_key_from_provider_token(
                        db,
                        user_id,
                        label,
                        provider_config_id.expect("provider token implies provider config"),
                        provider_token,
                    )
                    .await?,
                )
            } else {
                let pending_oauth = matches!(provider_type, Some("oauth2" | "device_code"))
                    && credential.is_empty()
                    && node_id.is_none();
                Some(
                    user_api_key_service::create_api_key(
                        db,
                        encryption_keys,
                        user_id,
                        user_api_key_service::CreateApiKeyParams {
                            label,
                            credential_type: &credential_type,
                            credential,
                            access_token: (credential_type == "oauth2" && !credential.is_empty())
                                .then_some(credential),
                            refresh_token: None,
                            token_scopes: None,
                            expires_at: None,
                            provider_config_id,
                            status: if pending_oauth {
                                "pending_auth"
                            } else {
                                "active"
                            },
                            source: Some("user_created"),
                            source_id: None,
                        },
                    )
                    .await?,
                )
            }
        } else {
            Some(
                user_api_key_service::create_api_key(
                    db,
                    encryption_keys,
                    user_id,
                    user_api_key_service::CreateApiKeyParams {
                        label,
                        credential_type: "node_managed",
                        credential,
                        access_token: None,
                        refresh_token: None,
                        token_scopes: None,
                        expires_at: None,
                        provider_config_id,
                        status: "active",
                        source: Some("user_created"),
                        source_id: None,
                    },
                )
                .await?,
            )
        };

        // Auto-suffix slug if one already exists for this user (e.g. llm-openai -> llm-openai-2)
        let unique_slug = resolve_unique_slug(db, user_id, &svc.slug).await?;

        let catalog_identity =
            identity.unwrap_or_else(|| identity_config_from_downstream_service(&svc));

        let service = user_service_service::create_user_service(
            db,
            user_id,
            &unique_slug,
            &endpoint.id,
            api_key.as_ref().map(|k| k.id.as_str()),
            &svc.auth_method,
            &svc.auth_key_name,
            Some(&svc.id),
            node_id,
            0,
            &svc.service_type,
            None,
            None,
            &catalog_identity,
        )
        .await?;

        // Auto-sync NodeServiceBinding for the catalog service.
        node_service::sync_node_binding_for_user_service(db, user_id, Some(&svc.id), node_id, None)
            .await?;

        let (
            ssh_host,
            ssh_port,
            ssh_ca_public_key,
            ssh_allowed_principals,
            ssh_certificate_ttl_minutes,
        ) = if is_ssh {
            svc.ssh_config
                .as_ref()
                .map(|ssh| {
                    (
                        Some(ssh.host.clone()),
                        Some(ssh.port),
                        ssh.ca_public_key.clone(),
                        Some(ssh.allowed_principals.clone()),
                        Some(ssh.certificate_ttl_minutes),
                    )
                })
                .unwrap_or_default()
        } else {
            Default::default()
        };

        Ok(CreateKeyResult {
            endpoint,
            api_key,
            service,
            ssh_host,
            ssh_port,
            ssh_ca_public_key,
            ssh_allowed_principals,
            ssh_certificate_ttl_minutes,
        })
    } else if let Some(ssh) = ssh_params {
        // -- Custom SSH path --
        if node_id.is_none() {
            return Err(AppError::BadRequest(
                "SSH services must be routed through a node agent".to_string(),
            ));
        }

        let slug = match slug_override {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => generate_slug_from_label(label),
        };

        // Build SSH config (generates CA keypair)
        let ds_id = Uuid::new_v4().to_string();
        let built_ssh_config = ssh_service::build_ssh_config(
            encryption_keys,
            &ds_id,
            None,
            ssh_service::SshConfigInput {
                host: ssh.host,
                port: ssh.port,
                certificate_auth_enabled: ssh.certificate_auth,
                certificate_ttl_minutes: ssh.certificate_ttl_minutes,
                allowed_principals: &ssh.principals,
            },
        )
        .await?;

        let now = Utc::now();
        let base_url = ssh_service::target_base_url(&built_ssh_config.host, built_ssh_config.port);

        // Create DownstreamService with SSH config
        let ds = DownstreamService {
            id: ds_id.clone(),
            name: label.to_string(),
            slug: slug.to_string(),
            description: None,
            base_url: base_url.clone(),
            service_type: "ssh".to_string(),
            visibility: "private".to_string(),
            auth_method: "none".to_string(),
            auth_type: Some("ssh".to_string()),
            auth_key_name: String::new(),
            credential_encrypted: encryption_keys.encrypt(b"").await?,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: Some(built_ssh_config.clone()),
            oauth_client_id: None,
            service_category: "internal".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: user_id.to_string(),
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
            token_exchange_config: None,
            created_at: now,
            updated_at: now,
        };

        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&ds)
            .await?;

        let endpoint =
            user_endpoint_service::create_endpoint(db, user_id, label, &base_url, Some(&ds_id))
                .await?;

        let api_key = user_api_key_service::create_api_key(
            db,
            encryption_keys,
            user_id,
            user_api_key_service::CreateApiKeyParams {
                label,
                credential_type: "ssh_certificate",
                credential: "",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                status: "active",
                source: Some("user_created"),
                source_id: None,
            },
        )
        .await?;

        let unique_slug = resolve_unique_slug(db, user_id, &slug).await?;
        let service = user_service_service::create_user_service(
            db,
            user_id,
            &unique_slug,
            &endpoint.id,
            Some(&api_key.id),
            "none",
            "",
            Some(&ds_id),
            node_id,
            0,
            "ssh",
            None,
            None,
            &user_service_service::IdentityConfig::none(),
        )
        .await?;

        // Auto-sync NodeServiceBinding for the custom SSH service.
        node_service::sync_node_binding_for_user_service(db, user_id, Some(&ds_id), node_id, None)
            .await?;

        Ok(CreateKeyResult {
            endpoint,
            api_key: Some(api_key),
            service,
            ssh_host: Some(built_ssh_config.host),
            ssh_port: Some(built_ssh_config.port),
            ssh_ca_public_key: built_ssh_config.ca_public_key,
            ssh_allowed_principals: Some(built_ssh_config.allowed_principals),
            ssh_certificate_ttl_minutes: Some(built_ssh_config.certificate_ttl_minutes),
        })
    } else {
        // -- Custom HTTP path --
        let ep_url = endpoint_url.unwrap_or("");
        if ep_url.is_empty() && node_id.is_none() {
            return Err(AppError::BadRequest(
                "endpoint_url is required for custom endpoints without node routing".to_string(),
            ));
        }

        let slug = match slug_override {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => generate_slug_from_label(label),
        };
        let am = auth_method.unwrap_or("bearer");
        let akn = auth_key_name.unwrap_or("Authorization");
        let is_no_auth = am == "none";

        // Validate: credential required for direct routing unless no-auth
        if credential.is_empty() && node_id.is_none() && !is_no_auth {
            return Err(AppError::BadRequest(
                "Credential is required for direct routing (or select a node)".to_string(),
            ));
        }

        let endpoint =
            user_endpoint_service::create_endpoint(db, user_id, label, ep_url, None).await?;

        // Skip api key creation for no-auth custom endpoints
        let api_key = if is_no_auth {
            None
        } else {
            let credential_type = if credential.is_empty() && node_id.is_some() {
                "node_managed"
            } else {
                "api_key"
            };

            Some(
                user_api_key_service::create_api_key(
                    db,
                    encryption_keys,
                    user_id,
                    user_api_key_service::CreateApiKeyParams {
                        label,
                        credential_type,
                        credential,
                        access_token: None,
                        refresh_token: None,
                        token_scopes: None,
                        expires_at: None,
                        provider_config_id: None,
                        status: "active",
                        source: Some("user_created"),
                        source_id: None,
                    },
                )
                .await?,
            )
        };

        let unique_slug = resolve_unique_slug(db, user_id, &slug).await?;
        let custom_identity = identity.unwrap_or_else(user_service_service::IdentityConfig::none);
        let service = user_service_service::create_user_service(
            db,
            user_id,
            &unique_slug,
            &endpoint.id,
            api_key.as_ref().map(|k| k.id.as_str()),
            am,
            akn,
            None,
            node_id,
            0,
            "http",
            None,
            None,
            &custom_identity,
        )
        .await?;

        // Auto-sync NodeServiceBinding (no-op for custom HTTP without catalog_service_id).
        node_service::sync_node_binding_for_user_service(db, user_id, None, node_id, None).await?;

        Ok(CreateKeyResult {
            endpoint,
            api_key,
            service,
            ssh_host: None,
            ssh_port: None,
            ssh_ca_public_key: None,
            ssh_allowed_principals: None,
            ssh_certificate_ttl_minutes: None,
        })
    }
}

async fn cleanup_auto_provision_endpoint(db: &mongodb::Database, user_id: &str, endpoint_id: &str) {
    if let Err(error) = db
        .collection::<mongodb::bson::Document>(crate::models::user_endpoint::COLLECTION_NAME)
        .delete_one(doc! { "_id": endpoint_id, "user_id": user_id })
        .await
    {
        tracing::warn!(
            endpoint_id = %endpoint_id,
            user_id = %user_id,
            error = %error,
            "Failed to clean up auto-provisioned endpoint"
        );
    }
}

/// Auto-provision UserEndpoint + UserService for truly no-auth catalog services.
/// Called lazily on list_keys. Idempotent: skips services already provisioned
/// (including deactivated ones, to respect user opt-out).
///
/// "Truly no-auth" means: `auth_method == "none"` on the DownstreamService AND
/// no `ServiceProviderRequirement` exists (which would indicate master-credential
/// injection). Internal services with SPRs use master credentials and are NOT no-auth.
pub async fn auto_provision_no_auth_services(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<()> {
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
    };

    // Find all active services with auth_method "none" and no user credential requirement
    let candidates: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! {
            "is_active": true,
            "auth_method": "none",
            "requires_user_credential": false,
            "service_category": { "$in": ["connection", "internal"] },
            "service_type": "http",
        })
        .await?
        .try_collect()
        .await?;

    if candidates.is_empty() {
        return Ok(());
    }

    // Load SPRs to exclude services that use master credentials
    let candidate_ids: Vec<&str> = candidates.iter().map(|s| s.id.as_str()).collect();
    let sprs: Vec<ServiceProviderRequirement> = db
        .collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
        .find(doc! { "service_id": { "$in": &candidate_ids } })
        .await?
        .try_collect()
        .await?;
    let has_spr: std::collections::HashSet<&str> =
        sprs.iter().map(|r| r.service_id.as_str()).collect();

    // Filter to truly no-auth services (no SPR = no credential injection needed)
    let no_auth_services: Vec<&DownstreamService> = candidates
        .iter()
        .filter(|s| !has_spr.contains(s.id.as_str()))
        .collect();

    if no_auth_services.is_empty() {
        return Ok(());
    }

    // Find which catalog_service_ids this user already has (active or inactive)
    let catalog_ids: Vec<&str> = no_auth_services.iter().map(|s| s.id.as_str()).collect();
    let existing: Vec<crate::models::user_service::UserService> = db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .find(doc! {
            "user_id": user_id,
            "catalog_service_id": { "$in": &catalog_ids },
        })
        .await?
        .try_collect()
        .await?;

    let existing_catalog_ids: std::collections::HashSet<&str> = existing
        .iter()
        .filter_map(|s| s.catalog_service_id.as_deref())
        .collect();

    for svc in &no_auth_services {
        if existing_catalog_ids.contains(svc.id.as_str()) {
            continue;
        }

        let unique_slug = match resolve_unique_slug(db, user_id, &svc.slug).await {
            Ok(slug) => slug,
            Err(e) => {
                tracing::warn!(
                    service = %svc.slug,
                    error = %e,
                    "Failed to resolve slug for auto-provision"
                );
                continue;
            }
        };

        let endpoint = match user_endpoint_service::create_endpoint(
            db,
            user_id,
            &svc.name,
            &svc.base_url,
            Some(&svc.id),
        )
        .await
        {
            Ok(ep) => ep,
            Err(e) => {
                tracing::warn!(
                    service = %svc.slug,
                    error = %e,
                    "Failed to create endpoint for auto-provision"
                );
                continue;
            }
        };

        let source_id = auto_provision_source_id(user_id, &svc.id);
        let catalog_identity = identity_config_from_downstream_service(svc);
        match user_service_service::create_user_service(
            db,
            user_id,
            &unique_slug,
            &endpoint.id,
            None, // no api key for no-auth services
            "none",
            "",
            Some(&svc.id),
            None,
            0,
            "http",
            Some(AUTO_PROVISION_SOURCE),
            Some(&source_id),
            &catalog_identity,
        )
        .await
        {
            Ok(_) => {}
            Err(AppError::Conflict(_)) => {
                cleanup_auto_provision_endpoint(db, user_id, &endpoint.id).await;
            }
            Err(AppError::DatabaseError(error)) if is_duplicate_key_error(&error) => {
                cleanup_auto_provision_endpoint(db, user_id, &endpoint.id).await;
            }
            Err(e) => {
                cleanup_auto_provision_endpoint(db, user_id, &endpoint.id).await;
                tracing::warn!(
                    service = %svc.slug,
                    error = %e,
                    "Failed to create user service for auto-provision"
                );
            }
        }
    }

    Ok(())
}

/// GET /api/v1/keys -- list all keys as combined views.
pub async fn list_keys(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<KeyView>> {
    let services = user_service_service::list_user_services(db, user_id).await?;
    if services.is_empty() {
        return Ok(vec![]);
    }

    // Batch-load endpoints
    let endpoint_ids: Vec<&str> = services.iter().map(|s| s.endpoint_id.as_str()).collect();
    let endpoints: Vec<UserEndpoint> = db
        .collection::<UserEndpoint>(crate::models::user_endpoint::COLLECTION_NAME)
        .find(doc! { "_id": { "$in": &endpoint_ids } })
        .await?
        .try_collect()
        .await?;
    let ep_map: HashMap<&str, &UserEndpoint> =
        endpoints.iter().map(|e| (e.id.as_str(), e)).collect();

    // Batch-load api keys (only for services that have one)
    let api_key_ids: Vec<&str> = services
        .iter()
        .filter_map(|s| s.api_key_id.as_deref())
        .collect();
    let api_keys: Vec<UserApiKey> = if api_key_ids.is_empty() {
        vec![]
    } else {
        db.collection::<UserApiKey>(crate::models::user_api_key::COLLECTION_NAME)
            .find(doc! { "_id": { "$in": &api_key_ids } })
            .await?
            .try_collect()
            .await?
    };
    let ak_map: HashMap<&str, &UserApiKey> = api_keys.iter().map(|k| (k.id.as_str(), k)).collect();

    // Batch-load catalog services (for names + SSH config)
    let catalog_ids: Vec<&str> = services
        .iter()
        .filter_map(|s| s.catalog_service_id.as_deref())
        .collect();
    let catalog_services: Vec<DownstreamService> = if catalog_ids.is_empty() {
        vec![]
    } else {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": &catalog_ids } })
            .await?
            .try_collect()
            .await?
    };
    let cat_map: HashMap<&str, &DownstreamService> = catalog_services
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();

    let views = services
        .iter()
        .filter_map(|svc| {
            let ep = ep_map.get(svc.endpoint_id.as_str())?;
            let ak = svc
                .api_key_id
                .as_deref()
                .and_then(|id| ak_map.get(id).copied());
            Some(build_key_view(svc, ep, ak, &cat_map))
        })
        .collect();

    Ok(views)
}

/// GET /api/v1/keys/:id -- get single combined view.
pub async fn get_key(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<KeyView> {
    let svc = user_service_service::get_user_service(db, user_id, service_id).await?;
    let ep = user_endpoint_service::get_endpoint(db, user_id, &svc.endpoint_id).await?;
    let ak = if let Some(ref ak_id) = svc.api_key_id {
        Some(user_api_key_service::get_api_key(db, user_id, ak_id).await?)
    } else {
        None
    };

    // Load catalog service if applicable (for name + SSH config)
    let catalog_ds = if let Some(ref csid) = svc.catalog_service_id {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": csid })
            .await?
    } else {
        None
    };

    let cat_map: HashMap<&str, &DownstreamService> = catalog_ds
        .as_ref()
        .and_then(|ds| svc.catalog_service_id.as_deref().map(|id| (id, ds)))
        .into_iter()
        .collect();

    Ok(build_key_view(&svc, &ep, ak.as_ref(), &cat_map))
}

pub async fn reconcile_provider_key_for_service_routing(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let service = user_service_service::get_user_service(db, user_id, service_id).await?;

    // No-auth auto-connected services have no api key to reconcile
    let Some(ref ak_id) = service.api_key_id else {
        return Ok(());
    };
    let api_key = user_api_key_service::get_api_key(db, user_id, ak_id).await?;

    if service.node_id.is_some() {
        user_api_key_service::activate_node_managed_api_key(db, user_id, &api_key.id).await?;
        return Ok(());
    }

    if user_api_key_service::has_server_credential(&api_key) || service.auth_method == "none" {
        return Ok(());
    }

    let provider = if let Some(provider_config_id) = api_key.provider_config_id.as_deref() {
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find_one(doc! { "_id": provider_config_id })
            .await?
    } else {
        None
    };
    let Some(direct_credential_type) =
        direct_credential_type_for_service(&api_key, &service, provider.as_ref())
    else {
        return Ok(());
    };

    if let Some(provider_config_id) = api_key.provider_config_id.as_deref()
        && find_existing_provider_token(db, user_id, provider_config_id)
            .await?
            .is_some()
    {
        user_api_key_service::mark_provider_connection_pending(
            db,
            user_id,
            &api_key.id,
            direct_credential_type,
        )
        .await?;
        user_api_key_service::sync_provider_token_to_api_keys(db, user_id, provider_config_id)
            .await?;
        return Ok(());
    }

    user_api_key_service::mark_provider_connection_pending(
        db,
        user_id,
        &api_key.id,
        direct_credential_type,
    )
    .await
}

/// DELETE /api/v1/keys/:id -- revoke key.
pub async fn revoke_key(db: &mongodb::Database, user_id: &str, service_id: &str) -> AppResult<()> {
    let svc = user_service_service::get_user_service(db, user_id, service_id).await?;
    user_service_service::deactivate_user_service(db, user_id, service_id).await?;
    if let Some(ref ak_id) = svc.api_key_id {
        user_api_key_service::revoke_api_key(db, user_id, ak_id).await?;
    }

    // Deactivate the node binding if this service was node-routed.
    node_service::sync_node_binding_for_user_service(
        db,
        user_id,
        svc.catalog_service_id.as_deref(),
        None, // cleared
        svc.node_id.as_deref(),
    )
    .await?;

    Ok(())
}

fn build_key_view(
    svc: &UserService,
    ep: &UserEndpoint,
    ak: Option<&UserApiKey>,
    cat_map: &HashMap<&str, &DownstreamService>,
) -> KeyView {
    let catalog_ds = svc
        .catalog_service_id
        .as_deref()
        .and_then(|id| cat_map.get(id).copied());

    // SSH fields from catalog service
    let (
        ssh_host,
        ssh_port,
        ssh_ca_public_key,
        ssh_allowed_principals,
        ssh_certificate_ttl_minutes,
    ) = if svc.service_type == "ssh" {
        if let Some(ds) = catalog_ds {
            if let Some(ref ssh) = ds.ssh_config {
                (
                    Some(ssh.host.clone()),
                    Some(ssh.port),
                    ssh.ca_public_key.clone(),
                    Some(ssh.allowed_principals.clone()),
                    Some(ssh.certificate_ttl_minutes),
                )
            } else {
                (None, None, None, None, None)
            }
        } else {
            (None, None, None, None, None)
        }
    } else {
        (None, None, None, None, None)
    };

    let auto_connected = svc.source.as_deref() == Some(AUTO_PROVISION_SOURCE);

    KeyView {
        id: svc.id.clone(),
        label: ak.map_or_else(|| ep.label.clone(), |k| k.label.clone()),
        slug: svc.slug.clone(),
        endpoint_url: ep.url.clone(),
        endpoint_id: ep.id.clone(),
        api_key_id: ak.map(|k| k.id.clone()),
        credential_type: ak
            .map(|k| k.credential_type.clone())
            .unwrap_or_else(|| "none".to_string()),
        auth_method: svc.auth_method.clone(),
        auth_key_name: svc.auth_key_name.clone(),
        status: ak
            .map(|k| k.status.clone())
            .unwrap_or_else(|| "active".to_string()),
        catalog_service_id: svc.catalog_service_id.clone(),
        catalog_service_slug: catalog_ds.map(|ds| ds.slug.clone()),
        catalog_service_name: catalog_ds.map(|ds| ds.name.clone()),
        node_id: svc.node_id.clone(),
        node_priority: svc.node_priority,
        service_type: svc.service_type.clone(),
        is_active: svc.is_active,
        identity_propagation_mode: svc.identity_propagation_mode.clone(),
        identity_include_user_id: svc.identity_include_user_id,
        identity_include_email: svc.identity_include_email,
        identity_include_name: svc.identity_include_name,
        identity_jwt_audience: svc.identity_jwt_audience.clone(),
        forward_access_token: svc.forward_access_token,
        inject_delegation_token: svc.inject_delegation_token,
        delegation_token_scope: svc.delegation_token_scope.clone(),
        auto_connected,
        expires_at: ak.and_then(|k| k.expires_at.map(|dt| dt.to_rfc3339())),
        last_used_at: ak.and_then(|k| k.last_used_at.map(|dt| dt.to_rfc3339())),
        error_message: ak.and_then(|k| k.error_message.clone()),
        created_at: svc.created_at.to_rfc3339(),
        ssh_host,
        ssh_port,
        ssh_ca_public_key,
        ssh_allowed_principals,
        ssh_certificate_ttl_minutes,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;

    use super::{
        AUTO_PROVISION_SOURCE, auto_provision_source_id, build_key_view,
        direct_credential_type_for_service, direct_credential_type_from_auth_method,
        identity_config_from_downstream_service, validate_token_exchange_catalog_credential,
    };
    use crate::errors::AppError;
    use crate::models::downstream_service::{
        CredentialFieldSpec, DownstreamService, TokenExchangeConfig,
    };
    use crate::models::user_api_key::UserApiKey;
    use crate::models::user_endpoint::UserEndpoint;
    use crate::models::user_service::UserService;

    fn sample_api_key(credential_type: &str) -> UserApiKey {
        UserApiKey {
            id: "key-1".to_string(),
            user_id: "user-1".to_string(),
            label: "Test".to_string(),
            credential_type: credential_type.to_string(),
            credential_encrypted: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: None,
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_service(auth_method: &str) -> UserService {
        UserService {
            id: "svc-1".to_string(),
            user_id: "user-1".to_string(),
            slug: "test".to_string(),
            endpoint_id: "ep-1".to_string(),
            api_key_id: Some("key-1".to_string()),
            auth_method: auth_method.to_string(),
            auth_key_name: "Authorization".to_string(),
            catalog_service_id: None,
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
            is_active: true,
            source: None,
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_catalog_service() -> DownstreamService {
        DownstreamService {
            id: "cat-1".to_string(),
            name: "Catalog".to_string(),
            slug: "catalog".to_string(),
            description: None,
            base_url: "https://example.com".to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: "header".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential_encrypted: vec![],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "connection".to_string(),
            requires_user_credential: true,
            is_active: true,
            created_by: "system".to_string(),
            identity_propagation_mode: "both".to_string(),
            identity_include_user_id: true,
            identity_include_email: true,
            identity_include_name: false,
            identity_jwt_audience: Some("https://aud.example.com".to_string()),
            forward_access_token: false,
            inject_delegation_token: true,
            delegation_token_scope: "proxy:* llm:status".to_string(),
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
            token_exchange_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn infers_direct_credential_type_from_auth_method() {
        assert_eq!(
            direct_credential_type_from_auth_method("bearer"),
            Some("bearer")
        );
        assert_eq!(
            direct_credential_type_from_auth_method("basic"),
            Some("basic")
        );
        assert_eq!(
            direct_credential_type_from_auth_method("header"),
            Some("api_key")
        );
        assert_eq!(
            direct_credential_type_from_auth_method("query"),
            Some("api_key")
        );
        assert_eq!(direct_credential_type_from_auth_method("none"), None);
    }

    #[test]
    fn restores_custom_node_managed_service_to_auth_specific_type() {
        let key = sample_api_key("node_managed");
        let service = sample_service("bearer");
        assert_eq!(
            direct_credential_type_for_service(&key, &service, None),
            Some("bearer")
        );
    }

    #[test]
    fn build_key_view_uses_endpoint_label_for_no_auth_services() {
        let mut service = sample_service("none");
        service.api_key_id = None;
        service.source = Some(AUTO_PROVISION_SOURCE.to_string());

        let endpoint = UserEndpoint {
            id: "ep-1".to_string(),
            user_id: "user-1".to_string(),
            label: "Public service".to_string(),
            url: "https://example.com".to_string(),
            catalog_service_id: Some("cat-1".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let view = build_key_view(&service, &endpoint, None, &HashMap::new());
        assert_eq!(view.label, "Public service");
        assert_eq!(view.credential_type, "none");
        assert_eq!(view.status, "active");
        assert!(view.auto_connected);
    }

    #[test]
    fn auto_provision_source_id_is_user_scoped() {
        assert_ne!(
            auto_provision_source_id("user-1", "svc-1"),
            auto_provision_source_id("user-2", "svc-1")
        );
    }

    #[test]
    fn identity_config_from_downstream_service_preserves_catalog_settings() {
        let service = sample_catalog_service();

        let identity = identity_config_from_downstream_service(&service);
        assert_eq!(identity.identity_propagation_mode, "both");
        assert!(identity.identity_include_user_id);
        assert!(identity.identity_include_email);
        assert_eq!(
            identity.identity_jwt_audience.as_deref(),
            Some("https://aud.example.com")
        );
        assert!(!identity.forward_access_token);
        assert!(identity.inject_delegation_token);
        assert_eq!(identity.delegation_token_scope, "proxy:* llm:status");
    }

    #[test]
    fn identity_config_defaults_include_flags_when_mode_active_but_all_flags_off() {
        let mut service = sample_catalog_service();
        service.identity_propagation_mode = "headers".to_string();
        service.identity_include_user_id = false;
        service.identity_include_email = false;
        service.identity_include_name = false;

        let identity = identity_config_from_downstream_service(&service);
        assert_eq!(identity.identity_propagation_mode, "headers");
        assert!(
            identity.identity_include_user_id,
            "should default to true when mode is active but all flags off"
        );
        assert!(
            identity.identity_include_email,
            "should default to true when mode is active but all flags off"
        );
        assert!(
            identity.identity_include_name,
            "should default to true when mode is active but all flags off"
        );
    }

    #[test]
    fn identity_config_respects_explicit_flags_when_some_are_set() {
        let mut service = sample_catalog_service();
        service.identity_propagation_mode = "headers".to_string();
        service.identity_include_user_id = false;
        service.identity_include_email = true;
        service.identity_include_name = false;

        let identity = identity_config_from_downstream_service(&service);
        assert!(
            !identity.identity_include_user_id,
            "explicit false should be preserved"
        );
        assert!(identity.identity_include_email);
        assert!(
            !identity.identity_include_name,
            "explicit false should be preserved"
        );
    }

    #[test]
    fn identity_config_no_default_for_mode_none() {
        let mut service = sample_catalog_service();
        service.identity_propagation_mode = "none".to_string();
        service.identity_include_user_id = false;
        service.identity_include_email = false;
        service.identity_include_name = false;

        let identity = identity_config_from_downstream_service(&service);
        assert!(!identity.identity_include_user_id);
        assert!(!identity.identity_include_email);
        assert!(!identity.identity_include_name);
    }

    // ─── validate_token_exchange_catalog_credential ──────────────────

    fn lark_bot_catalog_service() -> DownstreamService {
        let mut svc = sample_catalog_service();
        svc.slug = "api-lark-bot".to_string();
        svc.auth_method = "token_exchange".to_string();
        svc.auth_key_name = String::new();
        svc.token_exchange_config = Some(TokenExchangeConfig {
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
        });
        svc
    }

    #[test]
    fn validate_token_exchange_credential_accepts_well_formed_json() {
        let svc = lark_bot_catalog_service();
        validate_token_exchange_catalog_credential(
            &svc,
            r#"{"app_id":"cli_xxx","app_secret":"yyy"}"#,
        )
        .expect("well-formed credential must be accepted");
    }

    #[test]
    fn validate_token_exchange_credential_rejects_raw_string_from_old_cli() {
        // Regression: an older CLI running `nyxid service add api-lark-bot`
        // against a new-server catalog would POST /api/v1/keys with
        // `credential: "<just the app_secret>"`. Under the new
        // token_exchange auth method that's unusable -- the proxy's
        // parse_credential needs {app_id, app_secret}. We fail at
        // registration time with a message that tells the caller how
        // to recover instead of silently creating a broken binding.
        let svc = lark_bot_catalog_service();
        let err = validate_token_exchange_catalog_credential(&svc, "just-the-app-secret")
            .expect_err("raw-string credential must be rejected");
        let msg = err.to_string();
        assert!(
            matches!(err, AppError::BadRequest(_)),
            "expected BadRequest, got: {msg}"
        );
        // The error must tell the user which fields are required and
        // point them at the update path.
        assert!(msg.contains("api-lark-bot"), "msg: {msg}");
        assert!(msg.contains("app_id"), "msg: {msg}");
        assert!(msg.contains("app_secret"), "msg: {msg}");
        assert!(msg.contains("nyxid update"), "msg: {msg}");
    }

    #[test]
    fn validate_token_exchange_credential_rejects_missing_field() {
        let svc = lark_bot_catalog_service();
        let err = validate_token_exchange_catalog_credential(&svc, r#"{"app_id":"cli_xxx"}"#)
            .expect_err("credential missing app_secret must be rejected");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_token_exchange_credential_is_noop_for_body_auth_service() {
        // Existing users on the old body-auth path still POST just the
        // app_secret string. The helper must short-circuit for any
        // auth_method other than token_exchange so it doesn't reject
        // perfectly valid pre-#220 bindings.
        let mut svc = lark_bot_catalog_service();
        svc.auth_method = "body".to_string();
        svc.auth_key_name = "app_secret".to_string();
        validate_token_exchange_catalog_credential(&svc, "raw-app-secret")
            .expect("body auth credentials must pass through without validation");
    }

    #[test]
    fn validate_token_exchange_credential_errors_cleanly_if_catalog_missing_config() {
        // Data integrity guard: if the catalog row somehow has
        // auth_method=token_exchange but no token_exchange_config, we
        // surface a clear Internal error pointing at the catalog slug
        // so admins know where to look.
        let mut svc = lark_bot_catalog_service();
        svc.token_exchange_config = None;
        let err =
            validate_token_exchange_catalog_credential(&svc, r#"{"app_id":"x","app_secret":"y"}"#)
                .expect_err("missing config must fail with an Internal error");
        assert!(matches!(err, AppError::Internal(_)));
        assert!(err.to_string().contains("api-lark-bot"));
    }
}
