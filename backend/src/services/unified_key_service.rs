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

/// Three-state representation for `openapi_spec_url` on create. The wire
/// format collapses "field absent" and "null" into the same value, so we
/// cannot round-trip the caller's intent through a bare `Option<String>`:
/// empty string must mean "opt out of catalog inheritance" while absent
/// must mean "inherit". Callers in the handler layer translate the HTTP
/// body into this enum.
#[derive(Clone, Debug)]
pub enum OpenApiSpecUrlInput<'a> {
    /// Field was omitted from the request. For catalog-backed keys, inherit
    /// the catalog entry's spec URL. For custom endpoints, store None.
    Inherit,
    /// Caller sent an empty string. Store None regardless of catalog default.
    Clear,
    /// Caller sent a non-empty URL.
    Set(&'a str),
}

/// Resolve the final OpenAPI spec URL to store, given the caller's intent,
/// whether the key is SSH-backed, and the catalog default (if any). Pulled
/// out of `create_key` so the three-state behaviour is unit-testable.
fn resolve_openapi_spec_url(
    input: &OpenApiSpecUrlInput<'_>,
    is_ssh: bool,
    catalog_default: Option<&str>,
) -> Option<String> {
    if is_ssh {
        return None;
    }
    match input {
        OpenApiSpecUrlInput::Inherit => catalog_default.map(str::to_string),
        OpenApiSpecUrlInput::Clear => None,
        OpenApiSpecUrlInput::Set(url) => Some(url.trim().to_string()),
    }
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
    pub custom_user_agent: Option<String>,
    pub auto_connected: bool,
    /// Developer app (OAuth client) ID that triggered this auto-provision.
    pub source_app_id: Option<String>,
    /// Human-readable name of the developer app (resolved from OauthClient).
    pub source_app_name: Option<String>,
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
    /// User-supplied (or catalog-inherited) OpenAPI spec URL for endpoint
    /// discovery, lifted from `UserEndpoint.openapi_spec_url`.
    pub openapi_spec_url: Option<String>,
    /// Provenance: personal credentials, or inherited from an org membership.
    /// Defaults to `Personal` for backward compatibility with single-key paths
    /// (`get_key`, post-create) which always operate on personally-owned keys.
    pub credential_source: user_service_service::CredentialSource,
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
///
/// `user_id` is the *effective owner* of the new key (the actor for personal,
/// the org's user_id for `target_org_id`-scoped creation). `actor_user_id`
/// is the human/API key actually making the request -- used for the node
/// permission check inside `user_service_service::create_user_service` so
/// that an admin can route an org service through their personal node.
#[allow(clippy::too_many_arguments)]
pub async fn create_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    actor_user_id: &str,
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
    openapi_spec_url: OpenApiSpecUrlInput<'_>,
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

        // Create all three records. Resolution is centralised in
        // `resolve_openapi_spec_url` so the SSH / inherit / clear / set
        // matrix is covered by unit tests.
        let resolved_spec_url =
            resolve_openapi_spec_url(&openapi_spec_url, is_ssh, svc.openapi_spec_url.as_deref());
        let endpoint = user_endpoint_service::create_endpoint(
            db,
            user_id,
            &svc.name,
            &ep_url,
            Some(&svc.id),
            resolved_spec_url.as_deref(),
        )
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
            actor_user_id,
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
            None,
            &catalog_identity,
        )
        .await?;

        // Auto-sync NodeServiceBinding for the catalog service. The binding
        // is owned by the org (when target_org_id is set), but the node is
        // owned by the actor making the request -- pass both so the node
        // permission check uses the actor while the binding row is created
        // under the org.
        node_service::sync_node_binding_for_user_service(
            db,
            user_id,
            actor_user_id,
            Some(&svc.id),
            node_id,
            None,
        )
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
            custom_user_agent: None,
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: now,
            updated_at: now,
        };

        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&ds)
            .await?;

        // Custom SSH services don't have OpenAPI specs; ignore any URL sent.
        let endpoint = user_endpoint_service::create_endpoint(
            db,
            user_id,
            label,
            &base_url,
            Some(&ds_id),
            None,
        )
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
            actor_user_id,
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
            None,
            &user_service_service::IdentityConfig::none(),
        )
        .await?;

        // Auto-sync NodeServiceBinding for the custom SSH service. See
        // comment in the catalog branch above for why both user_id and
        // actor_user_id are passed.
        node_service::sync_node_binding_for_user_service(
            db,
            user_id,
            actor_user_id,
            Some(&ds_id),
            node_id,
            None,
        )
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

        // Custom HTTP path: no catalog default exists, so the resolver
        // collapses Inherit/Clear to None and only a Set is stored.
        let custom_spec_url = resolve_openapi_spec_url(&openapi_spec_url, false, None);
        let endpoint = user_endpoint_service::create_endpoint(
            db,
            user_id,
            label,
            ep_url,
            None,
            custom_spec_url.as_deref(),
        )
        .await?;

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
            actor_user_id,
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
            None,
            &custom_identity,
        )
        .await?;

        // Auto-sync NodeServiceBinding (no-op for custom HTTP without catalog_service_id).
        node_service::sync_node_binding_for_user_service(
            db,
            user_id,
            actor_user_id,
            None,
            node_id,
            None,
        )
        .await?;

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
/// Called lazily on list_keys. Idempotent: skips services already provisioned.
///
/// "Truly no-auth" means: `auth_method == "none"` on the DownstreamService AND
/// no `ServiceProviderRequirement` exists (which would indicate master-credential
/// injection). Internal services with SPRs use master credentials and are NOT no-auth.
///
/// Visibility rules:
/// - Public services: auto-provision for all users.
/// - Private services with `developer_app_ids`: only auto-provision if the user
///   has an active consent for at least one of those OAuth clients (developer apps).
///   The matched app ID is stored as `source_app_id` on the UserService.
/// - Private services without `developer_app_ids`: never auto-provision.
///
/// Reconciliation runs first: any previously auto-provisioned services whose
/// catalog entry is no longer eligible are deleted (not deactivated). Deletion
/// allows re-provisioning if the user becomes eligible again later. Users
/// cannot deactivate auto-connected services themselves (the handler rejects
/// PUT/DELETE on auto-connected keys), so existing rows for a given
/// `(user_id, catalog_service_id)` pair are always either active (valid) or
/// absent (deleted by reconciliation / never created).
pub async fn auto_provision_no_auth_services(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<()> {
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
    };

    // Reconcile first: delete any previously auto-provisioned services whose
    // catalog entry is no longer eligible (deleted, deactivated, changed auth
    // method, gained an SPR, went private without consent, etc). This is
    // fully independent of the provisioning pipeline below.
    reconcile_stale_auto_provisions(db, user_id).await;

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

    // Collect all developer_app_ids from private services to batch-check consents
    let all_app_ids: Vec<&str> = no_auth_services
        .iter()
        .filter(|s| s.visibility == "private")
        .filter_map(|s| s.developer_app_ids.as_ref())
        .flat_map(|ids| ids.iter().map(|id| id.as_str()))
        .collect();

    // Load user's consents for the referenced developer apps (if any).
    // Only non-expired consents for active OAuth clients count.
    let consented_app_ids: std::collections::HashSet<String> = if all_app_ids.is_empty() {
        std::collections::HashSet::new()
    } else {
        load_valid_app_consents(db, user_id, &all_app_ids).await?
    };

    // Build the eligible list: (service, matched_app_id)
    // - Public: always eligible, no app context
    // - Private with developer_app_ids: eligible only if user consented to >= 1 app
    // - Private without developer_app_ids: never eligible
    let eligible: Vec<(&DownstreamService, Option<&str>)> = no_auth_services
        .iter()
        .filter_map(|svc| {
            if svc.visibility != "private" {
                // Public (or legacy without visibility) -- always eligible
                Some((*svc, None))
            } else if let Some(ref app_ids) = svc.developer_app_ids {
                // Private with developer_app_ids -- find first consented app
                let matched = app_ids
                    .iter()
                    .find(|id| consented_app_ids.contains(id.as_str()));
                matched.map(|app_id| (*svc, Some(app_id.as_str())))
            } else {
                // Private without developer_app_ids -- skip
                None
            }
        })
        .collect();

    if eligible.is_empty() {
        return Ok(());
    }

    // Find which catalog_service_ids this user already has (active or inactive)
    let catalog_ids: Vec<&str> = eligible.iter().map(|(s, _)| s.id.as_str()).collect();
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

    for (svc, source_app_id) in &eligible {
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
            svc.openapi_spec_url.as_deref(),
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
        // Auto-provision is always personal (node_id = None), so the actor
        // and the effective owner are the same.
        match user_service_service::create_user_service(
            db,
            user_id,
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
            *source_app_id,
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

/// Load valid (non-expired, active-client) app consents for a user.
/// Shared between the provisioning pipeline and reconciliation.
pub async fn load_valid_app_consents(
    db: &mongodb::Database,
    user_id: &str,
    app_ids: &[&str],
) -> AppResult<std::collections::HashSet<String>> {
    use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
    use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};

    if app_ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    // Filter to only active OAuth clients
    let active_clients: Vec<OauthClient> = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find(doc! {
            "_id": { "$in": app_ids },
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;
    let active_app_ids: Vec<&str> = active_clients.iter().map(|c| c.id.as_str()).collect();

    if active_app_ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    // Filter consents: non-expired (null or future) for active apps
    let now_bson = bson::DateTime::from_chrono(chrono::Utc::now());
    let consents: Vec<Consent> = db
        .collection::<Consent>(CONSENTS)
        .find(doc! {
            "user_id": user_id,
            "client_id": { "$in": &active_app_ids },
            "$or": [
                { "expires_at": { "$exists": false } },
                { "expires_at": bson::Bson::Null },
                { "expires_at": { "$gt": now_bson } },
            ],
        })
        .await?
        .try_collect()
        .await?;
    Ok(consents.into_iter().map(|c| c.client_id).collect())
}

/// Delete stale auto-provisioned UserServices that the user is no longer
/// eligible for. Fully self-contained: loads the user's active
/// auto-provisioned services, their catalog entries, SPRs, and consents,
/// then applies the complete "truly no-auth" eligibility predicate.
///
/// A service is stale if its catalog entry:
/// - No longer exists or is inactive
/// - No longer satisfies the "truly no-auth" predicate (auth_method changed,
///   gained an SPR, changed to SSH, changed category, now requires user
///   credential, etc.)
/// - Is now private without `developer_app_ids`
/// - Is now private with `developer_app_ids` but the user has no valid consent
async fn reconcile_stale_auto_provisions(db: &mongodb::Database, user_id: &str) {
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
    };

    // Load all active auto-provisioned services for this user
    let auto_services: Vec<crate::models::user_service::UserService> = match db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .find(doc! {
            "user_id": user_id,
            "source": AUTO_PROVISION_SOURCE,
            "is_active": true,
        })
        .await
    {
        Ok(cursor) => match cursor.try_collect().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "reconcile: failed to load auto-provisioned services");
                return;
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "reconcile: failed to query auto-provisioned services");
            return;
        }
    };

    if auto_services.is_empty() {
        return;
    }

    // Batch-load catalog entries
    let catalog_ids: Vec<&str> = auto_services
        .iter()
        .filter_map(|s| s.catalog_service_id.as_deref())
        .collect();
    let catalog_map: std::collections::HashMap<String, DownstreamService> =
        if catalog_ids.is_empty() {
            std::collections::HashMap::new()
        } else {
            match db
                .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .find(doc! { "_id": { "$in": &catalog_ids } })
                .await
            {
                Ok(cursor) => match cursor.try_collect::<Vec<_>>().await {
                    Ok(svcs) => svcs.into_iter().map(|s| (s.id.clone(), s)).collect(),
                    Err(e) => {
                        tracing::warn!(error = %e, "reconcile: failed to load catalog services");
                        return;
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "reconcile: failed to query catalog services");
                    return;
                }
            }
        };

    // Load SPRs for the catalog entries to check the "truly no-auth" predicate
    let spr_set: std::collections::HashSet<String> = if catalog_ids.is_empty() {
        std::collections::HashSet::new()
    } else {
        match db
            .collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
            .find(doc! { "service_id": { "$in": &catalog_ids } })
            .await
        {
            Ok(cursor) => match cursor
                .try_collect::<Vec<ServiceProviderRequirement>>()
                .await
            {
                Ok(sprs) => sprs.into_iter().map(|r| r.service_id).collect(),
                Err(e) => {
                    tracing::warn!(error = %e, "reconcile: failed to load SPRs");
                    return;
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "reconcile: failed to query SPRs");
                return;
            }
        }
    };

    // Collect all developer_app_ids from private catalog entries to load consents
    let all_app_ids: Vec<&str> = catalog_map
        .values()
        .filter(|ds| ds.visibility == "private")
        .filter_map(|ds| ds.developer_app_ids.as_ref())
        .flat_map(|ids| ids.iter().map(|id| id.as_str()))
        .collect();

    let consented_app_ids = match load_valid_app_consents(db, user_id, &all_app_ids).await {
        Ok(set) => set,
        Err(e) => {
            tracing::warn!(error = %e, "reconcile: failed to load consents");
            return;
        }
    };

    // Determine which auto-provisioned services are now stale.
    // A service is valid only if its catalog entry still satisfies the full
    // "truly no-auth" predicate AND the visibility/consent rules.
    let stale: Vec<&crate::models::user_service::UserService> = auto_services
        .iter()
        .filter(|us| {
            let catalog = us
                .catalog_service_id
                .as_deref()
                .and_then(|id| catalog_map.get(id));

            match catalog {
                None => true, // catalog entry deleted
                Some(ds) => {
                    // Re-check the full "truly no-auth" predicate
                    let is_truly_no_auth = ds.is_active
                        && ds.auth_method == "none"
                        && !ds.requires_user_credential
                        && (ds.service_category == "connection"
                            || ds.service_category == "internal")
                        && ds.service_type == "http"
                        && !spr_set.contains(&ds.id);

                    if !is_truly_no_auth {
                        return true; // catalog changed -- stale
                    }

                    // Check visibility/consent rules
                    if ds.visibility == "private" {
                        match ds.developer_app_ids.as_ref() {
                            Some(app_ids) if !app_ids.is_empty() => {
                                // Stale if no consent matches
                                !app_ids
                                    .iter()
                                    .any(|id| consented_app_ids.contains(id.as_str()))
                            }
                            _ => true, // private without app_ids -- stale
                        }
                    } else {
                        false // public + truly-no-auth -- still valid
                    }
                }
            }
        })
        .collect();

    if stale.is_empty() {
        return;
    }

    let stale_service_ids: Vec<&str> = stale.iter().map(|us| us.id.as_str()).collect();
    let stale_endpoint_ids: Vec<&str> = stale.iter().map(|us| us.endpoint_id.as_str()).collect();

    // Delete stale UserService rows (not deactivate). Deletion lets the
    // provisioning path re-create the service when the user becomes
    // eligible again (e.g., re-consents to a developer app). Deactivation
    // would leave an inactive row that the provisioning path treats as
    // "already provisioned" and skips.
    //
    // Note: users cannot deactivate auto-connected services themselves --
    // DELETE /keys/:id and PUT /keys/:id both reject auto-connected rows.
    // So all inactive auto-provisioned rows are from reconciliation, and
    // deleting here is always correct.
    match db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .delete_many(doc! { "_id": { "$in": &stale_service_ids } })
        .await
    {
        Ok(result) => {
            if result.deleted_count > 0 {
                tracing::info!(
                    user_id = %user_id,
                    count = result.deleted_count,
                    "Deleted stale auto-provisioned services"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                user_id = %user_id,
                count = stale_service_ids.len(),
                error = %e,
                "Failed to delete stale auto-provisioned services"
            );
            return; // don't clean up endpoints if services weren't deleted
        }
    }

    // Clean up orphaned auto-provisioned endpoints. Only delete endpoints
    // that are not referenced by any remaining UserService.
    if !stale_endpoint_ids.is_empty() {
        // Find which of these endpoints are still referenced by other services
        let still_referenced: std::collections::HashSet<String> = match db
            .collection::<crate::models::user_service::UserService>(
                crate::models::user_service::COLLECTION_NAME,
            )
            .find(doc! {
                "user_id": user_id,
                "endpoint_id": { "$in": &stale_endpoint_ids },
            })
            .await
        {
            Ok(cursor) => match cursor
                .try_collect::<Vec<crate::models::user_service::UserService>>()
                .await
            {
                Ok(svcs) => svcs.into_iter().map(|s| s.endpoint_id).collect(),
                Err(_) => return,
            },
            Err(_) => return,
        };

        let orphaned: Vec<&str> = stale_endpoint_ids
            .iter()
            .filter(|id| !still_referenced.contains(**id))
            .copied()
            .collect();

        if !orphaned.is_empty() {
            let _ = db
                .collection::<mongodb::bson::Document>(
                    crate::models::user_endpoint::COLLECTION_NAME,
                )
                .delete_many(doc! {
                    "_id": { "$in": &orphaned },
                    "user_id": user_id,
                })
                .await;
        }
    }
}

/// GET /api/v1/keys -- list all keys (personal + org-inherited) as combined views.
///
/// Each returned `KeyView` carries a `credential_source` tag matching the
/// `/user-services` endpoint. Org-inherited services appear after the user's
/// personal ones, grouped per org. Viewer-role org services are returned with
/// `credential_source.allowed = false` so the frontend can render them as
/// read-only.
pub async fn list_keys(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<KeyView>> {
    let tagged = user_service_service::list_user_services_with_sources(db, user_id).await?;
    if tagged.is_empty() {
        return Ok(vec![]);
    }

    // Batch-load endpoints. Endpoints are looked up by `_id` only, so personal
    // and org-owned endpoints can be fetched in the same query.
    let endpoint_ids: Vec<&str> = tagged
        .iter()
        .map(|t| t.service.endpoint_id.as_str())
        .collect();
    let endpoints: Vec<UserEndpoint> = db
        .collection::<UserEndpoint>(crate::models::user_endpoint::COLLECTION_NAME)
        .find(doc! { "_id": { "$in": &endpoint_ids } })
        .await?
        .try_collect()
        .await?;
    let ep_map: HashMap<&str, &UserEndpoint> =
        endpoints.iter().map(|e| (e.id.as_str(), e)).collect();

    // Batch-load api keys (only for services that have one).
    let api_key_ids: Vec<&str> = tagged
        .iter()
        .filter_map(|t| t.service.api_key_id.as_deref())
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

    // Batch-load catalog services (for names + SSH config).
    let catalog_ids: Vec<&str> = tagged
        .iter()
        .filter_map(|t| t.service.catalog_service_id.as_deref())
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

    // Batch-load developer app names (for auto-provisioned services from apps).
    let source_app_ids: Vec<&str> = tagged
        .iter()
        .filter_map(|t| t.service.source_app_id.as_deref())
        .collect();
    let app_name_map: HashMap<String, String> = if source_app_ids.is_empty() {
        HashMap::new()
    } else {
        use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
        let apps: Vec<OauthClient> = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find(doc! { "_id": { "$in": &source_app_ids } })
            .await?
            .try_collect()
            .await?;
        apps.into_iter().map(|a| (a.id, a.client_name)).collect()
    };

    let views = tagged
        .into_iter()
        .filter_map(|t| {
            let ep = ep_map.get(t.service.endpoint_id.as_str())?;
            let ak = t
                .service
                .api_key_id
                .as_deref()
                .and_then(|id| ak_map.get(id).copied());
            Some(build_key_view(
                &t.service,
                ep,
                ak,
                &cat_map,
                &app_name_map,
                t.source,
            ))
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

    // Load developer app name if this service was app-provisioned
    let app_name_map: HashMap<String, String> = if let Some(ref app_id) = svc.source_app_id {
        use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
        if let Some(app) = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": app_id })
            .await?
        {
            [(app.id, app.client_name)].into_iter().collect()
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    // get_key returns the personal view by default. The handler is responsible
    // for tagging the response with the actual credential_source when the
    // request was authenticated as an org member -- see resolve_key_read_owner
    // in handlers/keys.rs.
    Ok(build_key_view(
        &svc,
        &ep,
        ak.as_ref(),
        &cat_map,
        &app_name_map,
        user_service_service::CredentialSource::Personal,
    ))
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
///
/// `actor_user_id` is forwarded to `deactivate_user_service` for symmetry
/// with the create/update path; it is not actually consulted because
/// deactivation does not change the node_id.
pub async fn revoke_key(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let svc = user_service_service::get_user_service(db, user_id, service_id).await?;
    user_service_service::deactivate_user_service(db, user_id, actor_user_id, service_id).await?;
    if let Some(ref ak_id) = svc.api_key_id {
        user_api_key_service::revoke_api_key(db, user_id, ak_id).await?;
    }

    // Deactivate the node binding if this service was node-routed. The
    // delete path clears the node, so the actor only matters for the
    // (skipped) node validation -- pass it for symmetry.
    node_service::sync_node_binding_for_user_service(
        db,
        user_id,
        actor_user_id,
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
    app_name_map: &HashMap<String, String>,
    credential_source: user_service_service::CredentialSource,
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
    let source_app_name = svc
        .source_app_id
        .as_ref()
        .and_then(|id| app_name_map.get(id).cloned());

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
        custom_user_agent: svc.custom_user_agent.clone(),
        auto_connected,
        source_app_id: svc.source_app_id.clone(),
        source_app_name,
        expires_at: ak.and_then(|k| k.expires_at.map(|dt| dt.to_rfc3339())),
        last_used_at: ak.and_then(|k| k.last_used_at.map(|dt| dt.to_rfc3339())),
        error_message: ak.and_then(|k| k.error_message.clone()),
        created_at: svc.created_at.to_rfc3339(),
        ssh_host,
        ssh_port,
        ssh_ca_public_key,
        ssh_allowed_principals,
        ssh_certificate_ttl_minutes,
        openapi_spec_url: ep.openapi_spec_url.clone(),
        credential_source,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;

    use super::{
        AUTO_PROVISION_SOURCE, OpenApiSpecUrlInput, auto_provision_source_id, build_key_view,
        direct_credential_type_for_service, direct_credential_type_from_auth_method,
        identity_config_from_downstream_service, resolve_openapi_spec_url,
        validate_token_exchange_catalog_credential,
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
            custom_user_agent: None,
            is_active: true,
            source_app_id: None,
            source: None,
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_endpoint() -> UserEndpoint {
        UserEndpoint {
            id: "ep-1".to_string(),
            user_id: "user-1".to_string(),
            label: "Test Endpoint".to_string(),
            url: "https://example.com".to_string(),
            catalog_service_id: None,
            openapi_spec_url: None,
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
            custom_user_agent: None,
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn resolve_spec_inherit_uses_catalog_default_for_http_services() {
        let out = resolve_openapi_spec_url(
            &OpenApiSpecUrlInput::Inherit,
            false,
            Some("https://catalog.example/openapi.json"),
        );
        assert_eq!(out.as_deref(), Some("https://catalog.example/openapi.json"));
    }

    #[test]
    fn resolve_spec_clear_opts_out_even_when_catalog_has_default() {
        // Regression: P3 finding -- empty-string opt-out used to fall back
        // to the catalog default because `""` was normalised to `None` before
        // the inheritance lookup.
        let out = resolve_openapi_spec_url(
            &OpenApiSpecUrlInput::Clear,
            false,
            Some("https://catalog.example/openapi.json"),
        );
        assert_eq!(out, None);
    }

    #[test]
    fn resolve_spec_set_overrides_catalog_default() {
        let out = resolve_openapi_spec_url(
            &OpenApiSpecUrlInput::Set("https://user.example/spec.json"),
            false,
            Some("https://catalog.example/openapi.json"),
        );
        assert_eq!(out.as_deref(), Some("https://user.example/spec.json"));
    }

    #[test]
    fn resolve_spec_set_trims_whitespace() {
        let out = resolve_openapi_spec_url(
            &OpenApiSpecUrlInput::Set("  https://user.example/spec.json  "),
            false,
            None,
        );
        assert_eq!(out.as_deref(), Some("https://user.example/spec.json"));
    }

    #[test]
    fn resolve_spec_ssh_catalog_always_none() {
        // Regression: P3 finding -- SSH catalog services could persist a
        // user-supplied or catalog-inherited spec URL even though they have
        // no OpenAPI surface and the frontend hides the field.
        assert_eq!(
            resolve_openapi_spec_url(
                &OpenApiSpecUrlInput::Set("https://user.example/spec.json"),
                true,
                Some("https://catalog.example/openapi.json"),
            ),
            None
        );
        assert_eq!(
            resolve_openapi_spec_url(
                &OpenApiSpecUrlInput::Inherit,
                true,
                Some("https://catalog.example/openapi.json"),
            ),
            None
        );
    }

    #[test]
    fn resolve_spec_custom_http_no_catalog_default() {
        // Custom HTTP path: Inherit and Clear both collapse to None because
        // there is no catalog entry to inherit from.
        assert_eq!(
            resolve_openapi_spec_url(&OpenApiSpecUrlInput::Inherit, false, None),
            None
        );
        assert_eq!(
            resolve_openapi_spec_url(&OpenApiSpecUrlInput::Clear, false, None),
            None
        );
        assert_eq!(
            resolve_openapi_spec_url(
                &OpenApiSpecUrlInput::Set("https://user.example/spec.json"),
                false,
                None,
            )
            .as_deref(),
            Some("https://user.example/spec.json"),
        );
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
            openapi_spec_url: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let view = build_key_view(
            &service,
            &endpoint,
            None,
            &HashMap::new(),
            &HashMap::new(),
            crate::services::user_service_service::CredentialSource::Personal,
        );
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

    // ─── Developer app auto-provision visibility tests ─────────────────

    #[test]
    fn build_key_view_sets_source_app_name_from_map() {
        let mut service = sample_service("none");
        service.source = Some(AUTO_PROVISION_SOURCE.to_string());
        service.source_app_id = Some("app-123".to_string());
        let endpoint = sample_endpoint();

        let app_map: HashMap<String, String> = [("app-123".to_string(), "My Dev App".to_string())]
            .into_iter()
            .collect();

        let view = build_key_view(
            &service,
            &endpoint,
            None,
            &HashMap::new(),
            &app_map,
            crate::services::user_service_service::CredentialSource::Personal,
        );

        assert!(view.auto_connected);
        assert_eq!(view.source_app_id.as_deref(), Some("app-123"));
        assert_eq!(view.source_app_name.as_deref(), Some("My Dev App"));
    }

    #[test]
    fn build_key_view_no_source_app_for_public_auto_provision() {
        let mut service = sample_service("none");
        service.source = Some(AUTO_PROVISION_SOURCE.to_string());
        // No source_app_id set -- public auto-provision
        let endpoint = sample_endpoint();

        let view = build_key_view(
            &service,
            &endpoint,
            None,
            &HashMap::new(),
            &HashMap::new(),
            crate::services::user_service_service::CredentialSource::Personal,
        );

        assert!(view.auto_connected);
        assert!(view.source_app_id.is_none());
        assert!(view.source_app_name.is_none());
    }

    #[test]
    fn build_key_view_source_app_id_without_matching_name() {
        // Edge case: source_app_id exists but app was deleted (not in map)
        let mut service = sample_service("none");
        service.source = Some(AUTO_PROVISION_SOURCE.to_string());
        service.source_app_id = Some("deleted-app".to_string());
        let endpoint = sample_endpoint();

        let view = build_key_view(
            &service,
            &endpoint,
            None,
            &HashMap::new(),
            &HashMap::new(), // empty map -- app not found
            crate::services::user_service_service::CredentialSource::Personal,
        );

        assert!(view.auto_connected);
        assert_eq!(view.source_app_id.as_deref(), Some("deleted-app"));
        assert!(
            view.source_app_name.is_none(),
            "deleted app should not resolve a name"
        );
    }

    #[test]
    fn build_key_view_not_auto_connected_without_source() {
        let service = sample_service("bearer");
        let endpoint = sample_endpoint();

        let view = build_key_view(
            &service,
            &endpoint,
            None,
            &HashMap::new(),
            &HashMap::new(),
            crate::services::user_service_service::CredentialSource::Personal,
        );

        assert!(!view.auto_connected);
        assert!(view.source_app_id.is_none());
        assert!(view.source_app_name.is_none());
    }

    /// Visibility eligibility matrix (documents the logic, not an integration test)
    #[test]
    fn visibility_eligibility_rules() {
        use crate::models::downstream_service::test_helpers::dummy_service;

        let consented: std::collections::HashSet<String> =
            ["app-a".to_string()].into_iter().collect();

        // Public service: always eligible
        let mut public_svc = dummy_service();
        public_svc.visibility = "public".to_string();
        assert_ne!(public_svc.visibility, "private");

        // Private + developer_app_ids with matching consent: eligible
        let mut private_with_consent = dummy_service();
        private_with_consent.visibility = "private".to_string();
        private_with_consent.developer_app_ids = Some(vec!["app-a".to_string()]);
        let matched = private_with_consent
            .developer_app_ids
            .as_ref()
            .unwrap()
            .iter()
            .find(|id| consented.contains(id.as_str()));
        assert!(
            matched.is_some(),
            "private with matching consent should be eligible"
        );

        // Private + developer_app_ids without matching consent: ineligible
        let mut private_no_consent = dummy_service();
        private_no_consent.visibility = "private".to_string();
        private_no_consent.developer_app_ids = Some(vec!["app-b".to_string()]);
        let matched = private_no_consent
            .developer_app_ids
            .as_ref()
            .unwrap()
            .iter()
            .find(|id| consented.contains(id.as_str()));
        assert!(
            matched.is_none(),
            "private without matching consent should be ineligible"
        );

        // Private without developer_app_ids: ineligible
        let mut private_no_apps = dummy_service();
        private_no_apps.visibility = "private".to_string();
        private_no_apps.developer_app_ids = None;
        assert!(
            private_no_apps.developer_app_ids.is_none(),
            "private without developer_app_ids should never auto-provision"
        );

        // Private with empty developer_app_ids: ineligible
        let mut private_empty_apps = dummy_service();
        private_empty_apps.visibility = "private".to_string();
        private_empty_apps.developer_app_ids = Some(vec![]);
        let has_match = private_empty_apps
            .developer_app_ids
            .as_ref()
            .unwrap()
            .iter()
            .any(|id| consented.contains(id.as_str()));
        assert!(
            !has_match,
            "private with empty developer_app_ids should never auto-provision"
        );
    }
}
